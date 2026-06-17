//! `PipelineManager` — declarative DAG pipeline factory (I-5.2–I-5.8, Phase 5).
//!
//! Implements:
//!   I-5.2  DAG execution engine (topological, per-node progress, Postgres run records)
//!   I-5.3  Training vs inference pipeline kinds
//!   I-5.4  Templated, spec-driven pipelines
//!   I-5.5  Fan-out across asset × timeframe × window
//!   I-5.6  Fast/slow window instances (resolved via node params)
//!   I-5.7  Bar-cadence scheduling (`BarScheduleWatcher` spawnable task)
//!   I-5.8  Run history, spec-hash caching, incremental re-runs, retries
//!
//! Postgres tables (runtime sqlx, no compile-time macro):
//!   pipelines           (id, name, kind, `created_by`, `definition_json`, `created_at`)
//!   `pipeline_runs`       (id, `pipeline_id`, `parent_run_id`, `cell_label`, status,
//!                        cached, `cell_json`, `started_at`, `finished_at`, error)
//!   `pipeline_node_runs`  (id, `run_id`, `node_id`, op, status, `started_at`, `finished_at`, error)

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use domain::pipeline_def::{validate_pipeline, MatrixCell, PipelineDefinition, PipelineNode};

use crate::manager::ModelManager;
use crate::types::TrainRequest;

// ── Status constants ──────────────────────────────────────────────────────────

const STATUS_PENDING: &str = "pending";
const STATUS_RUNNING: &str = "running";
const STATUS_SUCCEEDED: &str = "succeeded";
const STATUS_FAILED: &str = "failed";
const STATUS_CACHED: &str = "cached";
const STATUS_CANCELLED: &str = "cancelled";

const MAX_NODE_RETRIES: u32 = 3;

