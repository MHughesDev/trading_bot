//! `EnsembleManager` — first-class ensemble artifact lifecycle (I-4.3, ADR-0018).
//!
//! Mirrors `ModelManager`: persists ensembles + versions + aliases; reuses gated
//! promotion, rollback, and the `models.jobs` WS lane.
//!
//! Postgres tables (migration 0026+):
//!   ensembles        (id TEXT PK, name TEXT, `created_by` TEXT,
//!                     `definition_json` JSONB, `created_at` TIMESTAMPTZ)
//!   `ensemble_versions` (id TEXT PK, `ensemble_id` TEXT, version INT,
//!                      `artifact_uri` TEXT, `artifact_hash` TEXT,
//!                      `metrics_json` JSONB, `scorecard_json` JSONB, `report_json` JSONB,
//!                      `created_at` TIMESTAMPTZ)
//!   `ensemble_aliases`  (`ensemble_id` TEXT, alias TEXT, version INT, `updated_at` TIMESTAMPTZ,
//!                      PRIMARY KEY (`ensemble_id`, alias))
//!   `ensemble_members`  (`ensemble_id` TEXT, version INT, `model_ref` TEXT,
//!                      alias TEXT, sigma DOUBLE PRECISION, crps DOUBLE PRECISION)
//!
//! Uses `sqlx::query()` (runtime verification) so the code compiles before
//! the migration has been applied.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use domain::ensemble_def::{validate_ensemble, EnsembleDefinition};

use crate::sidecar::{
    EnsembleCombineDispatch, EnsembleCombineResult, EnsembleRosterMember, ProgressConfig,
    SidecarClient,
};

/// Thin in-memory record returned from list/get calls.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EnsembleRecord {
    pub id: String,
    pub name: String,
    pub created_by: String,
    pub definition: EnsembleDefinition,
    pub created_at: chrono::DateTime<Utc>,
}

/// Request to create a new ensemble.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateEnsembleRequest {
    pub name: String,
    pub definition: EnsembleDefinition,
}

/// Result of registering a combined ensemble version.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EnsembleVersionRecord {
    pub ensemble_id: String,
    pub version: i32,
    pub artifact_uri: Option<String>,
    pub artifact_hash: Option<String>,
    pub metrics: Option<Value>,
    pub scorecard: Option<Value>,
    pub report: Option<Value>,
    pub created_at: chrono::DateTime<Utc>,
}

pub struct EnsembleManager {
    pg: PgPool,
    sidecar: Arc<SidecarClient>,
    /// Broadcast channel (same WS lane as `ModelManager` — `models.jobs`).
    progress_tx: tokio::sync::broadcast::Sender<Value>,
}

impl EnsembleManager {
    pub fn new(pg: PgPool, sidecar: Arc<SidecarClient>) -> Arc<Self> {
        let (progress_tx, _) = tokio::sync::broadcast::channel(256);
        Arc::new(Self {
            pg,
            sidecar,
            progress_tx,
        })
    }

    pub fn subscribe_progress(&self) -> tokio::sync::broadcast::Receiver<Value> {
        self.progress_tx.subscribe()
    }

    // ── CRUD ─────────────────────────────────────────────────────────────────

    /// Create a new ensemble definition (no version yet; combine to produce one).
    pub async fn create_ensemble(
        self: &Arc<Self>,
        req: CreateEnsembleRequest,
        user_id: &str,
    ) -> anyhow::Result<EnsembleRecord> {
        validate_ensemble(&req.definition)
            .map_err(|errs| anyhow::anyhow!("invalid ensemble definition: {errs:?}"))?;

        let id = format!("ens_{}", Uuid::new_v4().simple());
        let def_json = serde_json::to_value(&req.definition)?;
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO ensembles (id, name, created_by, definition_json, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&id)
        .bind(&req.name)
        .bind(user_id)
        .bind(&def_json)
        .bind(now)
        .execute(&self.pg)
        .await?;

        Ok(EnsembleRecord {
            id,
            name: req.name,
            created_by: user_id.to_string(),
            definition: req.definition,
            created_at: now,
        })
    }

