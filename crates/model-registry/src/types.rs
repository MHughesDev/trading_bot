use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use domain::model_def::ModelDefinition;

/// Lifecycle status of a model training/evaluation job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Queued,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRunKind {
    Train,
    Evaluate,
}

/// Snapshot of a training or evaluation job served to the API/WS.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelRunSnapshot {
    pub run_id: Uuid,
    pub model_id: String,
    pub kind: ModelRunKind,
    pub status: RunStatus,
    /// 0-100.
    pub progress: f32,
    /// Human-readable phase (e.g. "materializing", "fitting", "validating").
    pub phase: String,
    pub error: Option<String>,
    pub metrics: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Request to create a new model definition.
#[derive(Debug, Deserialize)]
pub struct CreateModelRequest {
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub definition: ModelDefinition,
}

/// User-chosen training data: which instruments, timeframe, and how far back.
/// When present on a `TrainRequest`, the trainer pulls these exact bars from
/// `ClickHouse` instead of falling back to synthetic data.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrainDataSelection {
    /// Instrument IDs to train on, e.g. `["BTC-USD"]`.
    pub instruments: Vec<String>,
    /// Bar timeframe key: `"1m" | "5m" | "1h" | ...`.
    #[serde(default = "default_timeframe")]
    pub timeframe: String,
    /// Window size: bars from `now - lookback_days` up to `now`.
    #[serde(default = "default_lookback_days")]
    pub lookback_days: u32,
    /// Override the model's feature set; falls back to the definition / default.
    #[serde(default)]
    pub feature_set_ref: Option<String>,
    /// Forward-return label horizon, e.g. `"1h"`; falls back to the definition.
    #[serde(default)]
    pub label_horizon: Option<String>,
}

fn default_timeframe() -> String {
    "1m".to_string()
}

const fn default_lookback_days() -> u32 {
    30
}

/// Request to start a training run.
#[derive(Debug, Deserialize)]
pub struct TrainRequest {
    #[serde(default)]
    pub dataset_version_id: Option<Uuid>,
    /// Hyperparameter overrides applied on top of the model definition before
    /// dispatch. The UI sends this as `hyperparams`; older callers may send
    /// `hyperparameter_overrides`.
    #[serde(default, alias = "hyperparams")]
    pub hyperparameter_overrides: Option<serde_json::Value>,
    /// Optional note recorded on the resulting model version.
    #[serde(default)]
    pub version_note: Option<String>,
    /// Explicit data selection from the UI.  When omitted, the trainer uses
    /// definition defaults (back-compat with older clients).
    #[serde(default)]
    pub data: Option<TrainDataSelection>,
}

/// A model record as stored and served via the API.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelRecord {
    pub model_id: String,
    pub slug: String,
    pub display_name: String,
    pub description: Option<String>,
    pub model_kind: String,
    pub asset_class: String,
    pub definition: ModelDefinition,
    pub status: String,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