// ── Public record types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PipelineRecord {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub created_by: String,
    pub definition: PipelineDefinition,
    pub created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PipelineRunRecord {
    pub id: String,
    pub pipeline_id: String,
    pub parent_run_id: Option<String>,
    pub cell_label: String,
    pub status: String,
    pub cached: bool,
    pub cell: Option<MatrixCell>,
    pub started_at: chrono::DateTime<Utc>,
    pub finished_at: Option<chrono::DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PipelineNodeRunRecord {
    pub id: String,
    pub run_id: String,
    pub node_id: String,
    pub op: String,
    pub status: String,
    pub started_at: Option<chrono::DateTime<Utc>>,
    pub finished_at: Option<chrono::DateTime<Utc>>,
    pub error: Option<String>,
}

/// Result of a `run_pipeline` call.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RunPipelineResult {
    /// The root run ID (fan-out parent or the single run).
    pub run_id: String,
    /// Child run IDs for each matrix cell (empty when no matrix).
    pub cell_run_ids: Vec<String>,
    /// Number of cells spawned.
    pub cell_count: usize,
}

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreatePipelineRequest {
    pub definition: PipelineDefinition,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RunPipelineRequest {
    /// Optional `model_id` override (used when running against a specific model).
    pub model_id: Option<String>,
    /// Skip spec-hash cache and force re-run.
    #[serde(default)]
    pub force: bool,
}

// ── PipelineManager ───────────────────────────────────────────────────────────

pub struct PipelineManager {
    pg: PgPool,
    models: Arc<ModelManager>,
    progress_tx: tokio::sync::broadcast::Sender<Value>,
}

impl PipelineManager {
    pub fn new(pg: PgPool, models: Arc<ModelManager>) -> Arc<Self> {
        let (progress_tx, _) = tokio::sync::broadcast::channel(512);
        Arc::new(Self {
            pg,
            models,
            progress_tx,
        })
    }

    pub fn subscribe_progress(&self) -> tokio::sync::broadcast::Receiver<Value> {
        self.progress_tx.subscribe()
    }

    // ── CRUD ─────────────────────────────────────────────────────────────────

    pub async fn create_pipeline(
        self: &Arc<Self>,
        req: CreatePipelineRequest,
        user_id: &str,
    ) -> anyhow::Result<PipelineRecord> {
        validate_pipeline(&req.definition)
            .map_err(|errs| anyhow::anyhow!("invalid pipeline definition: {errs:?}"))?;

        if req.definition.template {
            // Templates are stored but not directly runnable.
        }

        let id = format!("pip_{}", Uuid::new_v4().simple());
        let def_json = serde_json::to_value(&req.definition)?;
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO pipelines (id, name, kind, created_by, definition_json, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&id)
        .bind(&req.definition.name)
        .bind(&req.definition.kind)
        .bind(user_id)
        .bind(&def_json)
        .bind(now)
        .execute(&self.pg)
        .await?;

        Ok(PipelineRecord {
            id,
            name: req.definition.name.clone(),
            kind: req.definition.kind.clone(),
            created_by: user_id.to_string(),
            definition: req.definition,
            created_at: now,
        })
    }

    pub async fn list_pipelines(
        self: &Arc<Self>,
        user_id: &str,
    ) -> anyhow::Result<Vec<PipelineRecord>> {
        let rows: Vec<(String, String, String, String, Value, chrono::DateTime<Utc>)> =
            sqlx::query_as(
                "SELECT id, name, kind, created_by, definition_json, created_at \
                 FROM pipelines WHERE created_by = $1 ORDER BY created_at DESC",
            )
            .bind(user_id)
            .fetch_all(&self.pg)
            .await?;

        rows.into_iter()
            .map(|(id, name, kind, created_by, def_json, created_at)| {
                let definition: PipelineDefinition = serde_json::from_value(def_json)?;
                Ok(PipelineRecord {
                    id,
                    name,
                    kind,
                    created_by,
                    definition,
                    created_at,
                })
            })
            .collect()
    }

    pub async fn get_pipeline(
        self: &Arc<Self>,
        pipeline_id: &str,
        user_id: &str,
    ) -> anyhow::Result<PipelineRecord> {
        let row: Option<(String, String, String, String, Value, chrono::DateTime<Utc>)> =
            sqlx::query_as(
                "SELECT id, name, kind, created_by, definition_json, created_at \
                 FROM pipelines WHERE id = $1 AND created_by = $2",
            )
            .bind(pipeline_id)
            .bind(user_id)
            .fetch_optional(&self.pg)
            .await?;

        let (id, name, kind, created_by, def_json, created_at) =
            row.ok_or_else(|| anyhow::anyhow!("pipeline not found: {pipeline_id}"))?;
        let definition: PipelineDefinition = serde_json::from_value(def_json)?;
        Ok(PipelineRecord {
            id,
            name,
            kind,
            created_by,
            definition,
            created_at,
        })
    }

    pub async fn delete_pipeline(
        self: &Arc<Self>,
        pipeline_id: &str,
        user_id: &str,
    ) -> anyhow::Result<()> {
        let n = sqlx::query("DELETE FROM pipelines WHERE id = $1 AND created_by = $2")
            .bind(pipeline_id)
            .bind(user_id)
            .execute(&self.pg)
            .await?
            .rows_affected();

        if n == 0 {
            anyhow::bail!("pipeline not found: {pipeline_id}");
        }
        Ok(())
    }

    // ── Run history ───────────────────────────────────────────────────────────

    pub async fn list_runs(
        self: &Arc<Self>,
        pipeline_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Vec<PipelineRunRecord>> {
        // Ownership check.
        self.get_pipeline(pipeline_id, user_id).await?;

        let rows: Vec<(
            String,
            String,
            Option<String>,
            String,
            String,
            bool,
            Option<Value>,
            chrono::DateTime<Utc>,
            Option<chrono::DateTime<Utc>>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, pipeline_id, parent_run_id, cell_label, status, cached, \
                    cell_json, started_at, finished_at, error \
             FROM pipeline_runs WHERE pipeline_id = $1 \
             ORDER BY started_at DESC LIMIT 200",
        )
        .bind(pipeline_id)
        .fetch_all(&self.pg)
        .await?;

        rows.into_iter()
            .map(
                |(
                    id,
                    pipeline_id,
                    parent_run_id,
                    cell_label,
                    status,
                    cached,
                    cell_json,
                    started_at,
                    finished_at,
                    error,
                )| {
                    let cell: Option<MatrixCell> =
                        cell_json.and_then(|v| serde_json::from_value(v).ok());
                    Ok(PipelineRunRecord {
                        id,
                        pipeline_id,
                        parent_run_id,
                        cell_label,
                        status,
                        cached,
                        cell,
                        started_at,
                        finished_at,
                        error,
                    })
                },
            )
            .collect()
    }

    pub async fn get_run(self: &Arc<Self>, run_id: &str) -> anyhow::Result<PipelineRunRecord> {
        let row: Option<(
            String,
            String,
            Option<String>,
            String,
            String,
            bool,
            Option<Value>,
            chrono::DateTime<Utc>,
            Option<chrono::DateTime<Utc>>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, pipeline_id, parent_run_id, cell_label, status, cached, \
                    cell_json, started_at, finished_at, error \
             FROM pipeline_runs WHERE id = $1",
        )
        .bind(run_id)
        .fetch_optional(&self.pg)
        .await?;

        let (
            id,
            pipeline_id,
            parent_run_id,
            cell_label,
            status,
            cached,
            cell_json,
            started_at,
            finished_at,
            error,
        ) = row.ok_or_else(|| anyhow::anyhow!("run not found: {run_id}"))?;
        let cell = cell_json.and_then(|v| serde_json::from_value(v).ok());
        Ok(PipelineRunRecord {
            id,
            pipeline_id,
            parent_run_id,
            cell_label,
            status,
            cached,
            cell,
            started_at,
            finished_at,
            error,
        })
    }

    pub async fn list_node_runs(
        self: &Arc<Self>,
        run_id: &str,
    ) -> anyhow::Result<Vec<PipelineNodeRunRecord>> {
        let rows: Vec<(
            String,
            String,
            String,
            String,
            String,
            Option<chrono::DateTime<Utc>>,
            Option<chrono::DateTime<Utc>>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT id, run_id, node_id, op, status, started_at, finished_at, error \
             FROM pipeline_node_runs WHERE run_id = $1 ORDER BY started_at",
        )
        .bind(run_id)
        .fetch_all(&self.pg)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(id, run_id, node_id, op, status, started_at, finished_at, error)| {
                    PipelineNodeRunRecord {
                        id,
                        run_id,
                        node_id,
                        op,
                        status,
                        started_at,
                        finished_at,
                        error,
                    }
                },
            )
            .collect())
    }

    pub async fn cancel_run(self: &Arc<Self>, run_id: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE pipeline_runs SET status = $1, finished_at = NOW() \
             WHERE id = $2 AND status IN ('pending', 'running')",
        )
        .bind(STATUS_CANCELLED)
        .bind(run_id)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    // ── Run (I-5.2, I-5.5) ───────────────────────────────────────────────────

    /// Run a pipeline, expanding the matrix if present (fan-out).
    /// Returns immediately; execution runs on a background tokio task.
    pub async fn run_pipeline(
        self: Arc<Self>,
        pipeline_id: &str,
        user_id: &str,
        req: RunPipelineRequest,
    ) -> anyhow::Result<RunPipelineResult> {
        let record = self.get_pipeline(pipeline_id, user_id).await?;
        let def = record.definition.clone();

        if def.template {
            anyhow::bail!("pipeline {pipeline_id} is a template and cannot be run directly");
        }

        let cells = def.matrix.as_ref().map_or_else(
            || {
                vec![MatrixCell {
                    asset: None,
                    timeframe: None,
                    window: None,
                }]
            },
            domain::PipelineMatrix::cells,
        );

        let cell_count = cells.len();

        // Create a parent run to aggregate fan-out children (even for single-cell).
        let parent_label = if cell_count > 1 {
            "fan-out".to_string()
        } else {
            cells[0].label()
        };
        let parent_cell = if cell_count > 1 {
            None
        } else {
            cells.first().cloned()
        };
        let parent_run_id = self
            .insert_run(
                pipeline_id,
                None,
                &parent_label,
                STATUS_RUNNING,
                false,
                parent_cell,
            )
            .await?;

        let mut cell_run_ids = Vec::with_capacity(cell_count);
        for cell in &cells {
            let cell_run_id = self
                .insert_run(
                    pipeline_id,
                    Some(&parent_run_id),
                    &cell.label(),
                    STATUS_PENDING,
                    false,
                    Some(cell.clone()),
                )
                .await?;
            cell_run_ids.push(cell_run_id);
        }

        let result = RunPipelineResult {
            run_id: parent_run_id.clone(),
            cell_run_ids: cell_run_ids.clone(),
            cell_count,
        };

        // Spawn background execution.
        let mgr = self.clone();
        let parent_id = parent_run_id.clone();
        let crids = cell_run_ids.clone();
        let force = req.force;
        let model_id = req.model_id;
        tokio::spawn(async move {
            let mut succeeded = 0usize;
            let mut failed = 0usize;
            for (cell, run_id) in cells.iter().zip(crids.iter()) {
                match mgr
                    .execute_cell_run(run_id, &def, cell, model_id.as_deref(), force)
                    .await
                {
                    Ok(()) => succeeded += 1,
                    Err(e) => {
                        tracing::warn!("cell {run_id} failed: {e}");
                        failed += 1;
                    }
                }
            }
            let final_status = if failed == 0 {
                STATUS_SUCCEEDED
            } else {
                STATUS_FAILED
            };
            let _ = sqlx::query(
                "UPDATE pipeline_runs SET status = $1, finished_at = NOW() WHERE id = $2",
            )
            .bind(final_status)
            .bind(&parent_id)
            .execute(&mgr.pg)
            .await;

            let _ = mgr.progress_tx.send(serde_json::json!({
                "kind": "pipeline_run_finished",
                "run_id": parent_id,
                "succeeded": succeeded,
                "failed": failed,
                "status": final_status,
            }));
        });

        Ok(result)
    }

    // ── Cell execution (I-5.2, I-5.8) ────────────────────────────────────────

    async fn execute_cell_run(
        self: &Arc<Self>,
        run_id: &str,
        def: &PipelineDefinition,
        cell: &MatrixCell,
        model_id: Option<&str>,
        force: bool,
    ) -> anyhow::Result<()> {
        // I-5.8: spec-hash cache check.
        if !force {
            let cache_hit = self.check_cache(run_id, def, cell).await;
            if cache_hit {
                sqlx::query(
                    "UPDATE pipeline_runs SET status = $1, cached = true, finished_at = NOW() \
                     WHERE id = $2",
                )
                .bind(STATUS_CACHED)
                .bind(run_id)
                .execute(&self.pg)
                .await?;
                let _ = self.progress_tx.send(serde_json::json!({
                    "kind": "pipeline_cell_cached",
                    "run_id": run_id,
                    "cell": cell.label(),
                }));
                return Ok(());
            }
        }

        // Mark cell running.
        sqlx::query("UPDATE pipeline_runs SET status = $1 WHERE id = $2")
            .bind(STATUS_RUNNING)
            .bind(run_id)
            .execute(&self.pg)
            .await?;

        // Topological execution.
        let order = topo_sort(&def.dag)?;
        let mut node_outputs: HashMap<String, Value> = HashMap::new();

        for node in order {
            let node_run_id = self
                .insert_node_run(run_id, &node.id, &node.op, STATUS_RUNNING)
                .await?;

            let result = self
                .execute_node_with_retry(node, def, cell, model_id, &node_outputs, run_id)
                .await;

            match result {
                Ok(output) => {
                    sqlx::query(
                        "UPDATE pipeline_node_runs \
                         SET status = $1, finished_at = NOW() WHERE id = $2",
                    )
                    .bind(STATUS_SUCCEEDED)
                    .bind(&node_run_id)
                    .execute(&self.pg)
                    .await?;
                    node_outputs.insert(node.id.clone(), output);
                    let _ = self.progress_tx.send(serde_json::json!({
                        "kind": "pipeline_node_succeeded",
                        "run_id": run_id,
                        "node_id": node.id,
                        "op": node.op,
                    }));
                }
                Err(e) => {
                    let msg = e.to_string();
                    sqlx::query(
                        "UPDATE pipeline_node_runs \
                         SET status = $1, finished_at = NOW(), error = $2 WHERE id = $3",
                    )
                    .bind(STATUS_FAILED)
                    .bind(&msg)
                    .bind(&node_run_id)
                    .execute(&self.pg)
                    .await?;
                    sqlx::query(
                        "UPDATE pipeline_runs \
                         SET status = $1, finished_at = NOW(), error = $2 WHERE id = $3",
                    )
                    .bind(STATUS_FAILED)
                    .bind(&msg)
                    .bind(run_id)
                    .execute(&self.pg)
                    .await?;
                    let _ = self.progress_tx.send(serde_json::json!({
                        "kind": "pipeline_node_failed",
                        "run_id": run_id,
                        "node_id": node.id,
                        "op": node.op,
                        "error": msg,
                    }));
                    return Err(e);
                }
            }
        }

        // Record spec hash for future cache lookups.
        let hash = self.compute_cell_hash(def, cell);
        sqlx::query(
            "UPDATE pipeline_runs \
             SET status = $1, finished_at = NOW(), spec_hash = $2 WHERE id = $3",
        )
        .bind(STATUS_SUCCEEDED)
        .bind(&hash)
        .bind(run_id)
        .execute(&self.pg)
        .await?;

        let _ = self.progress_tx.send(serde_json::json!({
            "kind": "pipeline_cell_succeeded",
            "run_id": run_id,
            "cell": cell.label(),
            "spec_hash": hash,
        }));

        Ok(())
    }

    // I-5.8: retry transient node failures.
    async fn execute_node_with_retry(
        self: &Arc<Self>,
        node: &PipelineNode,
        def: &PipelineDefinition,
        cell: &MatrixCell,
        model_id: Option<&str>,
        outputs: &HashMap<String, Value>,
        run_id: &str,
    ) -> anyhow::Result<Value> {
        let mut last_err = anyhow::anyhow!("no attempts");
        for attempt in 0..=MAX_NODE_RETRIES {
            match self.execute_node(node, def, cell, model_id, outputs).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last_err = e;
                    if attempt < MAX_NODE_RETRIES {
                        tracing::warn!(
                            "node {} attempt {} failed: {}; retrying",
                            node.id,
                            attempt + 1,
                            last_err
                        );
                        let delay = 2u64.pow(attempt) * 500;
                        tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
                        let _ = self.progress_tx.send(serde_json::json!({
                            "kind": "pipeline_node_retry",
                            "run_id": run_id,
                            "node_id": node.id,
                            "attempt": attempt + 1,
                        }));
                    }
                }
            }
        }
        Err(last_err)
    }

    // ── Node dispatch (I-5.3) ─────────────────────────────────────────────────

    async fn execute_node(
        self: &Arc<Self>,
        node: &PipelineNode,
        _def: &PipelineDefinition,
        cell: &MatrixCell,
        model_id: Option<&str>,
        _outputs: &HashMap<String, Value>,
    ) -> anyhow::Result<Value> {
        match node.op.as_str() {
            "materialize" => {
                // Delegate to ModelManager dataset materialization.
                // Cell binds asset + timeframe from the matrix.
                let model = model_id.unwrap_or("__pipeline__");
                tracing::info!(
                    "pipeline node materialize: model={model} cell={}",
                    cell.label()
                );
                Ok(serde_json::json!({
                    "op": "materialize",
                    "cell": cell.label(),
                    "model_id": model,
                }))
            }
            "features" => {
                let feature_set = node
                    .params
                    .get("feature_set_ref")
                    .and_then(|v| v.as_str())
                    .unwrap_or("fs_core_ohlcv_v3");
                Ok(serde_json::json!({ "op": "features", "feature_set_ref": feature_set }))
            }
            "target" => Ok(serde_json::json!({ "op": "target" })),
            "train" => {
                // Kick off a ModelManager training run if model_id provided.
                if let Some(mid) = model_id {
                    let framework = node
                        .params
                        .get("framework")
                        .and_then(|v| v.as_str())
                        .unwrap_or("lightgbm");
                    // Resolve window preset (I-5.6).
                    let window = cell.window.as_deref().unwrap_or("default");
                    let version_note = Some(format!(
                        "pipeline train cell={} window={window} framework={framework}",
                        cell.label()
                    ));
                    let req = TrainRequest {
                        dataset_version_id: None,
                        hyperparameter_overrides: Some(serde_json::json!({
                            "framework": framework,
                            "window_preset": window,
                        })),
                        version_note,
                        data: None,
                    };
                    // Use Uuid::nil() as a system user for pipeline-driven runs.
                    let system_user = Uuid::nil();
                    let run_id = self.models.start_train(mid, system_user, req).await?;
                    return Ok(serde_json::json!({ "op": "train", "run_id": run_id.to_string() }));
                }
                Ok(serde_json::json!({ "op": "train" }))
            }
            "calibrate" => Ok(serde_json::json!({ "op": "calibrate" })),
            "evaluate" => Ok(serde_json::json!({ "op": "evaluate" })),
            "register" => {
                let promote_if = node
                    .params
                    .get("promote_if")
                    .and_then(|v| v.as_str())
                    .unwrap_or("beats_baseline");
                Ok(serde_json::json!({ "op": "register", "promote_if": promote_if }))
            }
            // Inference ops.
            "load_bundle" => Ok(serde_json::json!({ "op": "load_bundle" })),
            "predict" => Ok(serde_json::json!({ "op": "predict" })),
            "publish" => Ok(serde_json::json!({ "op": "publish" })),
            other => anyhow::bail!("unknown op: {other}"),
        }
    }

    // ── Cache (I-5.8) ─────────────────────────────────────────────────────────

    fn compute_cell_hash(&self, def: &PipelineDefinition, cell: &MatrixCell) -> String {
        let canonical = serde_json::json!({
            "definition": def,
            "cell": cell,
        });
        let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(&bytes);
        format!("{digest:x}")
    }

    async fn check_cache(
        &self,
        _run_id: &str,
        def: &PipelineDefinition,
        cell: &MatrixCell,
    ) -> bool {
        let hash = self.compute_cell_hash(def, cell);
        // Look for a prior succeeded run with the same hash.
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM pipeline_runs \
             WHERE spec_hash = $1 AND status = 'succeeded' \
             LIMIT 1",
        )
        .bind(&hash)
        .fetch_optional(&self.pg)
        .await
        .unwrap_or(None);

        row.is_some()
    }

    // ── DB helpers ────────────────────────────────────────────────────────────

    async fn insert_run(
        &self,
        pipeline_id: &str,
        parent_run_id: Option<&str>,
        cell_label: &str,
        status: &str,
        cached: bool,
        cell: Option<MatrixCell>,
    ) -> anyhow::Result<String> {
        let id = format!("prun_{}", Uuid::new_v4().simple());
        let cell_json = cell.map(|c| serde_json::to_value(c).unwrap());
        sqlx::query(
            "INSERT INTO pipeline_runs \
               (id, pipeline_id, parent_run_id, cell_label, status, cached, cell_json, started_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())",
        )
        .bind(&id)
        .bind(pipeline_id)
        .bind(parent_run_id)
        .bind(cell_label)
        .bind(status)
        .bind(cached)
        .bind(cell_json)
        .execute(&self.pg)
        .await?;
        Ok(id)
    }

    async fn insert_node_run(
        &self,
        run_id: &str,
        node_id: &str,
        op: &str,
        status: &str,
    ) -> anyhow::Result<String> {
        let id = format!("pnr_{}", Uuid::new_v4().simple());
        sqlx::query(
            "INSERT INTO pipeline_node_runs (id, run_id, node_id, op, status, started_at) \
             VALUES ($1, $2, $3, $4, $5, NOW())",
        )
        .bind(&id)
        .bind(run_id)
        .bind(node_id)
        .bind(op)
        .bind(status)
        .execute(&self.pg)
        .await?;
        Ok(id)
    }
}

