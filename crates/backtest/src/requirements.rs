//! Derives the data requirements of the strategy under test.
//!
//! The collection system is driven by what the *strategy* needs: its bar
//! lane(s) determine the timeframe, and its indicator features determine the
//! warm-up lead-in that must exist before the simulation window starts.

use domain::payloads::bar::Timeframe;
use domain::strategy_def::nodes::NodeKind;
use domain::strategy_def::StrategyDefinition;
use thiserror::Error;

use crate::types::TimeframeExt;

/// An indicator the strategy reads via `feature('name')`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureSpec {
    pub name: String,
    pub kind: FeatureKind,
    pub period: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureKind {
    Ema,
    Rsi,
    /// Simple moving average (`sma_N`, the registry's alias of `rolling_mean_N`).
    Sma,
    /// Wilder average true range (`atr_N`) — needs high/low/close, which every
    /// simulated bar carries.
    Atr,
    /// Any other family in the feature registry (`zscore_N`, `bb_width_N_K`, …).
    /// What a name means is the registry's to say, not this enum's.
    Registry,
}

/// Everything the data pipeline must provide for one backtest run.
#[derive(Clone, Debug)]
pub struct DataRequirements {
    /// Bar timeframe consumed by the strategy.
    pub timeframe: Timeframe,
    /// Indicators to compute during replay.
    pub features: Vec<FeatureSpec>,
    /// Bars of lead-in needed before the window so indicators are warm.
    pub warmup_bars: u64,
}

#[derive(Debug, Error)]
pub enum RequirementsError {
    #[error("strategy references unsupported feature '{0}' (supported: every name `list_features` returns)")]
    UnsupportedFeature(String),
}

/// Derives [`DataRequirements`] from a v1.0 strategy definition.
///
/// `requested` is the timeframe chosen in the backtest form; it is used when
/// the definition does not declare an explicit `market.bars.*` input lane.
pub fn derive_requirements(
    def: &StrategyDefinition,
    requested: Timeframe,
) -> Result<DataRequirements, RequirementsError> {
    // Timeframe: prefer the strategy's own bar lane declaration.
    let mut timeframe = requested;
    for input in &def.inputs {
        if let Some(key) = input.lane.strip_prefix("market.bars.") {
            if let Some(tf) = <Timeframe as TimeframeExt>::from_key(key) {
                timeframe = tf;
                break;
            }
        }
    }

    // Feature names: declared on feature lanes plus any referenced in
    // condition expressions (definitions may rely on either).
    let mut names: Vec<String> = Vec::new();
    for input in &def.inputs {
        for feature in &input.features {
            if !names.contains(feature) {
                names.push(feature.clone());
            }
        }
    }
    for node in &def.nodes {
        let expr = match &node.kind {
            NodeKind::Condition { expr } | NodeKind::Filter { expr, .. } => expr.as_str(),
            _ => continue,
        };
        for name in feature_refs(expr) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }

    let mut specs = Vec::with_capacity(names.len());
    for name in names {
        specs.push(parse_feature(&name)?);
    }

    let warmup_bars = specs
        .iter()
        .map(|s| warmup_bars_for(&s.name))
        .max()
        .map_or(0, |w| w.max(MIN_WARMUP_BARS));

    Ok(DataRequirements {
        timeframe,
        features: specs,
        warmup_bars,
    })
}

/// Floor on warm-up whenever any indicator is in play, so very short-period
/// strategies still get a stable lead-in before the first tradable bar.
const MIN_WARMUP_BARS: u64 = 30;

/// Bars of warm-up lead-in an indicator needs: its declared lookback in the single
/// feature runtime, which is also the first bar at which it has a value at all.
fn warmup_bars_for(name: &str) -> u64 {
    features::runtime::lookback_bars(name).map_or(0, u64::from)
}

/// Extracts the names inside `feature('...')` calls from an expression.
fn feature_refs(expr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = expr;
    while let Some(idx) = rest.find("feature(") {
        rest = &rest[idx + "feature(".len()..];
        let Some(open) = rest.find('\'') else { break };
        let after = &rest[open + 1..];
        let Some(close) = after.find('\'') else { break };
        out.push(after[..close].to_string());
        rest = &after[close + 1..];
    }
    out
}

