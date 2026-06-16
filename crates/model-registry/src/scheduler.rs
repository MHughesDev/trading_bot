//! Nightly retrain orchestrator — polls for auto_retrain models on a schedule.

use std::sync::Arc;
use chrono::Utc;
use tokio::time::{interval, Duration};
use uuid::Uuid;
use crate::manager::ModelManager;
use crate::types::TrainRequest;

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

    async fn run_cycle(&self) -> anyhow::Result<()> {
        let rows: Vec<(String, serde_json::Value, Uuid)> = sqlx::query_as(
            r#"SELECT model_id, definition_json, created_by
               FROM ai_models
               WHERE status NOT IN ('archived', 'failed')
                 AND (definition_json -> 'auto_retrain')::boolean = true"#,
        )
        .fetch_all(self.manager.pg_ref())
        .await
        .unwrap_or_default();

        for (model_id, _def_json, created_by) in rows {
            let req = TrainRequest {
                dataset_version_id: None,
                // Auto-retrain reuses the model definition's baked-in
                // hyperparameters; no per-run overrides.
                hyperparameter_overrides: None,
                version_note: Some(format!(
                    "auto-retrain {}",
                    Utc::now().date_naive()
                )),
                // Auto-retrain reuses the model definition's data defaults.
                data: None,
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
