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
    #[error("strategy references unsupported feature '{0}' (supported: ema_N, rsi_N)")]
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

    // EMA converges asymptotically — 5× the longest period is a conservative
    // warm-up; RSI needs period+1.  Floor at 30 bars whenever any indicator
    // is in play so short-period strategies still get a stable lead-in.
    let warmup_bars = specs
        .iter()
        .map(|s| match s.kind {
            FeatureKind::Ema => (s.period as u64) * 5,
            FeatureKind::Rsi => (s.period as u64) + 1,
        })
        .max()
        .map(|w| w.max(30))
        .unwrap_or(0);

    Ok(DataRequirements {
        timeframe,
        features: specs,
        warmup_bars,
    })
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
    let unsupported = || RequirementsError::UnsupportedFeature(name.to_string());
    let (prefix, period) = name.rsplit_once('_').ok_or_else(unsupported)?;
    let period: usize = period.parse().map_err(|_| unsupported())?;
    let kind = match prefix {
        "ema" => FeatureKind::Ema,
        "rsi" => FeatureKind::Rsi,
        _ => return Err(unsupported()),
    };
    if period == 0 || (kind == FeatureKind::Rsi && period < 2) {
        return Err(unsupported());
    }
    Ok(FeatureSpec {
        name: name.to_string(),
        kind,
        period,
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
        assert_eq!(req.warmup_bars, 30); // floor
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