fn parse_feature(name: &str) -> Result<FeatureSpec, RequirementsError> {
    // One answer to "is this a feature": the registry the engine and
    // `list_features` read. This used to be a hand-kept list of four prefixes, so
    // a strategy naming `zscore_20` was refused before a bar was loaded although
    // the engine computes it and `list_features` advertises it (S-5.2).
    let resolved = features::registry::resolve(name)
        .ok_or_else(|| RequirementsError::UnsupportedFeature(name.to_string()))?;
    let kind = match name.split('_').next().unwrap_or_default() {
        "ema" => FeatureKind::Ema,
        "rsi" => FeatureKind::Rsi,
        "sma" => FeatureKind::Sma,
        "atr" => FeatureKind::Atr,
        _ => FeatureKind::Registry,
    };
    Ok(FeatureSpec {
        name: name.to_string(),
        kind,
        period: resolved.params().first().copied().unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::strategy_def::StrategyDefinition;

    fn def(json: &str) -> StrategyDefinition {
        serde_json::from_str(json).unwrap()
    }

    fn ema_cross_def() -> StrategyDefinition {
        def(r#"{
            "strategy_id": "ema_cross_v1",
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "inputs": [
                { "lane": "market.bars.5m", "instrument": "$bound_at_init" },
                { "lane": "features.technical", "instrument": "$bound_at_init", "features": ["ema_7", "ema_21"] }
            ],
            "nodes": [
                { "id": "n1", "type": "condition", "expr": "feature('ema_7') > feature('ema_21')" },
                { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }
            ],
            "actions": [
                { "on_signal": "long", "type": "place_order",
                  "order": { "side": "buy", "size_mode": "fixed", "size": "0.01" } }
            ]
        }"#)
    }

    #[test]
    fn derives_timeframe_from_bar_lane() {
        let req = derive_requirements(&ema_cross_def(), Timeframe::Minutes1).unwrap();
        assert_eq!(req.timeframe, Timeframe::Minutes5);
    }

    #[test]
    fn derives_features_and_warmup() {
        let req = derive_requirements(&ema_cross_def(), Timeframe::Minutes1).unwrap();
        let names: Vec<&str> = req.features.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["ema_7", "ema_21"]);
        assert_eq!(req.warmup_bars, 105); // 21 * 5
    }

    #[test]
    fn finds_features_referenced_only_in_expressions() {
        let d = def(r#"{
            "strategy_id": "s",
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "inputs": [],
            "nodes": [
                { "id": "n1", "type": "condition", "expr": "feature('rsi_14') < 30" }
            ],
            "actions": []
        }"#);
        let req = derive_requirements(&d, Timeframe::Minutes1).unwrap();
        assert_eq!(req.features.len(), 1);
        assert_eq!(req.features[0].kind, FeatureKind::Rsi);
        assert_eq!(req.warmup_bars, 71); // rsi_14 declares 5 × 14 + 1 bars
    }

    #[test]
    fn unsupported_feature_is_an_error() {
        let d = def(r#"{
            "strategy_id": "s",
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "inputs": [{ "lane": "features.technical", "instrument": "$bound_at_init", "features": ["macd_12"] }],
            "nodes": [],
            "actions": []
        }"#);
        assert!(derive_requirements(&d, Timeframe::Minutes1).is_err());
    }

    #[test]
    fn sma_and_atr_are_derivable_with_the_runtimes_own_warmup() {
        let d = def(r#"{
            "strategy_id": "s",
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "inputs": [],
            "nodes": [
                { "id": "n1", "type": "condition", "expr": "feature('sma_20') > feature('atr_14')" }
            ],
            "actions": []
        }"#);
        let req = derive_requirements(&d, Timeframe::Minutes1).unwrap();
        let kinds: Vec<FeatureKind> = req.features.iter().map(|f| f.kind).collect();
        assert!(kinds.contains(&FeatureKind::Sma) && kinds.contains(&FeatureKind::Atr));
        let atr_lookback = u64::from(features::runtime::lookback_bars("atr_14").unwrap());
        assert!(req.warmup_bars >= atr_lookback, "{} < {atr_lookback}", req.warmup_bars);
    }

    #[test]
    fn sma_and_atr_periods_outside_the_registry_bounds_are_refused() {
        for bad in ["sma_0", "atr_1", "atr_0", "sma_x", "sma"] {
            let d = def(&format!(r#"{{
                "strategy_id": "s", "definition_version": "1.0",
                "asset_class": "crypto_spot_cex", "inputs": [],
                "nodes": [{{ "id": "n1", "type": "condition", "expr": "feature('{bad}') > 1" }}],
                "actions": []
            }}"#));
            assert!(derive_requirements(&d, Timeframe::Minutes1).is_err(), "{bad}");
        }
    }

    #[test]
    fn every_family_in_the_registry_is_derivable_with_the_runtimes_warmup() {
        for f in features::registry::families() {
            let name = f.example();
            let d = def(&format!(r#"{{
                "strategy_id": "s", "definition_version": "1.0",
                "asset_class": "crypto_spot_cex", "inputs": [],
                "nodes": [{{ "id": "n1", "type": "condition", "expr": "feature('{name}') > 1" }}],
                "actions": []
            }}"#));
            let req = derive_requirements(&d, Timeframe::Minutes1)
                .unwrap_or_else(|e| panic!("`{name}`: {e}"));
            let lookback = u64::from(features::runtime::lookback_bars(&name).unwrap());
            assert!(req.warmup_bars >= lookback, "{name}: {} < {lookback}", req.warmup_bars);
        }
    }

    #[test]
    fn no_features_means_no_warmup() {
        let d = def(r#"{
            "strategy_id": "s",
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "inputs": [{ "lane": "market.bars.1m", "instrument": "$bound_at_init" }],
            "nodes": [
                { "id": "n1", "type": "condition", "expr": "bar('close') > 100" }
            ],
            "actions": []
        }"#);
        let req = derive_requirements(&d, Timeframe::Minutes1).unwrap();
        assert_eq!(req.warmup_bars, 0);
    }
}
