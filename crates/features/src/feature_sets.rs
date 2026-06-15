//! Named, versioned feature-set registry.
//! A feature set is a named list of features the `features` crate can compute.

use std::collections::HashMap;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeatureSetSpec {
    pub name: String,
    pub version: String,
    /// Ordered list of feature names this set produces.
    pub features: Vec<String>,
    pub description: &'static str,
}

/// All features computable by this crate.
fn known_features() -> &'static [&'static str] {
    &[
        "ema_7",
        "ema_14",
        "ema_21",
        "ema_50",
        "ema_200",
        "rsi_14",
        "rsi_21",
        "rolling_mean_7",
        "rolling_mean_14",
        "rolling_std_7",
        "rolling_std_14",
        "high",
        "low",
        "open",
        "close",
        "volume",
        "returns_1",
        "returns_5",
        "log_returns_1",
    ]
}

static KNOWN: Lazy<std::collections::HashSet<&'static str>> =
    Lazy::new(|| known_features().iter().copied().collect());

static BUILT_IN_SETS: Lazy<HashMap<String, FeatureSetSpec>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert(
        "fs_core_ohlcv_v3".to_string(),
        FeatureSetSpec {
            name: "fs_core_ohlcv_v3".to_string(),
            version: "3".to_string(),
            features: vec![
                "open".to_string(),
                "high".to_string(),
                "low".to_string(),
                "close".to_string(),
                "volume".to_string(),
                "ema_7".to_string(),
                "ema_14".to_string(),
                "ema_21".to_string(),
                "rsi_14".to_string(),
                "rolling_mean_7".to_string(),
                "rolling_std_7".to_string(),
                "returns_1".to_string(),
                "log_returns_1".to_string(),
            ],
            description: "Core OHLCV + EMA/RSI/rolling features v3",
        },
    );
    m
});

pub fn resolve(feature_set_ref: &str) -> Option<&'static FeatureSetSpec> {
    BUILT_IN_SETS.get(feature_set_ref)
}

pub fn validate_features(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|n| !KNOWN.contains(n.as_str()))
        .cloned()
        .collect()
}
