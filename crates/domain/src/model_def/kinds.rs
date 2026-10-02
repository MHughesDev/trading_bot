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
    /// Predicts one label from a set, with class probabilities (FEAT-007 §8.1).
    Classifier,
    /// Predicts one real value.
    Regressor,
    /// Custom code with a declared contract and no training — the escape hatch
    /// for anything the strategy operator library cannot express (ADR-0040 §5).
    Transform,
}

impl ModelKind {
    pub fn is_trainable(self) -> bool {
        !matches!(self, Self::ExternalLlmAdapter | Self::Transform)
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
    /// Bring-your-own code: a snapshot run in the trainer/inference sandbox
    /// (FEAT-006 §5).
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    #[default]
    Python,
    Rust,
}

/// Returns true if the (kind, framework) combination is valid.
pub fn is_compatible(kind: ModelKind, framework: Framework) -> bool {
    match kind {
        ModelKind::ExternalLlmAdapter | ModelKind::Embedding => {
            matches!(framework, Framework::ExternalApi)
        }
        ModelKind::Transform => matches!(framework, Framework::Custom),
        _ => !matches!(framework, Framework::ExternalApi),
    }
}
