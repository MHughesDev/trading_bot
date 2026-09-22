//! Derives the data requirements of the strategy under test.
//!
//! The collection system is driven by what the *strategy* needs: its bar
//! lane(s) determine the timeframe, and its features determine the warm-up
//! lead-in that must exist before the simulation window starts.
//!
//! # What changed, and why it was the platform's ceiling
//!
//! This module used to parse a feature name with a two-arm match — `ema` and
//! `rsi` — and return `UnsupportedFeature` for everything else. The feature
//! engine could already compute rolling statistics, z-scores, volatility
//! estimators and calendar terms, and the training frame used them; a *strategy*
//! naming any of them was refused here, before a bar was loaded. So the
//! archetype space a research agent could search was two moving-average families
//! wide, and the refusal looked like a data problem rather than a grammar one.
//!
//! Now the name is resolved by [`features::registry`], the one catalogue, and
//! the declared lookback comes from the engine rather than from a formula
//! repeated here. A family that exists in the engine is admissible in a
//! backtest, in the live runtime and in an analysis template, because all three
//! ask the same table (AGENT-011 P2, §8.1).

use domain::payloads::bar::Timeframe;
use domain::strategy_def::nodes::NodeKind;
use domain::strategy_def::StrategyDefinition;
use thiserror::Error;

use crate::types::TimeframeExt;

/// A feature the strategy reads via `feature('name')`, resolved against the one
/// catalogue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureSpec {
    /// The name as the definition wrote it, including any alias spelling.
    pub name: String,
    /// The canonical family the name resolves to. `sma_20` and `rolling_mean_20`
    /// carry different `name`s and the same `family`.
    pub family: &'static str,
    /// Declared lookback in bars — read from the engine, never recomputed here.
    /// Recomputing it is how a warm-up requirement comes to disagree with the
    /// window the feature actually reads (INV-13).
    pub lookback_bars: u32,
    /// `cheap | rolling | model`, for the preview's cost estimate.
    pub cost_class: &'static str,
}

/// Everything the data pipeline must provide for one backtest run.
#[derive(Clone, Debug)]
pub struct DataRequirements {
    /// Bar timeframe consumed by the strategy.
    pub timeframe: Timeframe,
    /// Features to compute during replay.
    pub features: Vec<FeatureSpec>,
    /// Bars of lead-in needed before the window so features are warm.
    pub warmup_bars: u64,
}

#[derive(Debug, Error)]
pub enum RequirementsError {
    /// The name resolves to no catalogued family, or a parameter is outside the
    /// family's declared range. The message names the nearest families rather
    /// than the two that used to be the whole world: a model that wrote
    /// `rsi14` has the family right and the syntax wrong, and telling it so is
    /// the difference between one round trip and several (AGENT-011 §5).
    #[error("strategy references unknown feature '{name}'{}", suggestion_tail(.suggestions))]
    UnknownFeature { name: String, suggestions: Vec<String> },
}

impl RequirementsError {
    /// The name that was refused.
    #[must_use]
    pub fn feature_name(&self) -> &str {
        match self {
            Self::UnknownFeature { name, .. } => name,
        }
    }

    /// Catalogued names to try instead, best first.
    #[must_use]
    pub fn suggestions(&self) -> &[String] {
        match self {
            Self::UnknownFeature { suggestions, .. } => suggestions,
        }
    }
}

fn suggestion_tail(suggestions: &[String]) -> String {
    if suggestions.is_empty() {
        " (call list_features to see what exists)".to_string()
    } else {
        format!(" — did you mean {}?", suggestions.join(", "))
    }
}

/// Derives [`DataRequirements`] from a strategy definition.
///
/// `requested` is the timeframe chosen in the backtest form; it is used when
/// the definition does not declare an explicit `market.bars.*` input lane.
///
/// # Errors
/// [`RequirementsError::UnknownFeature`] for a name the catalogue does not hold.
pub fn derive_requirements(def: &StrategyDefinition, requested: Timeframe) -> Result<DataRequirements, RequirementsError> {
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

    let mut specs = Vec::new();
    for name in referenced_features(def) {
        specs.push(resolve(&name)?);
    }

    let warmup_bars = specs.iter().map(|s| u64::from(s.lookback_bars)).max().map_or(0, |w| w.max(MIN_WARMUP_BARS));

    Ok(DataRequirements { timeframe, features: specs, warmup_bars })
}

