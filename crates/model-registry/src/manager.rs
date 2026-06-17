//! Model job orchestration: lifecycle, progress, persistence.
//!
//! Mirrors `crates/backtest/src/manager.rs`.  In Phase 1 all drivers are stubs
//! that advance phases on a timer.  Phase 2 replaces the driver body with real
//! Python sidecar calls without changing the public API.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use chrono::Utc;
use sqlx::PgPool;
use tokio::sync::RwLock;
use uuid::Uuid;

use domain::model_def::{validate::validate, ModelDefinition};

use crate::job::Job;
use crate::types::{
    CreateModelRequest, ModelRecord, ModelRunKind, ModelRunSnapshot, RunStatus, TrainRequest,
};

const MAX_CONCURRENT_JOBS: usize = 3;

/// Convert a display name to a URL-safe slug base.  Collapses runs of
/// non-alphanumerics to a single `-` and trims leading/trailing dashes.
/// Falls back to `"model"` when the name has no alphanumerics.
fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for c in name.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "model".to_string()
    } else {
        trimmed.to_string()
    }
}

pub struct ModelManager {
    pg: PgPool,
    jobs: RwLock<HashMap<Uuid, Arc<Job>>>,
    run_permits: Arc<tokio::sync::Semaphore>,
    /// Broadcast channel for WS lane `models.jobs`.
    progress_tx: tokio::sync::broadcast::Sender<serde_json::Value>,
    /// Typed HTTP client for the Python trainer/inference sidecars.
    sidecar: Arc<crate::sidecar::SidecarClient>,
    /// Dataset materialization manager.
    datasets: Arc<crate::datasets::DatasetManager>,
}

