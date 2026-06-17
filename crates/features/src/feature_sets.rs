//! Named, versioned feature-set registry (I-3.1).
//!
//! A feature set is a named, versioned list of feature names the `features`
//! crate can compute.  The registry holds the built-in sets and supports
//! user-defined sets composed from the known-feature namespace (I-3.7).

use std::collections::{HashMap, HashSet};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

/// Version of this feature-set registry — increment when adding/changing built-ins.
pub const REGISTRY_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeatureSetSpec {
    pub name: String,
    pub version: String,
    /// Ordered list of feature names this set produces.
    pub features: Vec<String>,
    pub description: &'static str,
}

// ---------------------------------------------------------------------------
// All features computable by this crate (I-3.2 + I-3.4 families)
// ---------------------------------------------------------------------------

/// Every feature name the crate can compute.  Gated by prefix pattern in
/// `is_known_feature` in `training_frame.rs`; this list is authoritative for
/// validation.
fn known_features_static() -> Vec<&'static str> {
    vec![
        // ── Passthrough ──────────────────────────────────────────────────
        "open", "high", "low", "close", "volume",
        // ── EMA family ───────────────────────────────────────────────────
        "ema_7", "ema_14", "ema_21", "ema_50", "ema_200",
        // ── RSI family ───────────────────────────────────────────────────
        "rsi_14", "rsi_21",
        // ── Rolling moments ──────────────────────────────────────────────
        "rolling_mean_7", "rolling_mean_14", "rolling_mean_20", "rolling_mean_50",
        "rolling_std_7", "rolling_std_14", "rolling_std_20",
        // ── Returns / lags (I-3.2) ───────────────────────────────────────
        "returns_1", "returns_5", "returns_10", "returns_20",
        "log_returns_1",
        // ── Volatility estimators (I-3.2) ────────────────────────────────
        "parkinson_vol_20",   // Parkinson range-based vol, 20-bar rolling
        "garman_klass_vol_20", // Garman–Klass vol, 20-bar rolling
        // ── Momentum (I-3.2) ─────────────────────────────────────────────
        "momentum_5", "momentum_10", "momentum_20",
        // ── Mean-reversion / z-score (I-3.2) ────────────────────────────
        "zscore_20", "zscore_50",
        // ── Volume (I-3.2) ───────────────────────────────────────────────
        "rel_volume_20",
        "obv",               // On-balance volume (cumulative)
        // ── Calendar / session (I-3.2) ───────────────────────────────────
        "hour_sin", "hour_cos",
        "dow_sin",  "dow_cos",
    ]
}

static KNOWN: Lazy<HashSet<&'static str>> =
    Lazy::new(|| known_features_static().into_iter().collect());

// ---------------------------------------------------------------------------
// Built-in feature sets (I-3.1)
// ---------------------------------------------------------------------------

static BUILT_IN_SETS: Lazy<HashMap<String, FeatureSetSpec>> = Lazy::new(|| {
    let mut m = HashMap::new();

    // ── fs_core_ohlcv_v3 (original, unchanged) ────────────────────────────
    m.insert(
        "fs_core_ohlcv_v3".to_string(),
        FeatureSetSpec {
            name: "fs_core_ohlcv_v3".to_string(),
            version: "3".to_string(),
            features: vec![
                "open", "high", "low", "close", "volume",
                "ema_7", "ema_14", "ema_21",
                "rsi_14",
                "rolling_mean_7", "rolling_std_7",
                "returns_1", "log_returns_1",
            ].into_iter().map(str::to_string).collect(),
            description: "Core OHLCV + EMA/RSI/rolling features v3",
        },
    );

    // ── fs_extended_v1 (adds vol estimators + mean-reversion) ────────────
    m.insert(
        "fs_extended_v1".to_string(),
        FeatureSetSpec {
            name: "fs_extended_v1".to_string(),
            version: "1".to_string(),
            features: vec![
                "open", "high", "low", "close", "volume",
                "ema_7", "ema_14", "ema_21", "ema_50",
                "rsi_14", "rsi_21",
                "rolling_mean_20", "rolling_std_20",
                "returns_1", "returns_5", "log_returns_1",
                "parkinson_vol_20", "garman_klass_vol_20",
                "zscore_20",
                "rel_volume_20",
            ].into_iter().map(str::to_string).collect(),
            description: "Extended OHLCV + vol estimators + z-score + relative volume v1",
        },
    );

    // ── fs_momentum_v1 (momentum + calendar) ─────────────────────────────
    m.insert(
        "fs_momentum_v1".to_string(),
        FeatureSetSpec {
            name: "fs_momentum_v1".to_string(),
            version: "1".to_string(),
            features: vec![
                "close", "volume",
                "ema_7", "ema_21", "ema_50",
                "rsi_14",
                "momentum_5", "momentum_10", "momentum_20",
                "returns_1", "returns_5", "returns_10",
                "zscore_20",
                "hour_sin", "hour_cos",
                "dow_sin", "dow_cos",
            ].into_iter().map(str::to_string).collect(),
            description: "Momentum + calendar features v1",
        },
    );

    m
});

