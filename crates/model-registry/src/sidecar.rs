//! Typed HTTP clients for the model-trainer and model-inference Python sidecars.
//! Circuit-breaks on repeated failure; timeouts prevent hangs.

use std::time::Duration;

use domain::model_def::ModelDefinition;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// `features` is used for `Fold → FoldSpec` conversion only.

/// Serializable fold for trainer dispatch — half-open index ranges Rust computed
/// over the pinned dataset; the sidecar slices Parquet by these ranges without
/// ever picking its own split (leakage-structural by design, ADR-0017).
#[derive(Debug, Clone, Serialize)]
pub struct FoldSpec {
    /// Zero-based fold ordinal.
    pub index: u32,
    pub train_start: usize,
    pub train_end: usize,
    pub cal_start: usize,
    pub cal_end: usize,
    pub test_start: usize,
    pub test_end: usize,
}

impl From<&features::Fold> for FoldSpec {
    fn from(f: &features::Fold) -> Self {
        Self {
            index: f.index,
            train_start: f.train.start,
            train_end: f.train.end,
            cal_start: f.cal.start,
            cal_end: f.cal.end,
            test_start: f.test.start,
            test_end: f.test.end,
        }
    }
}

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
    /// Explicit data selection so the trainer pulls real bars from `ClickHouse`.
    /// `None` keeps the legacy synthetic-data path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<TrainDataSpec>,
    /// Walk-forward fold index ranges, Rust-computed from the pinned dataset and
    /// the model's `cv` spec (I-0.10 / ADR-0017). When `Some`, the sidecar uses
    /// these exact ranges to slice the Parquet and train/score per fold. When
    /// `None`, the sidecar falls back to ordinal `split_indices` (back-compat).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folds: Option<Vec<FoldSpec>>,
}

/// Resolved data-selection passed to the trainer sidecar.  Unlike the API-facing
/// `TrainDataSelection`, the window is already resolved to concrete RFC-3339
/// timestamps and the feature set to a concrete column list, so the Python side
/// queries `ClickHouse` without re-resolving anything.
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
    /// The SPEC §9 reason the sidecar classified this failure as, from the
    /// exception's type (`app/failures.py`). `None` on success — and on a
    /// failure from a sidecar too old to send it, which settles as
    /// `dependency_failure`: a dependency raised and did not say why.
    #[serde(default)]
    pub terminal: Option<String>,
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
    /// Present for distributional models (ADR-0016). Absent for point/classification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantile_levels: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantiles_return: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median_return: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sigma: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PredictResponse {
    pub model_id: String,
    pub version: i32,
    pub predictions: Vec<ForecastResponse>,
    pub latency_ms: u64,
}

/// Eval dispatch request: artifact + test-window dataset → scoring sidecar (I-2.1).
#[derive(Debug, Clone, Serialize)]
pub struct EvalDispatchRequest {
    pub eval_id: uuid::Uuid,
    pub model_id: String,
    pub version: i32,
    pub model_kind: String,
    pub artifact_uri: String,
    pub artifact_hash: String,
    pub dataset_uri: String,
    pub dataset_hash: String,
    pub definition: ModelDefinition,
    pub trial_count: i64,
    pub holdout_used: bool,
    pub run_baselines: bool,
    pub progress: ProgressConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folds: Option<Vec<FoldSpec>>,
}

/// Full eval result from the scoring sidecar.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EvalResult {
    pub status: String,
    pub metrics: Option<serde_json::Value>,
    pub scorecard: Option<serde_json::Value>,
    pub report: Option<serde_json::Value>,
    pub error: Option<String>,
    /// See [`TrainResult::terminal`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
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

/// What each outcome costs, in the objective's own units (SPEC §11.4).
///
/// Every field is required. A cost matrix with a default is a decision nobody
/// made, and the threshold that comes out of it is indistinguishable from one
/// that came out of a real one.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CostMatrix {
    pub true_positive: f64,
    pub false_positive: f64,
    pub true_negative: f64,
    pub false_negative: f64,
}