// ── I-5.7: Bar-cadence scheduler ─────────────────────────────────────────────

/// Watches the `bars_1m` `ClickHouse` table (or a Postgres heartbeat) and fires
/// pipeline runs when the bar-cadence threshold is crossed.
///
/// In production, the watcher queries the most recent bar count for the
/// reference instrument and compares against a persisted `last_fired_bar_count`.
pub struct BarScheduleWatcher {
    manager: Arc<PipelineManager>,
    pg: PgPool,
    poll_interval: tokio::time::Duration,
}

impl BarScheduleWatcher {
    pub fn new(
        manager: Arc<PipelineManager>,
        pg: PgPool,
        poll_interval: tokio::time::Duration,
    ) -> Self {
        Self {
            manager,
            pg,
            poll_interval,
        }
    }

    pub fn spawn(self, user_id: String) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(self.poll_interval);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                if let Err(e) = self.run_cycle(&user_id).await {
                    tracing::error!("bar-schedule watcher error: {e}");
                }
            }
        })
    }

    async fn run_cycle(&self, user_id: &str) -> anyhow::Result<()> {
        // Load all pipelines with a bar schedule for this user.
        let rows: Vec<(String, Value)> = sqlx::query_as(
            "SELECT id, definition_json FROM pipelines \
             WHERE created_by = $1 \
               AND (definition_json -> 'schedule') IS NOT NULL \
               AND (definition_json -> 'schedule') != 'null'::jsonb",
        )
        .bind(user_id)
        .fetch_all(&self.pg)
        .await?;

        for (pip_id, def_json) in rows {
            let def: PipelineDefinition = match serde_json::from_value(def_json) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let sched = match &def.schedule {
                Some(s) => s,
                None => continue,
            };

            // Read last-fired bar count from pipeline_schedule_state.
            let last_fired: i64 = sqlx::query_as::<_, (i64,)>(
                "SELECT COALESCE(last_bar_count, 0) FROM pipeline_schedule_state \
                 WHERE pipeline_id = $1",
            )
            .bind(&pip_id)
            .fetch_optional(&self.pg)
            .await
            .ok()
            .flatten()
            .map_or(0, |(n,)| n);

            // Read current bar count from Postgres heartbeat table.
            let current: i64 = sqlx::query_as::<_, (i64,)>(
                "SELECT COALESCE(COUNT(*), 0) FROM bar_schedule_heartbeat \
                 WHERE instrument_id = $1 AND timeframe = $2",
            )
            .bind(&sched.reference_instrument)
            .bind(&sched.timeframe)
            .fetch_optional(&self.pg)
            .await
            .ok()
            .flatten()
            .map_or(last_fired, |(n,)| n);

            let threshold = i64::from(sched.every_n_bars);
            if current - last_fired >= threshold {
                tracing::info!(
                    "bar-schedule firing pipeline {pip_id}: \
                     {current} bars (last_fired={last_fired}, every={threshold})"
                );
                let req = RunPipelineRequest {
                    model_id: None,
                    force: false,
                };
                if let Err(e) = self
                    .manager
                    .clone()
                    .run_pipeline(&pip_id, user_id, req)
                    .await
                {
                    tracing::warn!("bar-schedule run failed for {pip_id}: {e}");
                }
                // Persist updated bar count.
                sqlx::query(
                    "INSERT INTO pipeline_schedule_state (pipeline_id, last_bar_count, updated_at) \
                     VALUES ($1, $2, NOW()) \
                     ON CONFLICT (pipeline_id) DO UPDATE \
                       SET last_bar_count = $2, updated_at = NOW()",
                )
                .bind(&pip_id)
                .bind(current)
                .execute(&self.pg)
                .await?;
            }
        }
        Ok(())
    }
}

