use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetField {
    Return,
    Price,
    Volatility,
    Direction,
    Action,
    Score,
    SizeFraction,
    /// Absolute move size (|return|); used for triple-barrier and distributional labeling.
    MoveSize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetTransform {
    #[default]
    None,
    Logret,
    Zscore,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetSpec {
    pub field: TargetField,
    /// ISO-8601-ish horizon token e.g. "1h", "4h", "1d".
    pub horizon: String,
    #[serde(default)]
    pub transform: TargetTransform,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InferenceCfg {
    #[serde(default)]
    pub min_confidence: f64,
    #[serde(default = "default_true")]
    pub calibrate: bool,
}

impl Default for InferenceCfg {
    fn default() -> Self {
        Self {
            min_confidence: 0.0,
            calibrate: true,
        }
    }
}

fn default_true() -> bool {
    true
}