    /// List all ensembles for a user.
    pub async fn list_ensembles(
        self: &Arc<Self>,
        user_id: &str,
    ) -> anyhow::Result<Vec<EnsembleRecord>> {
        let rows: Vec<(String, String, String, Value, chrono::DateTime<Utc>)> = sqlx::query_as(
            "SELECT id, name, created_by, definition_json, created_at \
             FROM ensembles WHERE created_by = $1 ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pg)
        .await?;

        rows.into_iter()
            .map(|(id, name, created_by, def_json, created_at)| {
                let def: EnsembleDefinition = serde_json::from_value(def_json)?;
                Ok(EnsembleRecord {
                    id,
                    name,
                    created_by,
                    definition: def,
                    created_at,
                })
            })
            .collect()
    }

    /// Get one ensemble by ID.
    pub async fn get_ensemble(
        self: &Arc<Self>,
        ensemble_id: &str,
        user_id: &str,
    ) -> anyhow::Result<EnsembleRecord> {
        let row: Option<(String, String, String, Value, chrono::DateTime<Utc>)> = sqlx::query_as(
            "SELECT id, name, created_by, definition_json, created_at \
                 FROM ensembles WHERE id = $1 AND created_by = $2",
        )
        .bind(ensemble_id)
        .bind(user_id)
        .fetch_optional(&self.pg)
        .await?;

        let (id, name, created_by, def_json, created_at) =
            row.ok_or_else(|| anyhow::anyhow!("ensemble not found: {ensemble_id}"))?;
        let def: EnsembleDefinition = serde_json::from_value(def_json)?;
        Ok(EnsembleRecord {
            id,
            name,
            created_by,
            definition: def,
            created_at,
        })
    }

    // ── Combine (drives the Python sidecar) ──────────────────────────────────

    /// Resolve member artifacts from the registry, then dispatch to the sidecar
    /// for combination + scoring.  Persists the resulting version + scorecard.
    ///
    /// `dataset_uri` is a pre-materialized Parquet (test + cal rows).
    /// `cal_start`/`cal_end` are the calibration row index range within it.
    pub async fn drive_combine(
        self: Arc<Self>,
        ensemble_id: &str,
        user_id: &str,
        dataset_uri: &str,
        dataset_hash: &str,
        cal_start: usize,
        cal_end: usize,
        member_sigmas: HashMap<String, f64>,
        member_crps: HashMap<String, f64>,
    ) -> anyhow::Result<EnsembleVersionRecord> {
        let record = self.get_ensemble(ensemble_id, user_id).await?;
        let def = &record.definition;

        // Determine next version number.
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM ensemble_versions WHERE ensemble_id = $1",
        )
        .bind(ensemble_id)
        .fetch_optional(&self.pg)
        .await?;
        let version: i32 = row.map_or(1, |(v,)| v as i32);

        // Build roster for dispatch — resolve member artifact URIs from the registry.
        let mut roster: Vec<EnsembleRosterMember> = Vec::new();
        for member in &def.roster {
            let art: Option<(Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT v.artifact_uri, v.artifact_hash \
                 FROM ai_model_versions v \
                 JOIN ai_model_aliases a ON a.model_id = v.model_id AND a.version = v.version \
                 WHERE v.model_id = $1 AND a.alias = $2 \
                 LIMIT 1",
            )
            .bind(&member.model_ref)
            .bind(&member.alias)
            .fetch_optional(&self.pg)
            .await?;

            let (artifact_uri, artifact_hash) = art
                .map(|(u, h)| (u.unwrap_or_default(), h.unwrap_or_default()))
                .unwrap_or_default();

            roster.push(EnsembleRosterMember {
                model_ref: member.model_ref.clone(),
                alias: member.alias.clone(),
                artifact_uri,
                artifact_hash,
                sigma: member_sigmas.get(&member.model_ref).copied().unwrap_or(1.0),
                crps: member_crps.get(&member.model_ref).copied(),
            });
        }

        let nats_subject = format!("models.jobs.{ensemble_id}.v{version}");
        let dispatch = EnsembleCombineDispatch {
            ensemble_id: ensemble_id.to_string(),
            version,
            roster,
            combiner: def.combiner.clone(),
            weight_floor: def.weight_floor,
            temperature: def.temperature,
            calibration_method: def.calibration.method.clone(),
            calibration_adaptive: def.calibration.adaptive,
            dataset_uri: dataset_uri.to_string(),
            dataset_hash: dataset_hash.to_string(),
            cal_start,
            cal_end,
            levels: None,
            run_baselines: true,
            progress: ProgressConfig { nats_subject },
        };

        let result: EnsembleCombineResult =
            self.sidecar.dispatch_ensemble_combine(dispatch).await?;

        let now = Utc::now();
        let metrics_val = result.metrics.clone();
        let scorecard_val = result.scorecard.clone();
        let report_val = result.report.clone();

        // Persist ensemble version row.
        sqlx::query(
            "INSERT INTO ensemble_versions \
               (id, ensemble_id, version, artifact_uri, artifact_hash, \
                metrics_json, scorecard_json, report_json, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(format!("ensv_{}", Uuid::new_v4().simple()))
        .bind(ensemble_id)
        .bind(version)
        .bind(result.artifact_uri.as_deref())
        .bind(result.artifact_hash.as_deref())
        .bind(&metrics_val)
        .bind(&scorecard_val)
        .bind(&report_val)
        .bind(now)
        .execute(&self.pg)
        .await?;

        // Broadcast to WS lane.
        let _ = self.progress_tx.send(serde_json::json!({
            "kind": "ensemble_version_created",
            "ensemble_id": ensemble_id,
            "version": version,
            "status": result.status,
        }));

        Ok(EnsembleVersionRecord {
            ensemble_id: ensemble_id.to_string(),
            version,
            artifact_uri: result.artifact_uri,
            artifact_hash: result.artifact_hash,
            metrics: metrics_val,
            scorecard: scorecard_val,
            report: report_val,
            created_at: now,
        })
    }

    // ── Alias promotion / rollback (mirrors ModelManager) ────────────────────

    /// Promote an ensemble version to the given alias.
    pub async fn promote_ensemble(
        self: &Arc<Self>,
        ensemble_id: &str,
        version: i32,
        alias: &str,
        user_id: &str,
    ) -> anyhow::Result<()> {
        let _ = self.get_ensemble(ensemble_id, user_id).await?;

        sqlx::query(
            "INSERT INTO ensemble_aliases (ensemble_id, alias, version, updated_at) \
             VALUES ($1, $2, $3, NOW()) \
             ON CONFLICT (ensemble_id, alias) DO UPDATE \
               SET version = EXCLUDED.version, updated_at = NOW()",
        )
        .bind(ensemble_id)
        .bind(alias)
        .bind(version)
        .execute(&self.pg)
        .await?;

        let _ = self.progress_tx.send(serde_json::json!({
            "kind": "ensemble_promoted",
            "ensemble_id": ensemble_id,
            "version": version,
            "alias": alias,
        }));

        Ok(())
    }

    /// List versions for an ensemble.
    pub async fn list_ensemble_versions(
        self: &Arc<Self>,
        ensemble_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Vec<EnsembleVersionRecord>> {
        let _ = self.get_ensemble(ensemble_id, user_id).await?;

        let rows: Vec<(
            i32,
            Option<String>,
            Option<String>,
            Option<Value>,
            Option<Value>,
            Option<Value>,
            chrono::DateTime<Utc>,
        )> = sqlx::query_as(
            "SELECT version, artifact_uri, artifact_hash, \
                    metrics_json, scorecard_json, report_json, created_at \
             FROM ensemble_versions \
             WHERE ensemble_id = $1 \
             ORDER BY version DESC",
        )
        .bind(ensemble_id)
        .fetch_all(&self.pg)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(version, artifact_uri, artifact_hash, metrics, scorecard, report, created_at)| {
                    EnsembleVersionRecord {
                        ensemble_id: ensemble_id.to_string(),
                        version,
                        artifact_uri,
                        artifact_hash,
                        metrics,
                        scorecard,
                        report,
                        created_at,
                    }
                },
            )
            .collect())
    }
}