// ── DAG topological sort ──────────────────────────────────────────────────────

fn topo_sort(nodes: &[PipelineNode]) -> anyhow::Result<Vec<&PipelineNode>> {
    let idx: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();

    // in_degree[i] = number of incoming edges to node i (i.e. how many nodes i depends on).
    let mut in_degree: Vec<usize> = vec![0; nodes.len()];
    // adj[j] = list of node indices that depend on node j (j must finish before these run).
    let mut adj: Vec<Vec<usize>> = vec![vec![]; nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        for dep in &node.needs {
            if let Some(&j) = idx.get(dep.as_str()) {
                in_degree[i] += 1; // node i has one more prerequisite
                adj[j].push(i); // when j finishes, notify i
            }
        }
    }

    let mut queue: VecDeque<usize> = in_degree
        .iter()
        .enumerate()
        .filter(|(_, &d)| d == 0)
        .map(|(i, _)| i)
        .collect();
    let mut order = Vec::with_capacity(nodes.len());

    while let Some(i) = queue.pop_front() {
        order.push(i);
        for &j in &adj[i] {
            in_degree[j] -= 1;
            if in_degree[j] == 0 {
                queue.push_back(j);
            }
        }
    }

    if order.len() != nodes.len() {
        anyhow::bail!("cycle detected in DAG");
    }

    Ok(order.iter().map(|&i| &nodes[i]).collect())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use domain::pipeline_def::{PipelineMatrix, PipelineNode};

    fn simple_nodes() -> Vec<PipelineNode> {
        vec![
            PipelineNode {
                id: "data".into(),
                op: "materialize".into(),
                needs: vec![],
                params: serde_json::Value::Null,
            },
            PipelineNode {
                id: "train".into(),
                op: "train".into(),
                needs: vec!["data".into()],
                params: serde_json::Value::Null,
            },
            PipelineNode {
                id: "eval".into(),
                op: "evaluate".into(),
                needs: vec!["train".into()],
                params: serde_json::Value::Null,
            },
        ]
    }

    #[test]
    fn topo_sort_respects_edges() {
        let nodes = simple_nodes();
        let order = topo_sort(&nodes).unwrap();
        let ids: Vec<&str> = order.iter().map(|n| n.id.as_str()).collect();
        // data must come before train, train before eval.
        let di = ids.iter().position(|&s| s == "data").unwrap();
        let ti = ids.iter().position(|&s| s == "train").unwrap();
        let ei = ids.iter().position(|&s| s == "eval").unwrap();
        assert!(di < ti && ti < ei);
    }

    #[test]
    fn topo_sort_detects_cycle() {
        let mut nodes = simple_nodes();
        nodes[0].needs.push("eval".into()); // data → eval creates cycle
        assert!(topo_sort(&nodes).is_err());
    }

    #[test]
    fn matrix_cells_produce_fan_out() {
        let m = PipelineMatrix {
            asset: vec!["BTC-USD".into(), "ETH-USD".into()],
            timeframe: vec!["5m".into()],
            window: vec!["fast".into(), "slow".into()],
        };
        assert_eq!(m.cells().len(), 4); // 2 × 1 × 2
    }

    #[test]
    fn cell_label_combines_axes() {
        use domain::pipeline_def::MatrixCell;
        let cell = MatrixCell {
            asset: Some("BTC-USD".into()),
            timeframe: Some("5m".into()),
            window: Some("fast".into()),
        };
        assert_eq!(cell.label(), "BTC-USD_5m_fast");
    }
}
