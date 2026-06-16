//! Typed HTTP clients for the model-trainer and model-inference Python sidecars.
//! Circuit-breaks on repeated failure; timeouts prevent hangs.

use std::time::Duration;

use domain::model_def::ModelDefinition;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct TrainDispatchRequest {
    pub run_id: Uuid,
    pub model_id: String,
    pub model_kind: String,
    pub framework: String,
    pub runtime: String,
    pub definition: ModelDefinition,
    pub dataset_uri: String,
    pub dataset_hash: String,
    pub output_prefix: String,
    pub progress: ProgressConfig,
    /// Explicit data selection so the trainer pulls real bars from ClickHouse.
    /// `None` keeps the legacy synthetic-data path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<TrainDataSpec>,
}

/// Resolved data-selection passed to the trainer sidecar.  Unlike the API-facing
/// `TrainDataSelection`, the window is already resolved to concrete RFC-3339
/// timestamps and the feature set to a concrete column list, so the Python side
/// queries ClickHouse without re-resolving anything.
#[derive(Debug, Clone, Serialize)]
pub struct TrainDataSpec {
    pub instruments: Vec<String>,
    pub timeframe: String,
    /// Inclusive window start (RFC-3339, UTC).
    pub start: String,
    /// Exclusive window end (RFC-3339, UTC).
    pub end: String,
    /// Concrete ordered feature column list to compute.
    pub features: Vec<String>,
    /// Forward-return label horizon, e.g. `"1h"`.
    pub label_horizon: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressConfig {
    pub nats_subject: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrainResult {
    pub status: String,
    pub artifact_uri: Option<String>,
    pub sha256: Option<String>,
    pub size_bytes: Option<i64>,
    pub metrics: Option<serde_json::Value>,
    pub framework_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PredictInstance {
    pub instrument_id: String,
    pub features: std::collections::HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PredictRequest {
    pub model_id: String,
    pub version: i32,
    pub model_kind: String,
    pub artifact_uri: String,
    pub artifact_hash: String,
    pub instances: Vec<PredictInstance>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForecastResponse {
    pub direction: String,
    pub magnitude: String,
    pub confidence: f64,
    pub horizon: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PredictResponse {
    pub model_id: String,
    pub version: i32,
    pub predictions: Vec<ForecastResponse>,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LlmPredictRequest {
    pub model_id: String,
    pub version: i32,
    pub adapter: serde_json::Value,
    pub prompt: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmPredictResponse {
    pub text: String,
    pub tokens: i64,
    pub latency_ms: u64,
    pub cost_usd: String,
    pub trace_id: String,
}

pub struct SidecarClient {
    http: Client,
    trainer_url: String,
    inference_url: String,
    /// consecutive failure counter for basic circuit-break
    train_failures: std::sync::atomic::AtomicU32,
    infer_failures: std::sync::atomic::AtomicU32,
}

impl SidecarClient {
    pub fn from_env() -> Self {
        let trainer_url = std::env::var("MODEL_TRAINER_URL")
            .unwrap_or_else(|_| "http://localhost:8001".to_string());
        let inference_url = std::env::var("MODEL_INFERENCE_URL")
            .unwrap_or_else(|_| "http://localhost:8002".to_string());
        let http = Client::builder()
            .timeout(Duration::from_secs(600))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("http client");
        Self {
            http,
            trainer_url,
            inference_url,
            train_failures: std::sync::atomic::AtomicU32::new(0),
            infer_failures: std::sync::atomic::AtomicU32::new(0),
        }
    }

    pub async fn dispatch_train(&self, req: TrainDispatchRequest) -> anyhow::Result<TrainResult> {
        use std::sync::atomic::Ordering;
        // Basic circuit-break: if 5+ consecutive failures, fail fast.
        if self.train_failures.load(Ordering::Relaxed) >= 5 {
            anyhow::bail!("trainer sidecar circuit-breaker open — too many consecutive failures");
        }
        let resp = self
            .http
            .post(format!("{}/train", self.trainer_url))
            .json(&req)
            .send()
            .await
            .map_err(|e| {
                self.train_failures.fetch_add(1, Ordering::Relaxed);
                anyhow::anyhow!("trainer unreachable: {e}")
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            self.train_failures.fetch_add(1, Ordering::Relaxed);
            anyhow::bail!("trainer returned {status}: {body}");
        }
        self.train_failures.store(0, Ordering::Relaxed);
        Ok(resp.json::<TrainResult>().await?)
    }

    pub async fn predict(&self, req: PredictRequest) -> anyhow::Result<PredictResponse> {
        use std::sync::atomic::Ordering;
        if self.infer_failures.load(Ordering::Relaxed) >= 5 {
            anyhow::bail!("inference sidecar circuit-breaker open");
        }
        let resp = self
            .http
            .post(format!("{}/predict", self.inference_url))
            .json(&req)
            .send()
            .await
            .map_err(|e| {
                self.infer_failures.fetch_add(1, Ordering::Relaxed);
                anyhow::anyhow!("inference unreachable: {e}")
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            self.infer_failures.fetch_add(1, Ordering::Relaxed);
            anyhow::bail!("inference returned {status}: {body}");
        }
        self.infer_failures.store(0, Ordering::Relaxed);
        Ok(resp.json::<PredictResponse>().await?)
    }

    pub async fn predict_llm(&self, req: LlmPredictRequest) -> anyhow::Result<LlmPredictResponse> {
        use std::sync::atomic::Ordering;
        let resp = self
            .http
            .post(format!("{}/predict/llm", self.inference_url))
            .json(&req)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("inference LLM unreachable: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("inference LLM returned {status}: {body}");
        }
        self.infer_failures.store(0, Ordering::Relaxed);
        Ok(resp.json::<LlmPredictResponse>().await?)
    }

    pub async fn trainer_health(&self) -> bool {
        self.http
            .get(format!("{}/health", self.trainer_url))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    pub async fn inference_health(&self) -> bool {
        self.http
            .get(format!("{}/health", self.inference_url))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }
}