/// Every feature name the definition mentions, in first-seen order: declared on
/// a feature lane, referenced in a condition or filter expression, or bound as
/// an inference output slot.
///
/// The slot case matters and used to be missed: an `Inference` node publishes
/// `outputs[].as` into the `feature('…')` namespace, so a downstream condition
/// naming it is *not* naming a catalogued feature and must not be refused as
/// one.
#[must_use]
pub fn referenced_features(def: &StrategyDefinition) -> Vec<String> {
    let mut bound: Vec<&str> = Vec::new();
    for node in &def.nodes {
        match &node.kind {
            NodeKind::Inference { outputs, .. } => bound.extend(outputs.iter().map(|o| o.as_.as_str())),
            NodeKind::LlmInference { call, .. } => {
                if let domain::strategy_def::nodes::LlmParseMode::Json { outputs } = &call.parse {
                    bound.extend(outputs.iter().map(|o| o.as_.as_str()));
                } else if let domain::strategy_def::nodes::LlmParseMode::F64 { as_, .. } = &call.parse {
                    bound.push(as_.as_str());
                }
            }
            NodeKind::Combine { as_, .. } => bound.push(as_.as_str()),
            _ => {}
        }
    }

    let mut names: Vec<String> = Vec::new();
    let mut push = |name: String, names: &mut Vec<String>| {
        if !bound.contains(&name.as_str()) && !names.contains(&name) {
            names.push(name);
        }
    };
    for input in &def.inputs {
        for feature in &input.features {
            push(feature.clone(), &mut names);
        }
    }
    for node in &def.nodes {
        match &node.kind {
            NodeKind::Condition { expr } | NodeKind::Filter { expr, .. } => {
                for name in feature_refs(expr) {
                    push(name, &mut names);
                }
            }
            NodeKind::Rank { feature, .. } => push(feature.clone(), &mut names),
            _ => {}
        }
    }
    names
}

/// Resolve one feature name against the catalogue.
///
/// # Errors
/// [`RequirementsError::UnknownFeature`] with suggestions.
pub fn resolve(name: &str) -> Result<FeatureSpec, RequirementsError> {
    let r = features::registry::resolve(name).ok_or_else(|| RequirementsError::UnknownFeature {
        name: name.to_string(),
        suggestions: features::registry::suggestions(name).iter().map(|f| f.example()).collect(),
    })?;
    let lookback_bars = r.lookback_bars().ok_or_else(|| RequirementsError::UnknownFeature { name: name.to_string(), suggestions: vec![] })?;
    Ok(FeatureSpec { name: name.to_string(), family: r.family.name, lookback_bars, cost_class: r.family.cost.as_str() })
}

