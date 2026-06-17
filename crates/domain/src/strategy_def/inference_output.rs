//! The output envelope that an inference node publishes after each evaluation.
//!
//! `InferenceOutput` is the common carrier that flows from the inference gateway
//! (model-registry) into the strategy runtime (strategy-runtime).  The runtime
//! writes bound fields into feature slots so that downstream `Condition`, `Filter`,
//! `Rank`, and `Sizing` nodes can consume them without any new grammar extension.
//!
//! # Field naming for `OutputBinding.field`
//!
//! | `field` value          | Source                                               |
//! |------------------------|------------------------------------------------------|
//! | `"confidence"`         | `confidence` (0–1)                                   |
//! | `"direction"`          | +1.0 bullish / 0.0 flat / -1.0 bearish             |
//! | `"median_return"`      | distributional median (None for point models)        |
//! | `"sigma"`              | realized-vol scale (None for point models)           |
//! | `"q05"`…`"q95"`        | return-unit quantile at that level (None if absent)  |
//! | `"var_95"` / `"var_99"`| Value-at-Risk (loss quantile, None if not computed)  |
//! | `"es_95"`              | Expected Shortfall tail mean                         |
//! | `"skew"`               | (q75-q50)-(q50-q25) / σ                            |
//! | `"spread_90"`          | q95-q05 in return units                              |

/// Full output from one evaluation of an `Inference`, `Sizing`, `Decision`, or
/// `LlmInference` node. Produced by the inference gateway; consumed by the runtime.
#[derive(Debug, Clone, Default)]
pub struct InferenceOutput {
    // ── Core forecast fields ──────────────────────────────────────────────────
    /// Forecaster direction: `"up"` | `"down"` | `"flat"` (also `"bullish"` /
    /// `"bearish"` for legacy sidecar responses).
    pub direction: String,
    /// Calibrated confidence score, 0.0–1.0.
    pub confidence: f64,

    // ── Distributional fields (None for point/classification models) ──────────
    pub median_return: Option<f64>,
    pub sigma: Option<f64>,
    /// Sorted probability levels matching `quantiles_return`.
    pub quantile_levels: Option<Vec<f64>>,
    /// Return-unit quantile values, sorted ascending.
    pub quantiles_return: Option<Vec<f64>>,

    // ── Derived risk (computed from distribution if available) ────────────────
    pub var_95: Option<f64>,
    pub var_99: Option<f64>,
    pub es_95: Option<f64>,
    pub skew: Option<f64>,
    pub spread_90: Option<f64>,

    // ── Kind-specific fields ──────────────────────────────────────────────────
    /// Decimal-string size fraction from a `RiskSizing` model (e.g. `"0.02"`).
    /// Kept as `String` so the action boundary can parse it to `Decimal` (ADR-0002).
    pub size_fraction: Option<String>,
    /// Predicted action class from a `TradeDecision` model (e.g. `"long"`, `"flat"`).
    pub action_class: Option<String>,
}

impl InferenceOutput {
    /// Resolve a named output field to an `f64` suitable for a feature slot.
    ///
    /// Returns `None` when the field is not available (e.g. requesting
    /// `"median_return"` for a point model that doesn't produce a distribution).
    pub fn get_field(&self, field: &str) -> Option<f64> {
        match field {
            "confidence" => Some(self.confidence),
            "direction" => Some(self.direction_f64()),
            "median_return" => self.median_return,
            "sigma" => self.sigma,
            "var_95" => self.var_95,
            "var_99" => self.var_99,
            "es_95" => self.es_95,
            "skew" => self.skew,
            "spread_90" => self.spread_90,
            // Quantile shorthand: "q05" → level 0.05, "q50" → 0.50, etc.
            q if q.starts_with('q') && q.len() >= 3 => self.quantile_by_level(q),
            _ => None,
        }
    }

    fn direction_f64(&self) -> f64 {
        match self.direction.to_lowercase().as_str() {
            "up" | "bullish" => 1.0,
            "down" | "bearish" => -1.0,
            _ => 0.0,
        }
    }

    /// Look up a quantile value by level string (e.g. `"q05"` → level 0.05).
    fn quantile_by_level(&self, field: &str) -> Option<f64> {
        let level_str = field.strip_prefix('q')?;
        // "05" → 5 → 0.05, "50" → 50 → 0.50, "95" → 95 → 0.95
        let level_int: u32 = level_str.parse().ok()?;
        let level = level_int as f64 / 100.0;
        let levels = self.quantile_levels.as_ref()?;
        let returns = self.quantiles_return.as_ref()?;
        levels
            .iter()
            .zip(returns.iter())
            .min_by(|(a, _), (b, _)| {
                (*a - level)
                    .abs()
                    .partial_cmp(&(*b - level).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(_, v)| *v)
    }

    /// Check whether a `direction` string matches the expected direction from
    /// a `ModelForecast` / `Inference` node.
    ///
    /// `"any"` always matches.  Bullish/bearish map from both the sidecar
    /// convention (`"up"` / `"down"`) and the builder convention
    /// (`"bullish"` / `"bearish"`).
    pub fn direction_matches(&self, expected: &str) -> bool {
        if expected == "any" {
            return true;
        }
        let d = self.direction.to_lowercase();
        matches!(
            (d.as_str(), expected),
            ("up" | "bullish", "bullish") | ("down" | "bearish", "bearish") | ("flat", "flat")
        ) || d == expected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(direction: &str, confidence: f64) -> InferenceOutput {
        InferenceOutput {
            direction: direction.into(),
            confidence,
            ..Default::default()
        }
    }

    #[test]
    fn direction_f64_maps_correctly() {
        assert_eq!(make("up", 0.0).get_field("direction"), Some(1.0));
        assert_eq!(make("bullish", 0.0).get_field("direction"), Some(1.0));
        assert_eq!(make("down", 0.0).get_field("direction"), Some(-1.0));
        assert_eq!(make("flat", 0.0).get_field("direction"), Some(0.0));
    }

    #[test]
    fn confidence_field_works() {
        assert_eq!(make("up", 0.82).get_field("confidence"), Some(0.82));
    }

    #[test]
    fn quantile_lookup_finds_nearest() {
        let o = InferenceOutput {
            quantile_levels: Some(vec![0.05, 0.25, 0.50, 0.75, 0.95]),
            quantiles_return: Some(vec![-0.03, -0.01, 0.002, 0.012, 0.04]),
            ..Default::default()
        };
        assert_eq!(o.get_field("q05"), Some(-0.03));
        assert_eq!(o.get_field("q50"), Some(0.002));
        assert_eq!(o.get_field("q95"), Some(0.04));
    }

    #[test]
    fn direction_matches_any() {
        let o = make("up", 0.9);
        assert!(o.direction_matches("any"));
        assert!(o.direction_matches("bullish"));
        assert!(!o.direction_matches("bearish"));
    }
}
