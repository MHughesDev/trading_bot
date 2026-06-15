//! Subscribes to `models.run.*.progress` and drives `ModelManager` job state.
//! Phase 2: real training progress arrives from the Python trainer sidecar.

use std::sync::Arc;

use async_nats::Client as NatsClient;
use futures_util::StreamExt;
use tracing::warn;

#[derive(Debug, serde::Deserialize)]
struct ProgressFrame {
    run_id: uuid::Uuid,
    phase: String,
    progress: f32,
    #[serde(default)]
    metric: Option<serde_json::Value>,
}

pub async fn run(nats: NatsClient, manager: Arc<crate::ModelManager>) {
    let Ok(mut sub) = nats.subscribe("models.run.*.progress").await else {
        warn!("NATS progress bridge: subscribe failed — progress updates disabled");
        return;
    };
    while let Some(msg) = sub.next().await {
        let Ok(frame) = serde_json::from_slice::<ProgressFrame>(&msg.payload) else {
            continue;
        };
        manager
            .apply_nats_progress(frame.run_id, &frame.phase, frame.progress, frame.metric)
            .await;
    }
}