/// The post-hoc stage's request (checklist 2.11, ADR-P2-11).
///
/// There is deliberately no field that reorders, skips or disables a step. The
/// order — soup → greedy ensemble → calibrate → closed-form threshold — is the
/// guarantee, and a request that could change it would be a request that could
/// be retried until the order flattered the result.
#[derive(Debug, Clone, Serialize)]
pub struct PostHocRequest {
    pub posthoc_id: String,
    pub framework: String,
    pub costs: CostMatrix,
    /// `(n_models, n_rows)` out-of-sample predictions from the stored folds.
    pub oos_predictions: Vec<Vec<f64>>,
    pub oos_realized: Vec<f64>,
    /// Scores and outcomes on the dedicated `cal` role — never the train
    /// window, where the model is optimistic, and never the test window, which
    /// is the estimate this exists to protect.
    pub cal_scores: Vec<f64>,
    pub cal_outcomes: Vec<f64>,
    pub calibrator: String,
}

/// One step of the pipeline, including the ones that did not apply.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PostHocStep {
    pub step: String,
    pub applied: bool,
    #[serde(default)]
    pub detail: serde_json::Value,
    #[serde(default)]
    pub skipped: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PostHocResult {
    pub status: String,
    #[serde(default)]
    pub steps: Vec<PostHocStep>,
    #[serde(default)]
    pub weights: Option<Vec<f64>>,
    #[serde(default)]
    pub calibrator: Option<String>,
    #[serde(default)]
    pub calibration_params: Vec<f64>,
    #[serde(default)]
    pub threshold: Option<f64>,
    #[serde(default)]
    pub error: Option<String>,
    /// See [`TrainResult::terminal`].
    #[serde(default)]
    pub terminal: Option<String>,
}

impl PostHocResult {
    /// The step names in the order the sidecar ran them.
    #[must_use]
    pub fn order(&self) -> Vec<&str> {
        self.steps.iter().map(|s| s.step.as_str()).collect()
    }
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

    /// Run the post-hoc pipeline (SPEC §11.4, checklist 2.11).
    ///
    /// # Errors
    /// The sidecar being unreachable, returning a non-success status, or a
    /// response that does not deserialise.
    pub async fn dispatch_posthoc(&self, req: PostHocRequest) -> anyhow::Result<PostHocResult> {
        use std::sync::atomic::Ordering;
        if self.train_failures.load(Ordering::Relaxed) >= 5 {
            anyhow::bail!("trainer sidecar circuit-breaker open");
        }
        let resp = self
            .http
            .post(format!("{}/posthoc", self.trainer_url))
            .json(&req)
            .send()
            .await
            .map_err(|e| {
                self.train_failures.fetch_add(1, Ordering::Relaxed);
                anyhow::anyhow!("trainer posthoc unreachable: {e}")
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            self.train_failures.fetch_add(1, Ordering::Relaxed);
            anyhow::bail!("trainer posthoc returned {status}: {body}");
        }
        self.train_failures.store(0, Ordering::Relaxed);
        let result: PostHocResult = resp.json().await?;
        // The order is not a thing the caller asked for, so it is a thing the
        // caller checks: a sidecar that ran the steps in a different order is
        // not running this pipeline.
        let order = result.order();
        if result.status == "succeeded" && order != ["soup", "ensemble", "calibrate", "threshold"] {
            anyhow::bail!("post-hoc steps came back in the order {order:?}; SPEC §11.4 fixes it");
        }
        Ok(result)
    }

    pub async fn dispatch_evaluate(&self, req: EvalDispatchRequest) -> anyhow::Result<EvalResult> {
        use std::sync::atomic::Ordering;
        if self.train_failures.load(Ordering::Relaxed) >= 5 {
            anyhow::bail!("trainer sidecar circuit-breaker open");
        }
        let resp = self
            .http
            .post(format!("{}/evaluate", self.trainer_url))
            .json(&req)
            .send()
            .await
            .map_err(|e| {
                self.train_failures.fetch_add(1, Ordering::Relaxed);
                anyhow::anyhow!("trainer eval unreachable: {e}")
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            self.train_failures.fetch_add(1, Ordering::Relaxed);
            anyhow::bail!("trainer eval returned {status}: {body}");
        }
        self.train_failures.store(0, Ordering::Relaxed);
        Ok(resp.json::<EvalResult>().await?)
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