/// Floor on warm-up whenever any feature is in play, so very short-window
/// strategies still get a stable lead-in before the first tradable bar.
const MIN_WARMUP_BARS: u64 = 30;

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

    fn with_expr(expr: &str) -> StrategyDefinition {
        def(&format!(
            r#"{{
            "strategy_id": "s",
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "inputs": [],
            "nodes": [{{ "id": "n1", "type": "condition", "expr": {} }}],
            "actions": []
        }}"#,
            serde_json::to_string(expr).unwrap()
        ))
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
        let req = derive_requirements(&with_expr("feature('rsi_14') < 30"), Timeframe::Minutes1).unwrap();
        assert_eq!(req.features.len(), 1);
        assert_eq!(req.features[0].family, "rsi");
        assert_eq!(req.warmup_bars, 71); // rsi_14 declares 5 × 14 + 1 bars
    }

    /// The ceiling, gone. Every one of these was `UnsupportedFeature` before this
    /// phase even though the engine computed all of them, and each is a whole
    /// archetype: mean reversion, a volatility gate, a breakout channel, an
    /// ATR-scaled band, a session filter, a persistence filter.
    #[test]
    fn a_strategy_may_now_name_any_catalogued_family() {
        for name in [
            "zscore_20",
            "pct_rank_250",
            "atr_14",
            "macd_hist_12_26_9",
            "bb_pctb_20_2",
            "donchian_upper_55",
            "keltner_upper_20_2",
            "rolling_quantile_100_90",
            "utc_session_us",
            "efficiency_ratio_30",
            "variance_ratio_60_5",
            "jump_share_30",
            "rel_volume_20",
            "sma_50",
        ] {
            let req = derive_requirements(&with_expr(&format!("feature('{name}') > 0")), Timeframe::Hours1)
                .unwrap_or_else(|e| panic!("{name} refused: {e}"));
            assert_eq!(req.features.len(), 1, "{name}");
            assert!(req.warmup_bars >= u64::from(req.features[0].lookback_bars), "{name}");
        }
    }

    /// The warm-up is the engine's declared lookback, not a formula this module
    /// keeps its own copy of. If the two ever disagree the executor loads a
    /// lead-in that does not cover the window, and the first bars of every run
    /// are computed from bars that are not there (INV-13).
    #[test]
    fn warmup_is_the_engines_declared_lookback_for_the_longest_feature() {
        let d = with_expr("feature('ema_200') > feature('rsi_14') + feature('zscore_20')");
        let req = derive_requirements(&d, Timeframe::Hours1).unwrap();
        for f in &req.features {
            assert_eq!(Some(f.lookback_bars), features::runtime::lookback_bars(&f.name), "{}", f.name);
        }
        assert_eq!(req.warmup_bars, 1_000); // ema_200: 200 × 5, the longest
    }

    /// Adversarial: a name outside the catalogue is still refused, and the
    /// refusal now teaches. Opening the space is not the same as opening it to
    /// anything — an unresolvable name has no declared lookback, so nothing
    /// could say what lead-in it needs.
    #[test]
    fn an_uncatalogued_feature_is_refused_with_the_nearest_families_named() {
        let err = derive_requirements(&with_expr("feature('rsi14') < 30"), Timeframe::Minutes1).unwrap_err();
        assert_eq!(err.feature_name(), "rsi14");
        assert!(err.suggestions().iter().any(|s| s.starts_with("rsi_")), "{:?}", err.suggestions());
        assert!(err.to_string().contains("did you mean"), "{err}");
    }

    #[test]
    fn a_parameter_outside_its_declared_range_is_refused() {
        for bad in ["ema_0", "rsi_1", "rolling_std_1", "bb_upper_20_0", "rolling_quantile_20_101"] {
            let d = with_expr(&format!("feature('{bad}') > 0"));
            assert!(derive_requirements(&d, Timeframe::Minutes1).is_err(), "{bad} should be refused");
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

    /// A slot an inference node publishes is not a catalogued feature and must
    /// not be resolved as one. Before this, such a strategy was refused with
    /// "unsupported feature 'model_edge'", which is a true statement about the
    /// catalogue and the wrong answer about the strategy.
    #[test]
    fn a_feature_slot_bound_by_an_inference_node_is_not_looked_up_in_the_catalogue() {
        let d = def(r#"{
            "strategy_id": "s",
            "definition_version": "1.1",
            "asset_class": "crypto_spot_cex",
            "inputs": [],
            "nodes": [
                { "id": "inf", "type": "inference", "model_ref": "m",
                  "outputs": [{ "field": "median_return", "as": "model_edge" }] },
                { "id": "n1", "type": "condition", "expr": "feature('model_edge') > 0 and feature('ema_7') > 0" }
            ],
            "actions": []
        }"#);
        let req = derive_requirements(&d, Timeframe::Hours1).unwrap();
        let names: Vec<&str> = req.features.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["ema_7"], "the bound slot must not be resolved against the catalogue");
    }

    /// An alias carries its own name and the canonical family, so a run records
    /// what the author wrote and what was computed.
    #[test]
    fn an_alias_keeps_its_spelling_and_names_its_canonical_family() {
        let req = derive_requirements(&with_expr("feature('sma_50') > feature('momentum_10')"), Timeframe::Hours1).unwrap();
        assert_eq!(req.features[0].name, "sma_50");
        assert_eq!(req.features[0].family, "rolling_mean");
        assert_eq!(req.features[1].family, "returns");
    }
}
