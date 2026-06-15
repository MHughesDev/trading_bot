use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Forecaster,
    SignalRanker,
    TradeDecision,
    RiskSizing,
    Embedding,
    ExternalLlmAdapter,
}

impl ModelKind {
    pub fn is_trainable(self) -> bool {
        !matches!(self, Self::ExternalLlmAdapter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Framework {
    Xgboost,
    Lightgbm,
    Sklearn,
    Torch,
    ExternalApi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    Python,
    Rust,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::Python
    }
}

/// Returns true if the (kind, framework) combination is valid.
pub fn is_compatible(kind: ModelKind, framework: Framework) -> bool {
    match kind {
        ModelKind::ExternalLlmAdapter | ModelKind::Embedding => {
            matches!(framework, Framework::ExternalApi)
        }
        _ => !matches!(framework, Framework::ExternalApi),
    }
}
