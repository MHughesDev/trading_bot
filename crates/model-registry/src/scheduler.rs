//! Nightly retrain orchestrator — polls for `auto_retrain` models on a schedule.
//!
//! It only *starts* training: the run lands a candidate version and nothing here
//! promotes it, so no scheduled run can reach `production` without the promotion
//! gate. A model with a run already queued or running is skipped. Evaluation after
//! training is not scheduled here (H-4.8 stays partial on that).

use crate::manager::ModelManager;
use crate::types::TrainRequest;
use chrono::Utc;
use std::sync::Arc;
use tokio::time::{interval, Duration};
use uuid::Uuid;

pub struct RetrainScheduler {
    manager: Arc<ModelManager>,
    period: Duration,
}

impl RetrainScheduler {
    pub fn new(manager: Arc<ModelManager>, period: Duration) -> Self {
        Self { manager, period }
    }

    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = interval(self.period);
            ticker.tick().await; // skip first immediate tick
            loop {
                ticker.tick().await;
                if let Err(e) = self.run_cycle().await {
                    tracing::error!("retrain scheduler error: {e}");
                }
            }
        })
    }

    /// The models due a retrain: `auto_retrain` on, not archived or failed, and with no run
    /// already queued or running (H-4.8). A failed read is an error, not "no model is due": a
    /// scheduler that swallowed it would look idle while retraining nothing.
    ///
    /// # Errors
    /// A Postgres failure.
    pub async fn due_models(
        pg: &sqlx::PgPool,
    ) -> anyhow::Result<Vec<(String, serde_json::Value, Uuid)>> {
        Ok(sqlx::query_as(
            "SELECT model_id, definition_json, created_by \
               FROM ai_models \
               WHERE status NOT IN ('archived', 'failed') \
                 AND (definition_json -> 'auto_retrain')::boolean = true \
                 AND NOT EXISTS (SELECT 1 FROM training_runs r \
                                  WHERE r.model_id = ai_models.model_id \
                                    AND r.status IN ('queued', 'running'))",
        )
        .fetch_all(pg)
        .await?)
    }

    async fn run_cycle(&self) -> anyhow::Result<()> {
        let rows = Self::due_models(self.manager.pg_ref()).await?;

        for (model_id, _def_json, created_by) in rows {
            let req = TrainRequest {
                dataset_version_id: None,
                // Auto-retrain reuses the model definition's baked-in
                // hyperparameters; no per-run overrides.
                hyperparameter_overrides: None,
                version_note: Some(format!("auto-retrain {}", Utc::now().date_naive())),
                // Auto-retrain reuses the model definition's data defaults.
                data: None,
                declaration: None,
                family: None,
                experiment_id: None,
            };

            if let Err(e) = self.manager.start_train(&model_id, created_by, req).await {
                tracing::warn!("auto-retrain start failed for {model_id}: {e}");
            } else {
                tracing::info!("auto-retrain kicked for {model_id}");
            }
        }
        Ok(())
    }
}