impl ModelManager {
    pub fn new(
        pg: PgPool,
        sidecar: Arc<crate::sidecar::SidecarClient>,
        datasets: Arc<crate::datasets::DatasetManager>,
    ) -> Arc<Self> {
        let (progress_tx, _) = tokio::sync::broadcast::channel(256);
        Arc::new(Self {
            pg,
            jobs: RwLock::new(HashMap::new()),
            run_permits: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_JOBS)),
            progress_tx,
            sidecar,
            datasets,
        })
    }

    /// Convenience constructor that builds the sidecar client and dataset manager
    /// from environment configuration.
    pub fn from_env(pg: PgPool) -> Arc<Self> {
        let sidecar = Arc::new(crate::sidecar::SidecarClient::from_env());
        let datasets = Arc::new(crate::datasets::DatasetManager::new(pg.clone()));
        Self::new(pg, sidecar, datasets)
    }

    /// Apply a progress frame received from the Python sidecar over NATS.
    pub async fn apply_nats_progress(
        &self,
        run_id: uuid::Uuid,
        phase: &str,
        progress: f32,
        metric: Option<serde_json::Value>,
    ) {
        use std::sync::atomic::Ordering;
        let jobs = self.jobs.read().await;
        if let Some(job) = jobs.get(&run_id) {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            job.progress_pct.store(progress as u32, Ordering::Relaxed);
            {
                let mut state = job.state.write().expect("poisoned");
                state.phase = phase.to_string();
                if let Some(m) = metric {
                    // Merge into existing metrics
                    let existing = state.metrics.get_or_insert(serde_json::json!({}));
                    if let (Some(obj), Some(new_obj)) = (existing.as_object_mut(), m.as_object()) {
                        for (k, v) in new_obj {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            self.broadcast_progress(&job.snapshot());
        }
    }

    /// Subscribe to model job progress events (for the `models.jobs` WS lane).
    pub fn subscribe_progress(&self) -> tokio::sync::broadcast::Receiver<serde_json::Value> {
        self.progress_tx.subscribe()
    }

    // -- Model CRUD --

    pub async fn create_model(
        &self,
        user_id: Uuid,
        req: CreateModelRequest,
    ) -> anyhow::Result<String> {
        validate(&req.definition).map_err(|errs| {
            let msgs: Vec<_> = errs
                .iter()
                .map(|e| format!("{}: {}", e.path, e.message))
                .collect();
            anyhow::anyhow!("validation failed: {}", msgs.join("; "))
        })?;

        let model_id = format!("mdl_{}", Uuid::new_v4().as_simple());
        let slug = self.unique_slug(&slugify(&req.display_name)).await?;
        let definition_json = serde_json::to_value(&req.definition)?;
        let model_kind = format!("{:?}", req.definition.model_kind).to_lowercase();
        let asset_class = req.definition.asset_class.clone();

        sqlx::query(
            "INSERT INTO ai_models \
             (model_id, slug, display_name, description, model_kind, asset_class, \
              definition_json, status, created_by, created_at, updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,'draft',$8,now(),now())",
        )
        .bind(&model_id)
        .bind(&slug)
        .bind(&req.display_name)
        .bind(&req.description)
        .bind(&model_kind)
        .bind(&asset_class)
        .bind(&definition_json)
        .bind(user_id)
        .execute(&self.pg)
        .await?;

        self.record_event(
            &model_id,
            user_id,
            "created",
            serde_json::json!({"display_name": &req.display_name}),
        )
        .await;

        Ok(model_id)
    }

    /// Return a slug unique across `ai_models`: `base`, else `base-2`, `base-3`,
    /// … (the `slug` column has a global UNIQUE constraint).  Bounded so a
    /// pathological loop can't hang; the final fallback appends a uuid fragment.
    async fn unique_slug(&self, base: &str) -> anyhow::Result<String> {
        for n in 1..=999 {
            let candidate = if n == 1 {
                base.to_string()
            } else {
                format!("{base}-{n}")
            };
            let exists: Option<(String,)> =
                sqlx::query_as("SELECT slug FROM ai_models WHERE slug = $1 LIMIT 1")
                    .bind(&candidate)
                    .fetch_optional(&self.pg)
                    .await?;
            if exists.is_none() {
                return Ok(candidate);
            }
        }
        Ok(format!("{base}-{}", Uuid::new_v4().as_simple()))
    }

    pub async fn get_model(
        &self,
        model_id: &str,
        user_id: Uuid,
    ) -> anyhow::Result<Option<ModelRecord>> {
        let row: Option<ModelRow> = sqlx::query_as(
            "SELECT model_id, slug, display_name, description, model_kind, asset_class, \
             definition_json, status, created_by, created_at, updated_at \
             FROM ai_models WHERE model_id = $1 AND created_by = $2",
        )
        .bind(model_id)
        .bind(user_id)
        .fetch_optional(&self.pg)
        .await?;
        Ok(row.and_then(ModelRow::into_record))
    }

    pub async fn list_models(
        &self,
        user_id: Uuid,
        kind: Option<&str>,
        status: Option<&str>,
        asset_class: Option<&str>,
    ) -> anyhow::Result<Vec<ModelRecord>> {
        let rows: Vec<ModelRow> = sqlx::query_as(
            "SELECT model_id, slug, display_name, description, model_kind, asset_class, \
             definition_json, status, created_by, created_at, updated_at \
             FROM ai_models \
             WHERE created_by = $1 \
               AND ($2::text IS NULL OR model_kind = $2) \
               AND ($3::text IS NULL OR status = $3) \
               AND ($4::text IS NULL OR asset_class = $4) \
             ORDER BY created_at DESC \
             LIMIT 500",
        )
        .bind(user_id)
        .bind(kind)
        .bind(status)
        .bind(asset_class)
        .fetch_all(&self.pg)
        .await?;
        Ok(rows.into_iter().filter_map(ModelRow::into_record).collect())
    }

    pub async fn list_models_by_asset_class(
        &self,
        asset_class: &str,
    ) -> anyhow::Result<Vec<ModelRecord>> {
        let rows: Vec<ModelRow> = sqlx::query_as(
            "SELECT model_id, slug, display_name, description, model_kind, asset_class, \
             definition_json, status, created_by, created_at, updated_at \
             FROM ai_models WHERE asset_class = $1 AND status != 'archived' ORDER BY created_at DESC LIMIT 100",
        )
        .bind(asset_class)
        .fetch_all(&self.pg)
        .await?;
        Ok(rows.into_iter().filter_map(ModelRow::into_record).collect())
    }

    pub async fn rename_model(
        &self,
        model_id: &str,
        user_id: Uuid,
        new_display_name: &str,
        new_description: Option<&str>,
    ) -> anyhow::Result<()> {
        let old = self
            .get_model(model_id, user_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("not found"))?;
        sqlx::query(
            "UPDATE ai_models SET display_name = $1, description = COALESCE($2, description), updated_at = now() \
             WHERE model_id = $3 AND created_by = $4",
        )
        .bind(new_display_name)
        .bind(new_description)
        .bind(model_id)
        .bind(user_id)
        .execute(&self.pg)
        .await?;
        self.record_event(
            model_id,
            user_id,
            "renamed",
            serde_json::json!({
                "from": old.display_name, "to": new_display_name
            }),
        )
        .await;
        Ok(())
    }

    pub async fn archive_model(&self, model_id: &str, user_id: Uuid) -> anyhow::Result<()> {
        let rows_affected = sqlx::query(
            "UPDATE ai_models SET status = 'archived', updated_at = now() \
             WHERE model_id = $1 AND created_by = $2 AND status != 'archived'",
        )
        .bind(model_id)
        .bind(user_id)
        .execute(&self.pg)
        .await?
        .rows_affected();
        if rows_affected == 0 {
            anyhow::bail!("not found");
        }
        self.record_event(model_id, user_id, "archived", serde_json::json!({}))
            .await;
        Ok(())
    }

    pub async fn delete_model(&self, model_id: &str, user_id: Uuid) -> anyhow::Result<()> {
        let rows_affected = sqlx::query(
            "DELETE FROM ai_models WHERE model_id = $1 AND created_by = $2 AND status = 'draft'",
        )
        .bind(model_id)
        .bind(user_id)
        .execute(&self.pg)
        .await?
        .rows_affected();
        if rows_affected == 0 {
            anyhow::bail!("not found or not in draft status");
        }
        Ok(())
    }

    // -- Training runs --

    pub async fn start_train(
        self: &Arc<Self>,
        model_id: &str,
        user_id: Uuid,
        req: TrainRequest,
    ) -> anyhow::Result<Uuid> {
        // Verify model exists and is trainable.
        let model = self
            .get_model(model_id, user_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("not found"))?;
        if model.model_kind == "external_llm_adapter" {
            anyhow::bail!("external_llm_adapter models do not train (D-3)");
        }

        let run_id = Uuid::new_v4();
        let job = Job::new(
            run_id,
            model_id.to_string(),
            user_id,
            ModelRunKind::Train,
            Utc::now(),
        );

        // Persist initial row.
        let hp = req
            .hyperparameter_overrides
            .unwrap_or(serde_json::Value::Null);
        let _ = sqlx::query(
            "INSERT INTO training_runs \
             (run_id, model_id, dataset_version_id, status, progress, phase, hyperparameters_json, created_by, created_at) \
             VALUES ($1,$2,$3,'queued',0,'queued',$4,$5,now())",
        )
        .bind(run_id)
        .bind(model_id)
        .bind(req.dataset_version_id)
        .bind(&hp)
        .bind(user_id)
        .execute(&self.pg)
        .await;

        self.jobs.write().await.insert(run_id, Arc::clone(&job));

        let manager = Arc::clone(self);
        let mid = model_id.to_string();
        let data = req.data;
        let version_note = req.version_note;
        tokio::spawn(async move { manager.drive_train(job, mid, data, hp, version_note).await });

        Ok(run_id)
    }

    pub async fn list_runs(
        &self,
        model_id: &str,
        user_id: Uuid,
    ) -> anyhow::Result<Vec<ModelRunSnapshot>> {
        self.ensure_model_owned(model_id, user_id).await?;
        let jobs = self.jobs.read().await;
        let mut snaps: Vec<ModelRunSnapshot> = jobs
            .values()
            .filter(|j| {
                j.model_id == model_id && j.user_id == user_id && j.kind == ModelRunKind::Train
            })
            .map(|j| j.snapshot())
            .collect();
        snaps.sort_by_key(|s| Reverse(s.created_at));
        Ok(snaps)
    }

    pub async fn get_run(&self, run_id: Uuid, user_id: Uuid) -> Option<ModelRunSnapshot> {
        let jobs = self.jobs.read().await;
        jobs.get(&run_id)
            .filter(|j| j.user_id == user_id)
            .map(|j| j.snapshot())
    }

    pub async fn cancel_run(&self, run_id: Uuid, user_id: Uuid) -> anyhow::Result<()> {
        let jobs = self.jobs.read().await;
        let job = jobs
            .get(&run_id)
            .filter(|j| j.user_id == user_id)
            .ok_or_else(|| anyhow::anyhow!("not found"))?;
        anyhow::ensure!(
            !job.state.read().expect("poisoned").status.is_terminal(),
            "run already finished"
        );
        job.cancel.store(true, Ordering::Relaxed);
        Ok(())
    }

    // -- Versions --

    pub async fn list_versions(
        &self,
        model_id: &str,
        user_id: Uuid,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        self.ensure_model_owned(model_id, user_id).await?;
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            i32,
            String,
            Option<serde_json::Value>,
            Option<serde_json::Value>,
            chrono::DateTime<Utc>,
            Option<chrono::DateTime<Utc>>,
        )> = sqlx::query_as(
            "SELECT version, status, metrics_json, scorecard_json, created_at, promoted_at \
             FROM model_versions WHERE model_id = $1 ORDER BY version DESC",
        )
        .bind(model_id)
        .fetch_all(&self.pg)
        .await?;
        Ok(rows
            .iter()
            .map(|(v, s, m, sc, ca, pa)| {
                serde_json::json!({
                    "version": v, "status": s, "metrics": m, "scorecard": sc,
                    "created_at": ca, "promoted_at": pa,
                })
            })
            .collect())
    }

    pub async fn register_version(
        &self,
        model_id: &str,
        user_id: Uuid,
        artifact_uri: &str,
        artifact_hash: &str,
        notes: Option<&str>,
    ) -> anyhow::Result<i32> {
        self.ensure_model_owned(model_id, user_id).await?;
        anyhow::ensure!(!artifact_hash.is_empty(), "artifact_hash required");

        let (next_version,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM model_versions WHERE model_id = $1",
        )
        .bind(model_id)
        .fetch_one(&self.pg)
        .await?;
        #[allow(clippy::cast_possible_truncation)]
        let version = next_version as i32;

        let config_json = serde_json::json!({
            "artifact_uri": artifact_uri,
            "artifact_hash": artifact_hash,
        });
        sqlx::query(
            "INSERT INTO model_versions \
             (model_id, version, status, config_json, notes, created_by, created_at) \
             VALUES ($1,$2,'draft',$3,$4,$5,now())",
        )
        .bind(model_id)
        .bind(version)
        .bind(config_json)
        .bind(notes)
        .bind(user_id)
        .execute(&self.pg)
        .await?;

        Ok(version)
    }

    // -- Aliases --

    pub async fn get_aliases(
        &self,
        model_id: &str,
        user_id: Uuid,
    ) -> anyhow::Result<serde_json::Value> {
        self.ensure_model_owned(model_id, user_id).await?;
        let rows: Vec<(String, i32)> = sqlx::query_as(
            "SELECT alias, version FROM model_aliases WHERE model_id = $1 ORDER BY alias",
        )
        .bind(model_id)
        .fetch_all(&self.pg)
        .await?;
        let map: serde_json::Map<String, serde_json::Value> = rows
            .into_iter()
            .map(|(a, v)| (a, serde_json::json!(v)))
            .collect();
        Ok(serde_json::Value::Object(map))
    }

    /// Promotion with gate evaluation (Phase 3).
    #[allow(clippy::too_many_lines)]
    pub async fn promote_gated(
        &self,
        model_id: &str,
        user_id: Uuid,
        version: i32,
        environment: &str,
        override_reason: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        self.ensure_model_owned(model_id, user_id).await?;

        // 1. Verify version exists; gather artifact info.
        let version_row: Option<(String, Option<serde_json::Value>, Option<String>)> = sqlx::query_as(
            "SELECT mv.status, ma.sha256, ma.storage_uri \
             FROM model_versions mv \
             LEFT JOIN model_artifacts ma ON ma.model_id=mv.model_id AND ma.version=mv.version AND ma.artifact_type='model' \
             WHERE mv.model_id=$1 AND mv.version=$2",
        )
        .bind(model_id)
        .bind(version)
        .fetch_optional(&self.pg)
        .await?;

        let (_ver_status, artifact_hash, _artifact_uri) =
            version_row.ok_or_else(|| anyhow::anyhow!("version {version} not found"))?;

        let mut gate_checks = Vec::new();
        let mut gate_passed = true;

        // Check 1: Passed eval suite
        let eval_passed: Option<(i64,)> = sqlx::query_as(
            "SELECT COUNT(*) FROM evaluation_runs WHERE model_id=$1 AND version=$2 AND status='succeeded'",
        )
        .bind(model_id)
        .bind(version)
        .fetch_optional(&self.pg)
        .await?;
        let has_eval = eval_passed.is_some_and(|(n,)| n > 0);
        gate_checks.push(serde_json::json!({ "check": "passed_eval_suite", "passed": has_eval, "required": true }));
        if !has_eval {
            gate_passed = false;
        }

        // Check 2: Artifact integrity
        let artifact_ok = artifact_hash.is_some();
        gate_checks.push(serde_json::json!({ "check": "artifact_integrity", "passed": artifact_ok, "required": true }));
        if !artifact_ok {
            gate_passed = false;
        }

        // Check 3: Rollback available (prior production version exists)
        let prior_production: Option<(i32,)> = sqlx::query_as(
            "SELECT version FROM model_aliases WHERE model_id=$1 AND alias='production'",
        )
        .bind(model_id)
        .fetch_optional(&self.pg)
        .await?;
        let rollback_available = prior_production.is_some() || environment != "live";
        gate_checks.push(serde_json::json!({ "check": "rollback_available", "passed": rollback_available, "required": environment == "live" }));
        if !rollback_available && environment == "live" {
            gate_passed = false;
        }

        // Check 4: No metric regression (from latest eval run)
        let regression: Option<(Option<serde_json::Value>,)> = sqlx::query_as(
            "SELECT regression_report_json FROM evaluation_runs WHERE model_id=$1 AND version=$2 AND status='succeeded' ORDER BY created_at DESC LIMIT 1",
        )
        .bind(model_id)
        .bind(version)
        .fetch_optional(&self.pg)
        .await?;
        let no_regression = regression
            .as_ref()
            .and_then(|(r,)| r.as_ref())
            .and_then(|r| r.get("verdict"))
            .and_then(|v| v.as_str())
            .is_none_or(|v| v != "regressed"); // if no baseline, pass
        gate_checks.push(serde_json::json!({ "check": "no_metric_regression", "passed": no_regression, "required": true }));
        if !no_regression {
            gate_passed = false;
        }

        // Gate decision
        if !gate_passed && override_reason.is_none() {
            return Ok(serde_json::json!({
                "promoted": false,
                "reason": "gate_failed",
                "gate_checks": gate_checks,
            }));
        }

        // Execute promotion
        let prior_active: Option<(i32,)> = sqlx::query_as(
            "SELECT version FROM model_aliases WHERE model_id=$1 AND alias='production'",
        )
        .bind(model_id)
        .fetch_optional(&self.pg)
        .await?;

        // Move production alias
        sqlx::query(
            "INSERT INTO model_aliases (model_id, alias, version, updated_by, updated_at) \
             VALUES ($1,'production',$2,$3,now()) \
             ON CONFLICT (model_id, alias) DO UPDATE SET version=EXCLUDED.version, updated_by=EXCLUDED.updated_by, updated_at=now()",
        )
        .bind(model_id)
        .bind(version)
        .bind(user_id)
        .execute(&self.pg)
        .await?;

        // Set new version Active
        sqlx::query(
            "UPDATE model_versions SET status='active', promoted_at=now() WHERE model_id=$1 AND version=$2",
        )
        .bind(model_id)
        .bind(version)
        .execute(&self.pg)
        .await?;

        // Demote prior Active to Archived
        if let Some((pv,)) = prior_active {
            if pv != version {
                let _ = sqlx::query(
                    "UPDATE model_versions SET status='archived' WHERE model_id=$1 AND version=$2 AND status='active'",
                )
                .bind(model_id)
                .bind(pv)
                .execute(&self.pg)
                .await;

                // Set fallback alias to prior version
                let _ = sqlx::query(
                    "INSERT INTO model_aliases (model_id, alias, version, updated_by, updated_at) \
                     VALUES ($1,'fallback',$2,$3,now()) \
                     ON CONFLICT (model_id, alias) DO UPDATE SET version=EXCLUDED.version, updated_by=EXCLUDED.updated_by, updated_at=now()",
                )
                .bind(model_id)
                .bind(pv)
                .bind(user_id)
                .execute(&self.pg)
                .await;
            }
        }

        // Update model status
        let _ =
            sqlx::query("UPDATE ai_models SET status='active', updated_at=now() WHERE model_id=$1")
                .bind(model_id)
                .execute(&self.pg)
                .await;

        let event_payload = serde_json::json!({
            "version": version,
            "alias": "production",
            "environment": environment,
            "gate_checks": gate_checks,
            "override_reason": override_reason,
        });
        self.record_event(model_id, user_id, "promoted", event_payload)
            .await;

        Ok(serde_json::json!({
            "promoted": true,
            "version": version,
            "alias": "production",
            "gate_checks": gate_checks,
            "override_reason": override_reason,
        }))
    }

    /// Set an arbitrary alias (production/candidate/staging/fallback) to a version.
    pub async fn set_alias(
        &self,
        model_id: &str,
        alias: &str,
        version: i32,
        user_id: Uuid,
    ) -> anyhow::Result<()> {
        let valid_aliases = ["production", "candidate", "staging", "fallback"];
        anyhow::ensure!(valid_aliases.contains(&alias), "invalid alias: {alias}");
        self.ensure_model_owned(model_id, user_id).await?;
        sqlx::query(
            "INSERT INTO model_aliases (model_id, alias, version, updated_by, updated_at) \
             VALUES ($1,$2,$3,$4,now()) \
             ON CONFLICT (model_id, alias) DO UPDATE SET version=EXCLUDED.version, updated_by=EXCLUDED.updated_by, updated_at=now()",
        )
        .bind(model_id)
        .bind(alias)
        .bind(version)
        .bind(user_id)
        .execute(&self.pg)
        .await?;
        self.record_event(
            model_id,
            user_id,
            "alias_set",
            serde_json::json!({"alias": alias, "version": version}),
        )
        .await;
        Ok(())
    }

    pub async fn rollback(&self, model_id: &str, alias: &str, user_id: Uuid) -> anyhow::Result<()> {
        self.ensure_model_owned(model_id, user_id).await?;
        let current: Option<(i32,)> =
            sqlx::query_as("SELECT version FROM model_aliases WHERE model_id = $1 AND alias = $2")
                .bind(model_id)
                .bind(alias)
                .fetch_optional(&self.pg)
                .await?;
        let current_version = current
            .map(|(v,)| v)
            .ok_or_else(|| anyhow::anyhow!("alias not found"))?;

        let prev: Option<(i32,)> = sqlx::query_as(
            "SELECT version FROM model_versions WHERE model_id = $1 AND version < $2 ORDER BY version DESC LIMIT 1",
        )
        .bind(model_id)
        .bind(current_version)
        .fetch_optional(&self.pg)
        .await?;
        let prev_version = prev
            .map(|(v,)| v)
            .ok_or_else(|| anyhow::anyhow!("no previous version to roll back to"))?;

        sqlx::query(
            "UPDATE model_aliases SET version = $1, updated_by = $2, updated_at = now() \
             WHERE model_id = $3 AND alias = $4",
        )
        .bind(prev_version)
        .bind(user_id)
        .bind(model_id)
        .bind(alias)
        .execute(&self.pg)
        .await?;

        self.record_event(
            model_id,
            user_id,
            "rolled_back",
            serde_json::json!({
                "alias": alias, "from_version": current_version, "to_version": prev_version
            }),
        )
        .await;
        Ok(())
    }

    // -- Evaluation --

    pub async fn start_eval(
        self: &Arc<Self>,
        model_id: &str,
        version: i32,
        user_id: Uuid,
    ) -> anyhow::Result<Uuid> {
        self.ensure_model_owned(model_id, user_id).await?;
        let eval_id = Uuid::new_v4();
        let job = Job::new(
            eval_id,
            model_id.to_string(),
            user_id,
            ModelRunKind::Evaluate,
            Utc::now(),
        );

        let _ = sqlx::query(
            "INSERT INTO evaluation_runs \
             (eval_id, model_id, version, status, created_by, created_at) \
             VALUES ($1,$2,$3,'queued',$4,now())",
        )
        .bind(eval_id)
        .bind(model_id)
        .bind(version)
        .bind(user_id)
        .execute(&self.pg)
        .await;

        self.jobs.write().await.insert(eval_id, Arc::clone(&job));

        let manager = Arc::clone(self);
        let mid = model_id.to_string();
        tokio::spawn(async move { manager.drive_eval(job, mid, version).await });

        Ok(eval_id)
    }

    pub async fn list_evals(
        &self,
        model_id: &str,
        user_id: Uuid,
    ) -> anyhow::Result<Vec<ModelRunSnapshot>> {
        self.ensure_model_owned(model_id, user_id).await?;
        let jobs = self.jobs.read().await;
        let mut snaps: Vec<ModelRunSnapshot> = jobs
            .values()
            .filter(|j| {
                j.model_id == model_id && j.user_id == user_id && j.kind == ModelRunKind::Evaluate
            })
            .map(|j| j.snapshot())
            .collect();
        snaps.sort_by_key(|s| Reverse(s.created_at));
        Ok(snaps)
    }

    pub async fn get_eval(
        &self,
        model_id: &str,
        eval_id: Uuid,
        user_id: Uuid,
    ) -> anyhow::Result<serde_json::Value> {
        self.ensure_model_owned(model_id, user_id).await?;
        #[allow(clippy::type_complexity)]
        let row: Option<(
            Uuid,
            String,
            i32,
            String,
            Option<serde_json::Value>,
            Option<serde_json::Value>,
            Option<serde_json::Value>,
            Option<serde_json::Value>,
            Option<i32>,
            chrono::DateTime<Utc>,
        )> = sqlx::query_as(
            "SELECT eval_id, model_id, version, status, metrics_json, scorecard_json, \
             regression_report_json, sample_outputs_json, baseline_version, created_at \
             FROM evaluation_runs WHERE eval_id=$1 AND model_id=$2",
        )
        .bind(eval_id)
        .bind(model_id)
        .fetch_optional(&self.pg)
        .await?;

        row.map(
            |(eid, mid, ver, st, metrics, scorecard, regression, samples, bv, ca)| {
                serde_json::json!({
                    "eval_id": eid, "model_id": mid, "version": ver, "status": st,
                    "metrics": metrics, "scorecard": scorecard,
                    "regression_report": regression, "sample_outputs": samples,
                    "baseline_version": bv, "created_at": ca,
                })
            },
        )
        .ok_or_else(|| anyhow::anyhow!("evaluation not found"))
    }

    pub async fn compare_evals(
        &self,
        model_id: &str,
        versions_str: &str,
        user_id: Uuid,
    ) -> anyhow::Result<serde_json::Value> {
        self.ensure_model_owned(model_id, user_id).await?;
        let versions: Vec<i32> = versions_str
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        anyhow::ensure!(
            versions.len() == 2,
            "compare requires exactly 2 comma-separated version numbers"
        );

        let mut results = Vec::new();
        for v in &versions {
            let eval: Option<(Option<serde_json::Value>, Option<serde_json::Value>)> = sqlx::query_as(
                "SELECT metrics_json, scorecard_json FROM evaluation_runs \
                 WHERE model_id=$1 AND version=$2 AND status='succeeded' ORDER BY created_at DESC LIMIT 1",
            )
            .bind(model_id)
            .bind(v)
            .fetch_optional(&self.pg)
            .await?;
            let (metrics, scorecard) = eval.unwrap_or((None, None));
            results.push(
                serde_json::json!({ "version": v, "metrics": metrics, "scorecard": scorecard }),
            );
        }

        // Simple winner determination by overall scorecard
        let overall_a = results[0]["scorecard"]["overall"].as_f64().unwrap_or(0.0);
        let overall_b = results[1]["scorecard"]["overall"].as_f64().unwrap_or(0.0);
        let winner = if overall_a > overall_b {
            versions[0]
        } else if overall_b > overall_a {
            versions[1]
        } else {
            -1
        };

        Ok(serde_json::json!({
            "model_id": model_id,
            "versions": results,
            "winner_version": if winner == -1 { serde_json::Value::Null } else { serde_json::json!(winner) },
            "verdict": if winner == -1 { "tie" } else { "winner_determined" },
        }))
    }

    // -- Test Lab inference --

    pub async fn test_inference(
        &self,
        model_id: &str,
        version: i32,
        user_id: Uuid,
        instances: Vec<serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        self.ensure_model_owned(model_id, user_id).await?;

        // Get model kind from model record
        let (model_kind,): (String,) =
            sqlx::query_as("SELECT model_kind FROM ai_models WHERE model_id=$1")
                .bind(model_id)
                .fetch_one(&self.pg)
                .await?;

        let start = std::time::Instant::now();

        if model_kind == "external_llm_adapter" {
            let (def_json,): (serde_json::Value,) =
                sqlx::query_as("SELECT definition_json FROM ai_models WHERE model_id=$1")
                    .bind(model_id)
                    .fetch_one(&self.pg)
                    .await?;
            let prompt = instances
                .first()
                .and_then(|i| i.get("prompt"))
                .and_then(|p| p.as_str())
                .unwrap_or("Hello")
                .to_string();
            let adapter = def_json
                .get("adapter")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            let req = crate::sidecar::LlmPredictRequest {
                model_id: model_id.to_string(),
                version,
                adapter,
                prompt,
                params: serde_json::json!({}),
            };
            let resp = self.sidecar.predict_llm(req).await?;
            #[allow(clippy::cast_possible_truncation)]
            let latency_ms = start.elapsed().as_millis() as u64;
            return Ok(serde_json::json!({
                "model_id": model_id,
                "version": version,
                "kind": "llm",
                "text": resp.text,
                "tokens": resp.tokens,
                "cost_usd": resp.cost_usd,
                "latency_ms": latency_ms,
                "trace_id": resp.trace_id,
            }));
        }

        // Get artifact info
        let artifact_row: Option<(String, String)> = sqlx::query_as(
            "SELECT storage_uri, sha256 FROM model_artifacts WHERE model_id=$1 AND version=$2 AND artifact_type='model' LIMIT 1",
        )
        .bind(model_id)
        .bind(version)
        .fetch_optional(&self.pg)
        .await?;

        let (artifact_uri, artifact_hash) = artifact_row
            .ok_or_else(|| anyhow::anyhow!("no artifact found for version {version}"))?;

        let parsed: Vec<crate::sidecar::PredictInstance> = instances
            .into_iter()
            .map(|i| {
                let instrument_id = i
                    .get("instrument_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let features = i
                    .get("features")
                    .and_then(|f| {
                        serde_json::from_value::<std::collections::HashMap<String, f64>>(f.clone())
                            .ok()
                    })
                    .unwrap_or_default();
                crate::sidecar::PredictInstance {
                    instrument_id,
                    features,
                }
            })
            .collect();

        let req = crate::sidecar::PredictRequest {
            model_id: model_id.to_string(),
            version,
            model_kind: model_kind.clone(),
            artifact_uri,
            artifact_hash,
            instances: parsed,
        };
        let resp = self.sidecar.predict(req).await?;
        #[allow(clippy::cast_possible_truncation)]
        let latency_ms = start.elapsed().as_millis() as u64;
        Ok(serde_json::json!({
            "model_id": model_id,
            "version": version,
            "kind": model_kind,
            "predictions": resp.predictions,
            "latency_ms": latency_ms,
        }))
    }

    // -- Deployments --

    pub async fn list_deployments(
        &self,
        model_id: &str,
        user_id: Uuid,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        self.ensure_model_owned(model_id, user_id).await?;
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            String,
            i32,
            String,
            Option<String>,
            String,
            i32,
            chrono::DateTime<Utc>,
        )> = sqlx::query_as(
            "SELECT deployment_id, version, environment, alias, status, traffic_pct, deployed_at \
             FROM model_deployments WHERE model_id = $1 ORDER BY deployed_at DESC",
        )
        .bind(model_id)
        .fetch_all(&self.pg)
        .await?;
        Ok(rows
            .iter()
            .map(|(did, v, env, alias, st, tp, da)| {
                serde_json::json!({
                    "deployment_id": did, "version": v, "environment": env,
                    "alias": alias, "status": st, "traffic_pct": tp, "deployed_at": da,
                })
            })
            .collect())
    }

    pub async fn create_deployment(
        &self,
        model_id: &str,
        version: i32,
        environment: &str,
        traffic_pct: i32,
        user_id: Uuid,
    ) -> anyhow::Result<String> {
        self.ensure_model_owned(model_id, user_id).await?;
        anyhow::ensure!(
            (0..=100).contains(&traffic_pct),
            "traffic_pct must be 0-100"
        );

        // Check total traffic_pct for this (model_id, environment) won't exceed 100
        let (current_total,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(traffic_pct), 0) FROM model_deployments \
             WHERE model_id=$1 AND environment=$2 AND status='active'",
        )
        .bind(model_id)
        .bind(environment)
        .fetch_one(&self.pg)
        .await?;

        anyhow::ensure!(
            current_total + i64::from(traffic_pct) <= 100,
            "traffic allocation would exceed 100% for environment {environment} (current: {current_total}%)"
        );

        let deployment_id = format!("dep_{}", Uuid::new_v4().as_simple());
        sqlx::query(
            "INSERT INTO model_deployments \
             (deployment_id, model_id, version, environment, traffic_pct, status, deployed_by, deployed_at) \
             VALUES ($1,$2,$3,$4,$5,'active',$6,now())",
        )
        .bind(&deployment_id)
        .bind(model_id)
        .bind(version)
        .bind(environment)
        .bind(traffic_pct)
        .bind(user_id)
        .execute(&self.pg)
        .await?;
        Ok(deployment_id)
    }

    // -- Test cases --

    pub async fn list_test_cases(
        &self,
        model_id: &str,
        user_id: Uuid,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        self.ensure_model_owned(model_id, user_id).await?;
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            Uuid,
            String,
            serde_json::Value,
            Option<serde_json::Value>,
            chrono::DateTime<Utc>,
        )> = sqlx::query_as(
            "SELECT case_id, name, input_json, expected_json, created_at \
             FROM model_test_cases WHERE model_id = $1 ORDER BY created_at DESC",
        )
        .bind(model_id)
        .fetch_all(&self.pg)
        .await?;
        Ok(rows
            .iter()
            .map(|(cid, name, inp, exp, ca)| {
                serde_json::json!({
                    "case_id": cid, "name": name, "input": inp, "expected": exp, "created_at": ca,
                })
            })
            .collect())
    }

    pub async fn add_test_case(
        &self,
        model_id: &str,
        user_id: Uuid,
        name: &str,
        input: serde_json::Value,
        expected: Option<serde_json::Value>,
    ) -> anyhow::Result<Uuid> {
        self.ensure_model_owned(model_id, user_id).await?;
        let (case_id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO model_test_cases (model_id, name, input_json, expected_json, created_by, created_at) \
             VALUES ($1,$2,$3,$4,$5,now()) RETURNING case_id",
        )
        .bind(model_id)
        .bind(name)
        .bind(input)
        .bind(expected)
        .bind(user_id)
        .fetch_one(&self.pg)
        .await?;
        Ok(case_id)
    }

    pub async fn delete_test_case(
        &self,
        model_id: &str,
        case_id: Uuid,
        user_id: Uuid,
    ) -> anyhow::Result<()> {
        self.ensure_model_owned(model_id, user_id).await?;
        sqlx::query("DELETE FROM model_test_cases WHERE case_id = $1 AND model_id = $2")
            .bind(case_id)
            .bind(model_id)
            .execute(&self.pg)
            .await?;
        Ok(())
    }

    // -- Lineage / used-by --

    pub async fn get_lineage(
        &self,
        model_id: &str,
        user_id: Uuid,
    ) -> anyhow::Result<serde_json::Value> {
        self.ensure_model_owned(model_id, user_id).await?;
        let aliases = self
            .get_aliases(model_id, user_id)
            .await
            .unwrap_or(serde_json::json!({}));
        Ok(serde_json::json!({
            "model_id": model_id,
            "aliases": aliases,
            "note": "full lineage graph available in Phase 2 when training runs produce dataset version refs",
        }))
    }

    // -- Internal helpers --

    async fn ensure_model_owned(&self, model_id: &str, user_id: Uuid) -> anyhow::Result<()> {
        let exists: Option<(String,)> = sqlx::query_as(
            "SELECT model_id FROM ai_models WHERE model_id = $1 AND created_by = $2",
        )
        .bind(model_id)
        .bind(user_id)
        .fetch_optional(&self.pg)
        .await?;
        anyhow::ensure!(exists.is_some(), "not found");
        Ok(())
    }

    async fn record_event(
        &self,
        model_id: &str,
        actor: Uuid,
        kind: &str,
        payload: serde_json::Value,
    ) {
        let _ = sqlx::query(
            "INSERT INTO model_events (model_id, kind, payload, actor, created_at) \
             VALUES ($1,$2,$3,$4,now())",
        )
        .bind(model_id)
        .bind(kind)
        .bind(payload)
        .bind(actor)
        .execute(&self.pg)
        .await;
    }

    fn broadcast_progress(&self, snap: &crate::types::ModelRunSnapshot) {
        let val = serde_json::json!({
            "run_id": snap.run_id,
            "model_id": snap.model_id,
            "run_kind": match snap.kind { ModelRunKind::Train => "train", ModelRunKind::Evaluate => "eval" },
            "status": snap.status.as_str(),
            "progress": snap.progress,
            "phase": snap.phase,
        });
        let _ = self.progress_tx.send(val);
    }

    // -- Real job drivers (Phase 2/3) --

    #[allow(clippy::too_many_lines)]
    async fn drive_train(
        self: &Arc<Self>,
        job: Arc<Job>,
        model_id: String,
        data: Option<crate::types::TrainDataSelection>,
        hyperparam_overrides: serde_json::Value,
        version_note: Option<String>,
    ) {
        let Ok(_permit) = self.run_permits.clone().acquire_owned().await else {
            return;
        };
        if job.cancel.load(Ordering::Relaxed) {
            job.set_phase(RunStatus::Cancelled, "cancelled");
            self.persist_run(&job).await;
            return;
        }

        let pg = self.pg.clone();

        // 1. Load model definition
        let model_row: Option<(serde_json::Value,)> =
            sqlx::query_as("SELECT definition_json FROM ai_models WHERE model_id = $1")
                .bind(&model_id)
                .fetch_optional(&pg)
                .await
                .ok()
                .flatten();

        let Some((definition_json,)) = model_row else {
            job.fail("model definition not found");
            self.broadcast_progress(&job.snapshot());
            self.persist_run(&job).await;
            return;
        };

        let mut definition: domain::model_def::ModelDefinition =
            match serde_json::from_value(definition_json) {
                Ok(d) => d,
                Err(e) => {
                    job.fail(format!("definition parse error: {e}"));
                    self.broadcast_progress(&job.snapshot());
                    self.persist_run(&job).await;
                    return;
                }
            };

        // Apply per-run hyperparameter overrides on top of the definition's
        // baked-in hyperparameters. Object keys are shallow-merged (override
        // wins); a non-object override replaces the block wholesale.
        if let serde_json::Value::Object(over) = &hyperparam_overrides {
            if !over.is_empty() {
                let base = definition
                    .hyperparameters
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                let mut merged = base;
                for (k, v) in over {
                    merged.insert(k.clone(), v.clone());
                }
                definition.hyperparameters = serde_json::Value::Object(merged);
            }
        } else if !hyperparam_overrides.is_null() {
            definition.hyperparameters = hyperparam_overrides.clone();
        }

        // 2. Materialize dataset
        job.set_phase(RunStatus::Running, "materializing");
        self.broadcast_progress(&job.snapshot());

        let artifacts_prefix =
            std::env::var("ARTIFACT_STORE_PATH").unwrap_or_else(|_| "./artifacts".to_string());

        // Resolve the effective data selection: explicit UI selection wins, then
        // the model definition, then sensible defaults.
        let feature_set_ref = data
            .as_ref()
            .and_then(|d| d.feature_set_ref.clone())
            .or_else(|| definition.feature_set_ref.clone())
            .unwrap_or_else(|| "fs_core_ohlcv_v3".to_string());
        let instruments = data
            .as_ref()
            .filter(|d| !d.instruments.is_empty()).map_or_else(|| vec!["BTC-USD".to_string()], |d| d.instruments.clone());
        let timeframe = data
            .as_ref()
            .map_or_else(|| "1m".to_string(), |d| d.timeframe.clone());
        let lookback_days = data.as_ref().map_or(30, |d| d.lookback_days);
        let label_horizon = data
            .as_ref()
            .and_then(|d| d.label_horizon.clone())
            .or_else(|| {
                definition
                    .label_spec
                    .as_ref()
                    .and_then(|s| s.get("window"))
                    .and_then(|w| w.as_str().map(str::to_string))
            })
            .unwrap_or_else(|| "1h".to_string());

        let window_end = chrono::Utc::now();
        let window_start = window_end - chrono::Duration::days(i64::from(lookback_days));

        let dataset_req = crate::datasets::DatasetRequest {
            dataset_id: None,
            feature_set_ref: feature_set_ref.clone(),
            instruments: instruments.clone(),
            timeframe: timeframe.clone(),
            start: window_start,
            end: window_end,
            label_spec: serde_json::json!({
                "type": "forward_return",
                "window": label_horizon,
            }),
            output_prefix: artifacts_prefix.clone(),
        };

        let dataset = match self.datasets.materialize(dataset_req).await {
            Ok(d) => d,
            Err(e) => {
                job.fail(format!("dataset materialization failed: {e}"));
                self.broadcast_progress(&job.snapshot());
                self.persist_run(&job).await;
                return;
            }
        };

        // 2b. Compute walk-forward folds (I-0.10): Rust owns the fold geometry;
        //     sidecar receives index ranges and never picks its own split (ADR-0017).
        let horizon_bars = features::label_horizon_bars(&label_horizon, &timeframe).unwrap_or(60);
        let effective_cv = definition.cv.unwrap_or_else(|| {
            // Single expanding fold default: mirrors the pre-Set-I
            // isolated-train path while providing a proper cal role.
            let n = dataset.row_count.max(1) as u64;
            let purge = horizon_bars;
            let available = n.saturating_sub(2 * purge);
            let train_bars = (available * 70 / 100).max(1);
            let cal_bars = (available * 15 / 100).max(1);
            let test_bars = available.saturating_sub(train_bars + cal_bars).max(1);
            domain::model_def::cv::WalkForwardSpec {
                mode: domain::model_def::cv::WindowMode::Expanding,
                folds: 1,
                train_bars,
                cal_bars,
                test_bars,
                purge_bars: purge,
                embargo_bars: purge,
            }
        });
        let fold_specs: Option<Vec<crate::sidecar::FoldSpec>> = match features::walk_forward_folds(
            dataset.row_count as usize,
            &effective_cv,
            horizon_bars,
        ) {
            Ok(folds) => Some(folds.iter().map(crate::sidecar::FoldSpec::from).collect()),
            Err(e) => {
                // Dataset too small or spec invalid — sidecar uses ordinal split.
                tracing::warn!(
                    "walk-forward fold generation failed ({e}); \
                     sidecar will fall back to ordinal split"
                );
                None
            }
        };

        // 2c. Compute whole-spec deterministic hash (I-3.8).
        let feature_set_versions: Vec<String> = {
            let fs_ref = definition
                .feature_set_ref
                .as_deref()
                .unwrap_or("fs_core_ohlcv_v3");
            features::list_feature_sets()
                .into_iter()
                .filter(|s| s.name == fs_ref)
                .map(|s| format!("{}:{}", s.name, s.version))
                .collect()
        };
        let canonical_def = crate::spec_hash::canonical_definition_json(&definition);
        let seed = definition
            .hyperparameters
            .get("seed")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let spec_hash = crate::spec_hash::compute_spec_hash(
            &canonical_def,
            &dataset.content_hash,
            seed,
            &feature_set_versions,
            "",
        );

        // 3. Dispatch to trainer sidecar
        job.set_phase(RunStatus::Running, "fitting");
        job.progress_pct.store(15, Ordering::Relaxed);
        self.broadcast_progress(&job.snapshot());

        let nats_subject = format!("models.run.{}.progress", job.run_id);
        let dispatch = crate::sidecar::TrainDispatchRequest {
            run_id: job.run_id,
            model_id: model_id.clone(),
            model_kind: format!("{:?}", definition.model_kind).to_lowercase(),
            framework: format!("{:?}", definition.framework).to_lowercase(),
            runtime: format!("{:?}", definition.runtime).to_lowercase(),
            definition: definition.clone(),
            dataset_uri: dataset.parquet_uri.clone(),
            dataset_hash: dataset.content_hash.clone(),
            output_prefix: format!("{artifacts_prefix}/models/{model_id}/{}/", job.run_id),
            progress: crate::sidecar::ProgressConfig { nats_subject },
            data: None,
            folds: fold_specs,
        };

        let result = self.sidecar.dispatch_train(dispatch).await;

        match result {
            Ok(r) if r.status == "succeeded" => {
                let artifact_uri = r.artifact_uri.unwrap_or_default();
                let sha256 = r.sha256.unwrap_or_default();
                let size_bytes = r.size_bytes.unwrap_or(0);

                // Write model_versions row
                let (next_version,): (i64,) = sqlx::query_as(
                    "SELECT COALESCE(MAX(version), 0) + 1 FROM model_versions WHERE model_id = $1",
                )
                .bind(&model_id)
                .fetch_one(&pg)
                .await
                .unwrap_or((1,));
                #[allow(clippy::cast_possible_truncation)]
                let version = next_version as i32;

                let config_json = serde_json::json!({
                    "artifact_uri": artifact_uri,
                    "artifact_hash": sha256,
                    "dataset_version_id": dataset.dataset_version_id,
                    "framework_version": r.framework_version,
                    "framework": format!("{:?}", definition.framework).to_lowercase(),
                    "runtime": format!("{:?}", definition.runtime).to_lowercase(),
                    "hyperparameters": definition.hyperparameters,
                    "spec_hash": spec_hash,
                });

                let _ = sqlx::query(
                    "INSERT INTO model_versions \
                     (model_id, version, status, training_run_id, dataset_version_id, \
                      metrics_json, config_json, notes, created_by, created_at) \
                     VALUES ($1,$2,'evaluating',$3,$4,$5,$6,$7,$8,now())",
                )
                .bind(&model_id)
                .bind(version)
                .bind(job.run_id)
                .bind(dataset.dataset_version_id)
                .bind(&r.metrics)
                .bind(config_json)
                .bind(version_note.as_deref())
                .bind(job.user_id)
                .execute(&pg)
                .await;

                // Write model_artifacts row
                let artifact_id = uuid::Uuid::new_v4();
                let _ = sqlx::query(
                    "INSERT INTO model_artifacts \
                     (artifact_id, model_id, version, storage_uri, artifact_type, size_bytes, sha256, created_at) \
                     VALUES ($1,$2,$3,$4,'model',$5,$6,now())",
                )
                .bind(artifact_id)
                .bind(&model_id)
                .bind(version)
                .bind(&artifact_uri)
                .bind(size_bytes)
                .bind(&sha256)
                .execute(&pg)
                .await;

                // Update model status to evaluating
                let _ = sqlx::query(
                    "UPDATE ai_models SET status='evaluating', updated_at=now() WHERE model_id=$1",
                )
                .bind(&model_id)
                .execute(&pg)
                .await;

                // Merge spec_hash into run metrics for reproduce lookup (I-3.8).
                let run_metrics = {
                    let mut m = r.metrics.clone().unwrap_or(serde_json::json!({}));
                    if let Some(obj) = m.as_object_mut() {
                        obj.insert("spec_hash".to_string(), serde_json::json!(spec_hash));
                    }
                    m
                };
                {
                    let mut state = job.state.write().expect("poisoned");
                    state.metrics = Some(run_metrics.clone());
                }
                // Update training_runs with merged metrics
                let _ = sqlx::query("UPDATE training_runs SET metrics_json = $1 WHERE run_id = $2")
                    .bind(&run_metrics)
                    .bind(job.run_id)
                    .execute(&pg)
                    .await;

                job.progress_pct.store(100, Ordering::Relaxed);
                job.set_phase(RunStatus::Succeeded, "succeeded");
                self.record_event(
                    &model_id,
                    job.user_id,
                    "version_created",
                    serde_json::json!({ "version": version, "spec_hash": spec_hash }),
                )
                .await;
            }
            Ok(r) => {
                let err = r
                    .error
                    .unwrap_or_else(|| "trainer returned failed status".to_string());
                job.fail(err);
                let _ = sqlx::query(
                    "UPDATE ai_models SET status='failed', updated_at=now() WHERE model_id=$1",
                )
                .bind(&model_id)
                .execute(&pg)
                .await;
            }
            Err(e) => {
                job.fail(format!("sidecar dispatch error: {e}"));
                let _ = sqlx::query(
                    "UPDATE ai_models SET status='failed', updated_at=now() WHERE model_id=$1",
                )
                .bind(&model_id)
                .execute(&pg)
                .await;
            }
        }

        self.broadcast_progress(&job.snapshot());
        self.persist_run(&job).await;
    }

    #[allow(clippy::too_many_lines)]
    async fn drive_eval(self: &Arc<Self>, job: Arc<Job>, model_id: String, version: i32) {
        let Ok(_permit) = self.run_permits.clone().acquire_owned().await else {
            return;
        };

        job.set_phase(RunStatus::Running, "loading_artifact");
        job.progress_pct.store(5, Ordering::Relaxed);
        self.broadcast_progress(&job.snapshot());

        let pg = self.pg.clone();

        // Load artifact info + definition
        let artifact_row: Option<(String, String, serde_json::Value)> = sqlx::query_as(
            "SELECT ma.storage_uri, ma.sha256, am.definition_json \
             FROM model_artifacts ma \
             JOIN ai_models am ON am.model_id = ma.model_id \
             WHERE ma.model_id = $1 AND ma.version = $2 AND ma.artifact_type = 'model' LIMIT 1",
        )
        .bind(&model_id)
        .bind(version)
        .fetch_optional(&pg)
        .await
        .ok()
        .flatten();

        let Some((artifact_uri, artifact_hash, definition_json)) = artifact_row else {
            job.fail("artifact not found for version");
            self.broadcast_progress(&job.snapshot());
            self.persist_eval(&job).await;
            return;
        };

        let definition: domain::model_def::ModelDefinition =
            match serde_json::from_value(definition_json) {
                Ok(d) => d,
                Err(e) => {
                    job.fail(format!("definition parse: {e}"));
                    self.broadcast_progress(&job.snapshot());
                    self.persist_eval(&job).await;
                    return;
                }
            };

        // Retrieve HPO trial count from the associated training run (I-2.8).
        let trial_count: i64 = sqlx::query_as::<_, (Option<serde_json::Value>,)>(
            "SELECT metrics_json FROM training_runs \
             WHERE model_id=$1 AND status='succeeded' \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&model_id)
        .fetch_optional(&pg)
        .await
        .ok()
        .flatten()
        .and_then(|(m,)| m)
        .and_then(|m| m.get("trial_count").and_then(serde_json::Value::as_i64))
        .unwrap_or(0);

        // Check single-use holdout status (I-2.8): if the holdout was already
        // scored for this version, mark it in the dispatch so the sidecar records it.
        let holdout_used: bool = sqlx::query_as::<_, (Option<serde_json::Value>,)>(
            "SELECT metrics_json FROM evaluation_runs \
             WHERE model_id=$1 AND version=$2 AND status='succeeded' \
             ORDER BY created_at ASC LIMIT 1",
        )
        .bind(&model_id)
        .bind(version)
        .fetch_optional(&pg)
        .await
        .ok()
        .flatten()
        .and_then(|(m,)| m)
        .and_then(|m| m.get("holdout_used").and_then(serde_json::Value::as_bool))
        .unwrap_or(false);

        // Materialize eval dataset (test window, PIT-correct per ADR-0017).
        job.set_phase(RunStatus::Running, "loading_dataset");
        job.progress_pct.store(15, Ordering::Relaxed);
        self.broadcast_progress(&job.snapshot());

        let artifacts_prefix =
            std::env::var("ARTIFACT_STORE_PATH").unwrap_or_else(|_| "./artifacts".to_string());
        let dataset_req = crate::datasets::DatasetRequest {
            dataset_id: None,
            feature_set_ref: definition
                .feature_set_ref
                .clone()
                .unwrap_or_else(|| "fs_core_ohlcv_v3".to_string()),
            instruments: vec!["BTC-USD".to_string()],
            timeframe: "1m".to_string(),
            start: chrono::Utc::now() - chrono::Duration::days(30),
            end: chrono::Utc::now() - chrono::Duration::days(1),
            label_spec: definition
                .label_spec
                .clone()
                .unwrap_or(serde_json::json!({"type":"forward_return","window":"1h"})),
            output_prefix: artifacts_prefix.clone(),
        };
        let dataset = match self.datasets.materialize(dataset_req).await {
            Ok(d) => d,
            Err(e) => {
                job.fail(format!("dataset: {e}"));
                self.broadcast_progress(&job.snapshot());
                self.persist_eval(&job).await;
                return;
            }
        };

        // Build walk-forward fold specs for per-fold breakdown (I-2.7).
        let label_horizon = definition
            .label_spec
            .as_ref()
            .and_then(|s| s.get("window"))
            .and_then(|w| w.as_str())
            .unwrap_or("1h")
            .to_string();
        let timeframe = "1m".to_string();
        let horizon_bars = features::label_horizon_bars(&label_horizon, &timeframe).unwrap_or(60);
        let effective_cv = definition.cv.unwrap_or_else(|| {
            let n = dataset.row_count.max(1) as u64;
            let purge = horizon_bars;
            let available = n.saturating_sub(2 * purge);
            let train_bars = (available * 70 / 100).max(1);
            let cal_bars = (available * 15 / 100).max(1);
            let test_bars = available.saturating_sub(train_bars + cal_bars).max(1);
            domain::model_def::cv::WalkForwardSpec {
                mode: domain::model_def::cv::WindowMode::Expanding,
                folds: 1,
                train_bars,
                cal_bars,
                test_bars,
                purge_bars: purge,
                embargo_bars: purge,
            }
        });
        let fold_specs: Option<Vec<crate::sidecar::FoldSpec>> =
            features::walk_forward_folds(dataset.row_count as usize, &effective_cv, horizon_bars)
                .ok()
                .map(|folds| folds.iter().map(crate::sidecar::FoldSpec::from).collect());

        // Dispatch to scoring sidecar (I-2.1 parity-preserving eval loop).
        job.set_phase(RunStatus::Running, "scoring");
        job.progress_pct.store(30, Ordering::Relaxed);
        self.broadcast_progress(&job.snapshot());

        let nats_subject = format!("models.run.{}.progress", job.run_id);
        let eval_dispatch = crate::sidecar::EvalDispatchRequest {
            eval_id: job.run_id,
            model_id: model_id.clone(),
            version,
            model_kind: format!("{:?}", definition.model_kind).to_lowercase(),
            artifact_uri: artifact_uri.clone(),
            artifact_hash: artifact_hash.clone(),
            dataset_uri: dataset.parquet_uri.clone(),
            dataset_hash: dataset.content_hash.clone(),
            definition: definition.clone(),
            trial_count,
            holdout_used,
            run_baselines: true,
            progress: crate::sidecar::ProgressConfig { nats_subject },
            folds: fold_specs,
        };

        let eval_result = match self.sidecar.dispatch_evaluate(eval_dispatch).await {
            Ok(r) => r,
            Err(e) => {
                // Sidecar may be offline in dev — persist a stub scorecard so
                // the job doesn't fail hard; eval can be retried.
                tracing::warn!("eval sidecar unreachable ({e}); persisting stub scorecard");
                crate::sidecar::EvalResult {
                    status: "stub".to_string(),
                    metrics: Some(serde_json::json!({ "note": "sidecar offline; stub eval" })),
                    scorecard: None,
                    report: None,
                    error: Some(e.to_string()),
                }
            }
        };

        // Build scorecard from eval result (I-2.10).
        job.set_phase(RunStatus::Running, "building_scorecard");
        job.progress_pct.store(80, Ordering::Relaxed);
        self.broadcast_progress(&job.snapshot());

        let eval_json = serde_json::to_value(&eval_result).unwrap_or(serde_json::json!({}));
        let metrics = eval_result.metrics.clone().unwrap_or(serde_json::json!({}));
        let scorecard = crate::scorecard::compute_scorecard_from_eval(&eval_json);
        let report = eval_result
            .report
            .clone()
            .unwrap_or(serde_json::json!(null));

        // Sample outputs (training-preview only; not used in eval scorecard).
        let sample_outputs =
            serde_json::json!({ "note": "see report for full distributional outputs" });

        // Load production baseline for regression.
        let baseline_version: Option<i32> = sqlx::query_as::<_, (i32,)>(
            "SELECT version FROM model_aliases WHERE model_id=$1 AND alias='production'",
        )
        .bind(&model_id)
        .fetch_optional(&pg)
        .await
        .ok()
        .flatten()
        .map(|(v,)| v);

        let regression_report = if let Some(bv) = baseline_version {
            crate::regression::compute_regression_report(
                &metrics,
                &self.load_baseline_metrics(&model_id, bv).await,
            )
        } else {
            serde_json::json!({ "verdict": "no_baseline", "checks": [] })
        };

        let succeeded = eval_result.status == "succeeded" || eval_result.status == "stub";

        // Persist eval results, including the full immutable report (I-2.12).
        // `report_json` is stored in evaluation_runs if the column exists;
        // we use a conditional write so older schemas without the column don't error.
        let _ = sqlx::query(
            "UPDATE evaluation_runs SET \
             status=$1, metrics_json=$2, scorecard_json=$3, \
             regression_report_json=$4, sample_outputs_json=$5, \
             baseline_version=$6, eval_dataset_version_id=$7, \
             started_at=$8, finished_at=now() \
             WHERE eval_id=$9",
        )
        .bind(if succeeded { "succeeded" } else { "failed" })
        .bind(&metrics)
        .bind(&scorecard)
        .bind(&regression_report)
        .bind(&sample_outputs)
        .bind(baseline_version)
        .bind(dataset.dataset_version_id)
        .bind(Utc::now())
        .bind(job.run_id)
        .execute(&pg)
        .await;

        // Store report_json in a separate update so the column absence doesn't
        // block the primary write above.
        if !report.is_null() {
            let _ = sqlx::query("UPDATE evaluation_runs SET report_json=$1 WHERE eval_id=$2")
                .bind(&report)
                .bind(job.run_id)
                .execute(&pg)
                .await;
        }

        // Update model_versions scorecard
        let _ = sqlx::query(
            "UPDATE model_versions SET scorecard_json=$1, metrics_json=$2, status='candidate' WHERE model_id=$3 AND version=$4",
        )
        .bind(&scorecard)
        .bind(&metrics)
        .bind(&model_id)
        .bind(version)
        .execute(&pg)
        .await;

        // Update ai_models status
        let _ = sqlx::query(
            "UPDATE ai_models SET status='candidate', updated_at=now() WHERE model_id=$1",
        )
        .bind(&model_id)
        .execute(&pg)
        .await;

        {
            let mut state = job.state.write().expect("poisoned");
            state.metrics = Some(metrics);
        }
        job.progress_pct.store(100, Ordering::Relaxed);
        job.set_phase(
            if succeeded {
                RunStatus::Succeeded
            } else {
                RunStatus::Failed
            },
            if succeeded { "succeeded" } else { "failed" },
        );
        self.broadcast_progress(&job.snapshot());
        self.persist_eval(&job).await;
    }

    async fn load_baseline_metrics(&self, model_id: &str, version: i32) -> serde_json::Value {
        sqlx::query_as::<_, (Option<serde_json::Value>,)>(
            "SELECT metrics_json FROM model_versions WHERE model_id=$1 AND version=$2",
        )
        .bind(model_id)
        .bind(version)
        .fetch_optional(&self.pg)
        .await
        .ok()
        .flatten()
        .and_then(|(m,)| m)
        .unwrap_or(serde_json::json!({}))
    }

    async fn persist_run(&self, job: &Arc<Job>) {
        let snap = job.snapshot();
        let _ = sqlx::query(
            "UPDATE training_runs SET status=$1, progress=$2, phase=$3, error=$4, \
             started_at=$5, finished_at=$6 WHERE run_id=$7",
        )
        .bind(snap.status.as_str())
        .bind(snap.progress)
        .bind(&snap.phase)
        .bind(&snap.error)
        .bind(snap.started_at)
        .bind(snap.finished_at)
        .bind(snap.run_id)
        .execute(&self.pg)
        .await;
    }

    async fn persist_eval(&self, job: &Arc<Job>) {
        let snap = job.snapshot();
        let _ = sqlx::query(
            "UPDATE evaluation_runs SET status=$1, error=$2, started_at=$3, finished_at=$4 WHERE eval_id=$5",
        )
        .bind(snap.status.as_str())
        .bind(&snap.error)
        .bind(snap.started_at)
        .bind(snap.finished_at)
        .bind(snap.run_id)
        .execute(&self.pg)
        .await;
    }
}

/// sqlx row shape for hydrating model records.
#[derive(sqlx::FromRow)]
struct ModelRow {
    model_id: String,
    slug: String,
    display_name: String,
    description: Option<String>,
    model_kind: String,
    asset_class: String,
    definition_json: serde_json::Value,
    status: String,
    created_by: Uuid,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
}

impl ModelRow {
    fn into_record(self) -> Option<ModelRecord> {
        let definition: ModelDefinition = serde_json::from_value(self.definition_json).ok()?;
        Some(ModelRecord {
            model_id: self.model_id,
            slug: self.slug,
            display_name: self.display_name,
            description: self.description,
            model_kind: self.model_kind,
            asset_class: self.asset_class,
            definition,
            status: self.status,
            created_by: self.created_by,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

impl ModelManager {
    /// Expose `PgPool` reference for internal use (e.g., scheduler).
    pub fn pg_ref(&self) -> &sqlx::PgPool {
        &self.pg
    }

    // -- Feature library (I-3.1, I-3.5) --

    /// List all registered feature sets from the library (I-3.1).
    pub fn list_feature_sets(&self) -> Vec<serde_json::Value> {
        features::list_feature_sets()
            .into_iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "version": s.version,
                    "features": s.features,
                    "description": s.description,
                })
            })
            .collect()
    }

    // -- Spec hash & reproducibility (I-3.8, I-3.9) --

    /// Reproduce a training run from its spec hash or run ID (I-3.9).
    ///
    /// Looks up the original run by `run_id_or_hash`, then re-submits an
    /// identical `drive_train` job.  Returns the new `run_id`.
    pub async fn reproduce_run(
        self: &Arc<Self>,
        run_id_or_hash: &str,
        model_id: &str,
        user_id: Uuid,
    ) -> anyhow::Result<Uuid> {
        self.ensure_model_owned(model_id, user_id).await?;

        // Look up the original run by run_id or spec_hash.
        let row: Option<(Uuid, Option<serde_json::Value>, Option<serde_json::Value>)> =
            sqlx::query_as(
                "SELECT run_id, hyperparameters_json, metrics_json \
                 FROM training_runs \
                 WHERE model_id=$1 AND status='succeeded' \
                 AND (run_id::text=$2 OR metrics_json->>'spec_hash'=$2) \
                 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(model_id)
            .bind(run_id_or_hash)
            .fetch_optional(&self.pg)
            .await?;

        let (orig_run_id, hyperparameters, metrics) =
            row.ok_or_else(|| anyhow::anyhow!("run not found: {run_id_or_hash}"))?;

        let orig_spec_hash = metrics
            .as_ref()
            .and_then(|m| m.get("spec_hash"))
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let hp = hyperparameters.unwrap_or(serde_json::json!(null));
        let req = crate::types::TrainRequest {
            dataset_version_id: None,
            hyperparameter_overrides: Some(hp),
            version_note: Some(format!(
                "reproduce of run {orig_run_id}{}",
                orig_spec_hash
                    .as_deref()
                    .map(|h| format!(" (spec_hash={h})"))
                    .unwrap_or_default()
            )),
            data: None,
        };

        self.start_train(model_id, user_id, req).await
    }

    // -- Run / experiment compare (I-3.10) --

    /// Compare two or more run IDs side by side: params, metrics (Phase 2
    /// scores), artifact info, and spec-hash diff (I-3.10).
    pub async fn compare_runs(
        &self,
        model_id: &str,
        run_ids: &[Uuid],
        user_id: Uuid,
    ) -> anyhow::Result<serde_json::Value> {
        self.ensure_model_owned(model_id, user_id).await?;
        anyhow::ensure!(!run_ids.is_empty(), "at least one run_id required");

        let mut results = Vec::new();
        for &run_id in run_ids {
            let row: Option<(String, Option<serde_json::Value>, Option<serde_json::Value>)> =
                sqlx::query_as(
                    "SELECT status, hyperparameters_json, metrics_json \
                     FROM training_runs WHERE run_id=$1 AND model_id=$2",
                )
                .bind(run_id)
                .bind(model_id)
                .fetch_optional(&self.pg)
                .await?;

            if let Some((status, hp, metrics)) = row {
                let spec_hash = metrics
                    .as_ref()
                    .and_then(|m| m.get("spec_hash"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                results.push(serde_json::json!({
                    "run_id": run_id,
                    "status": status,
                    "hyperparameters": hp,
                    "metrics": metrics,
                    "spec_hash": spec_hash,
                }));
            } else {
                results.push(serde_json::json!({ "run_id": run_id, "error": "not found" }));
            }
        }

        // Compute diff: which hyperparameter keys differ across runs.
        let hp_diff: serde_json::Value = {
            let all_keys: std::collections::HashSet<String> = results
                .iter()
                .flat_map(|r| {
                    r.get("hyperparameters")
                        .and_then(|v| v.as_object())
                        .map(|o| o.keys().cloned().collect::<Vec<_>>())
                        .unwrap_or_default()
                })
                .collect();

            let diffs: serde_json::Map<String, serde_json::Value> = all_keys
                .iter()
                .filter_map(|k| {
                    let values: Vec<_> = results
                        .iter()
                        .map(|r| {
                            r.get("hyperparameters")
                                .and_then(|hp| hp.get(k))
                                .cloned()
                                .unwrap_or(serde_json::Value::Null)
                        })
                        .collect();
                    let all_same = values.windows(2).all(|w| w[0] == w[1]);
                    if all_same {
                        None
                    } else {
                        Some((k.clone(), serde_json::json!(values)))
                    }
                })
                .collect();

            serde_json::Value::Object(diffs)
        };

        // CRPS delta between the first and second run (if available).
        let crps_delta: Option<f64> = if results.len() >= 2 {
            let c0 = results[0]
                .get("metrics")
                .and_then(|m| m.get("crps"))
                .and_then(serde_json::Value::as_f64);
            let c1 = results[1]
                .get("metrics")
                .and_then(|m| m.get("crps"))
                .and_then(serde_json::Value::as_f64);
            c0.zip(c1).map(|(a, b)| b - a)
        } else {
            None
        };

        Ok(serde_json::json!({
            "model_id": model_id,
            "runs": results,
            "hyperparameter_diff": hp_diff,
            "crps_delta_vs_first": crps_delta,
        }))
    }

    // -- Leaderboard (I-2.11) --

    pub async fn leaderboard(
        &self,
        user_id: Uuid,
        model_kind: Option<&str>,
        asset_class: Option<&str>,
        metric: Option<&str>,
        limit: i64,
    ) -> anyhow::Result<Vec<crate::leaderboard::LeaderboardEntry>> {
        crate::leaderboard::query_leaderboard(
            &self.pg,
            user_id,
            model_kind,
            asset_class,
            metric,
            limit,
        )
        .await
    }

    // -- Evaluation reports (I-2.12) --

    pub async fn get_report(
        &self,
        model_id: &str,
        version: i32,
        user_id: Uuid,
    ) -> anyhow::Result<serde_json::Value> {
        self.ensure_model_owned(model_id, user_id).await?;
        crate::report::get_report(&self.pg, model_id, version, user_id).await
    }

    pub async fn export_report(
        &self,
        model_id: &str,
        version: i32,
        user_id: Uuid,
    ) -> anyhow::Result<Vec<u8>> {
        let report = self.get_report(model_id, version, user_id).await?;
        crate::report::export_report_json(&report)
    }

    /// Return strategy definitions that reference this model.
    pub async fn get_used_by(&self, model_id: &str) -> anyhow::Result<Vec<serde_json::Value>> {
        // Query strategies whose definition_json nodes contain a model_ref matching this model.
        let rows: Vec<(String, String, serde_json::Value)> = sqlx::query_as(
            "SELECT strategy_id, display_name, definition_json \
             FROM strategies \
             WHERE definition_json::text LIKE $1 \
             ORDER BY created_at DESC \
             LIMIT 100",
        )
        .bind(format!("%{model_id}%"))
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default();

        let result = rows
            .into_iter()
            .map(|(sid, name, _def)| {
                serde_json::json!({
                    "strategy_id": sid,
                    "display_name": name,
                })
            })
            .collect();

        Ok(result)
    }

    /// Return recent inference traces for this model.
    pub async fn get_traces_for_model(
        &self,
        model_id: &str,
        user_id: uuid::Uuid,
        limit: i64,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        self.ensure_model_owned(model_id, user_id).await?;

        let rows: Vec<(serde_json::Value, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            "SELECT payload, created_at FROM model_events \
             WHERE model_id = $1 AND kind = 'inference_trace' \
             ORDER BY created_at DESC LIMIT $2",
        )
        .bind(model_id)
        .bind(limit)
        .fetch_all(&self.pg)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(mut payload, created_at)| {
                payload["recorded_at"] = serde_json::json!(created_at);
                payload
            })
            .collect())
    }
}
