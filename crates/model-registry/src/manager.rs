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

/// Uniquifier suffix to avoid slug collisions (e.g. "my-model-2").
fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

pub struct ModelManager {
    pg: PgPool,
    jobs: RwLock<HashMap<Uuid, Arc<Job>>>,
    run_permits: Arc<tokio::sync::Semaphore>,
    /// Broadcast channel for WS lane `models.jobs`.
    progress_tx: tokio::sync::broadcast::Sender<serde_json::Value>,
}

impl ModelManager {
    pub fn new(pg: PgPool) -> Arc<Self> {
        let (progress_tx, _) = tokio::sync::broadcast::channel(256);
        Arc::new(Self {
            pg,
            jobs: RwLock::new(HashMap::new()),
            run_permits: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_JOBS)),
            progress_tx,
        })
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
        let slug = slugify(&req.display_name);
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
        .bind(hp)
        .bind(user_id)
        .execute(&self.pg)
        .await;

        self.jobs.write().await.insert(run_id, Arc::clone(&job));

        let manager = Arc::clone(self);
        let mid = model_id.to_string();
        tokio::spawn(async move { manager.drive_train(job, mid).await });

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

    pub async fn promote(&self, model_id: &str, user_id: Uuid, version: i32) -> anyhow::Result<()> {
        self.ensure_model_owned(model_id, user_id).await?;
        // Stub gate: verify version exists.
        let exists: Option<(i32,)> = sqlx::query_as(
            "SELECT version FROM model_versions WHERE model_id = $1 AND version = $2",
        )
        .bind(model_id)
        .bind(version)
        .fetch_optional(&self.pg)
        .await?;
        anyhow::ensure!(exists.is_some(), "version {version} not found");

        sqlx::query(
            "INSERT INTO model_aliases (model_id, alias, version, updated_by, updated_at) \
             VALUES ($1,'production',$2,$3,now()) \
             ON CONFLICT (model_id, alias) DO UPDATE SET version = EXCLUDED.version, \
             updated_by = EXCLUDED.updated_by, updated_at = now()",
        )
        .bind(model_id)
        .bind(version)
        .bind(user_id)
        .execute(&self.pg)
        .await?;

        self.record_event(
            model_id,
            user_id,
            "promoted",
            serde_json::json!({"version": version, "alias": "production"}),
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
        tokio::spawn(async move { manager.drive_eval(job, mid).await });

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
        user_id: Uuid,
    ) -> anyhow::Result<String> {
        self.ensure_model_owned(model_id, user_id).await?;
        let deployment_id = format!("dep_{}", Uuid::new_v4().as_simple());
        sqlx::query(
            "INSERT INTO model_deployments \
             (deployment_id, model_id, version, environment, deployed_by, deployed_at) \
             VALUES ($1,$2,$3,$4,$5,now())",
        )
        .bind(&deployment_id)
        .bind(model_id)
        .bind(version)
        .bind(environment)
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

    // -- Stub job drivers --

    async fn drive_train(self: &Arc<Self>, job: Arc<Job>, _model_id: String) {
        let Ok(_permit) = self.run_permits.clone().acquire_owned().await else {
            return;
        };
        if job.cancel.load(Ordering::Relaxed) {
            job.set_phase(RunStatus::Cancelled, "cancelled");
            return;
        }

        let phases = [
            (10u32, "materializing"),
            (35u32, "fitting"),
            (75u32, "validating"),
            (100u32, "succeeded"),
        ];

        job.set_phase(RunStatus::Running, "materializing");
        for (pct, phase) in &phases {
            if job.cancel.load(Ordering::Relaxed) {
                job.set_phase(RunStatus::Cancelled, "cancelled");
                self.persist_run(&job).await;
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            job.progress_pct.store(*pct, Ordering::Relaxed);
            job.set_phase(
                if *pct < 100 {
                    RunStatus::Running
                } else {
                    RunStatus::Succeeded
                },
                *phase,
            );
            self.broadcast_progress(&job.snapshot());
        }

        // Fake metrics on completion.
        {
            let mut state = job.state.write().expect("poisoned");
            state.metrics = Some(serde_json::json!({ "val_loss": 0.042, "accuracy": 0.91 }));
        }
        self.persist_run(&job).await;
    }

    async fn drive_eval(self: &Arc<Self>, job: Arc<Job>, _model_id: String) {
        let Ok(_permit) = self.run_permits.clone().acquire_owned().await else {
            return;
        };

        job.set_phase(RunStatus::Running, "loading_dataset");
        for (pct, phase) in [
            (30u32, "computing_metrics"),
            (80u32, "building_scorecard"),
            (100u32, "succeeded"),
        ] {
            if job.cancel.load(Ordering::Relaxed) {
                job.set_phase(RunStatus::Cancelled, "cancelled");
                self.persist_eval(&job).await;
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            job.progress_pct.store(pct, Ordering::Relaxed);
            job.set_phase(
                if pct < 100 {
                    RunStatus::Running
                } else {
                    RunStatus::Succeeded
                },
                phase,
            );
            self.broadcast_progress(&job.snapshot());
        }
        self.persist_eval(&job).await;
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