// ---------------------------------------------------------------------------
// Registry API (I-3.1)
// ---------------------------------------------------------------------------

/// Resolve a named feature set from the built-in registry or a user-registered one.
pub fn resolve(feature_set_ref: &str) -> Option<&'static FeatureSetSpec> {
    BUILT_IN_SETS.get(feature_set_ref)
}

/// List all registered feature sets.
pub fn list_feature_sets() -> Vec<&'static FeatureSetSpec> {
    let mut sets: Vec<_> = BUILT_IN_SETS.values().collect();
    sets.sort_by_key(|s| s.name.as_str());
    sets
}

/// Validate a list of feature names: returns names that are not computable.
pub fn validate_features(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|n| !is_known(n.as_str()))
        .cloned()
        .collect()
}

/// Whether a feature name can be computed by this crate.
pub fn is_known(name: &str) -> bool {
    if KNOWN.contains(name) {
        return true;
    }
    // Pattern-matched families (variable suffix not enumerated statically).
    name.starts_with("ema_")
        || name.starts_with("rsi_")
        || name.starts_with("rolling_mean_")
        || name.starts_with("rolling_std_")
        || name.starts_with("returns_")
        || name.starts_with("momentum_")
        || name.starts_with("zscore_")
        || name.starts_with("parkinson_vol_")
        || name.starts_with("garman_klass_vol_")
        || name.starts_with("rel_volume_")
}

// ---------------------------------------------------------------------------
// I-3.7  Pluggable user-defined feature sets
//
// A user-defined set is a `FeatureSetSpec` whose feature list is composed
// exclusively of names that `is_known()` returns true for.  No arbitrary code
// execution — the evaluation contract is inherited from the built-in primitives.
// ---------------------------------------------------------------------------

/// Validate a user-defined `FeatureSetSpec` for registration.
///
/// Returns `Ok(())` when every feature name is computable, or `Err` with the
/// list of unknown names so the caller can surface a clear error.
pub fn validate_user_spec(spec: &FeatureSetSpec) -> Result<(), Vec<String>> {
    let unknown = validate_features(&spec.features);
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_built_in_sets_are_valid() {
        for spec in list_feature_sets() {
            let unknown = validate_features(&spec.features);
            assert!(
                unknown.is_empty(),
                "set '{}' contains unknown features: {unknown:?}",
                spec.name
            );
        }
    }

    #[test]
    fn resolve_core_set_exists() {
        let spec = resolve("fs_core_ohlcv_v3").expect("core set must exist");
        assert_eq!(spec.version, "3");
        assert!(!spec.features.is_empty());
    }

    #[test]
    fn resolve_extended_and_momentum_exist() {
        assert!(resolve("fs_extended_v1").is_some());
        assert!(resolve("fs_momentum_v1").is_some());
    }

    #[test]
    fn list_returns_all_three_sets() {
        let sets = list_feature_sets();
        assert_eq!(sets.len(), 3);
    }

    #[test]
    fn unknown_feature_name_rejected() {
        let unknown = validate_features(&["close".to_string(), "not_a_feature".to_string()]);
        assert_eq!(unknown, vec!["not_a_feature".to_string()]);
    }

    #[test]
    fn user_spec_with_unknown_features_fails() {
        let spec = FeatureSetSpec {
            name: "test".to_string(),
            version: "1".to_string(),
            features: vec!["close".to_string(), "alien_signal".to_string()],
            description: "test",
        };
        let result = validate_user_spec(&spec);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), vec!["alien_signal".to_string()]);
    }

    #[test]
    fn user_spec_with_valid_features_passes() {
        let spec = FeatureSetSpec {
            name: "my_set".to_string(),
            version: "1".to_string(),
            features: vec!["close".to_string(), "rsi_14".to_string(), "momentum_10".to_string()],
            description: "test valid",
        };
        assert!(validate_user_spec(&spec).is_ok());
    }
}
