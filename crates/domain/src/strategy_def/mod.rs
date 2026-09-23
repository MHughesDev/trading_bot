//! Strategy-definition format, frozen at **v1.0**.
//!
//! The canonical example:
//!
//! ```json
//! {
//!   "strategy_id": "ema_cross_v1",
//!   "definition_version": "1.0",
//!   "asset_class": "crypto_spot_cex",
//!   "min_trust_tier": "centralized_exchange",
//!   "inputs": [...],
//!   "nodes": [...],
//!   "actions": [...],
//!   "risk_overrides": { "max_position": "0.5" }
//! }
//! ```
//!
//! See `nodes.rs` for the frozen expression grammar.  See `risk_overrides.rs`
//! for the tighten-only invariant.

pub mod actions;
pub mod exits;
pub mod inference_output;
pub mod inputs;
pub mod nodes;
pub mod params;
pub mod revision;
pub mod risk_overrides;

pub use inference_output::InferenceOutput;
pub use params::{ParamSpec, ParamValues};

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::trust::TrustTier;
use actions::Action;
use exits::ExitPolicy;
use inputs::InputDeclaration;
use nodes::Node;
use risk_overrides::RiskOverrides;

/// The format version this module implements.  This string is frozen.
/// Future breaking changes introduce a new version string value, not an
/// in-place edit of the v1.0 schema.
pub const DEFINITION_VERSION: &str = "1.0";
/// Format version for v1.1 additions (ModelForecast node).
pub const DEFINITION_VERSION_V1_1: &str = "1.1";
/// Format version for v1.2 additions (typed `parameters` block, `param('x')`
/// and `{{x}}` references — see [`params`]). Additive: a v1.0 document is a
/// valid v1.2 document.
pub const DEFINITION_VERSION_V1_2: &str = "1.2";
/// Format version for v1.3 additions (the typed `exits` block — see [`exits`]).
///
/// Additive in the parser and **not** additive in meaning: a document below 1.3
/// has no exit policy at all, which is why [`StrategyDefinition::exit_policy`]
/// reports the absence rather than inventing [`ExitPolicy::RiskLayerOnly`] on
/// its behalf. The two look identical at runtime and are different claims.
pub const DEFINITION_VERSION_V1_3: &str = "1.3";
/// Format version for v1.4 additions (the `unit` on an exit distance — see
/// [`exits::DistanceUnit`] and ADR-D-09).
///
/// Additive in both the parser and the meaning, which 1.3 was not: the unit
/// defaults to [`exits::DistanceUnit::FractionOfEntry`] and is omitted from the
/// serialized form when it holds that value, so a 1.0/1.2/1.3 document parses
/// and re-serializes **byte for byte** unchanged. What 1.4 adds is the ability
/// to *say* `atr` or `sigma`, which no earlier version could store at all.
pub const DEFINITION_VERSION_V1_4: &str = "1.4";

/// A complete strategy definition — the artifact all three front doors produce.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrategyDefinition {
    /// Human-readable slug (e.g. `"ema_cross_v1"`).
    pub strategy_id: String,
    /// Must equal `"1.0"` for this format.  The validator rejects unknown versions.
    pub definition_version: String,
    /// Scopes which instruments this definition may be initialized on.
    /// Stored as a string to remain forward-compatible (e.g. `"crypto_spot_cex"`).
    pub asset_class: String,
    /// Minimum source trust required.  Defaults to `CentralizedExchange`.
    #[serde(default = "default_min_trust_tier")]
    pub min_trust_tier: TrustTier,
    pub inputs: Vec<InputDeclaration>,
    pub nodes: Vec<Node>,
    pub actions: Vec<Action>,
    #[serde(default)]
    pub risk_overrides: RiskOverrides,
    /// Typed, sweepable parameters (v1.2). Empty for v1.0/v1.1 documents.
    /// Referenced from expressions as `param('name')` and from any string as
    /// `{{name}}`; substituted by [`params::materialize`] before execution.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, ParamSpec>,
    /// Cross-parameter constraints, e.g. `"param('fast') < param('slow')"`.
    /// A sample violating one is rejected before a Run exists.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
    /// What closes a position (v1.3). REQUIRED at version 1.3 and above; the
    /// validator refuses a 1.3 document without one, and refuses a `Rules`
    /// policy with an empty rule list.
    ///
    /// `None` here means "this document predates 1.3", which is a different
    /// statement from [`ExitPolicy::RiskLayerOnly`] and must never be collapsed
    /// into it: one is a strategy that delegates its exits, the other is a
    /// strategy from before the question was asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exits: Option<ExitPolicy>,
}

impl StrategyDefinition {
    /// The declared exit policy, or `None` for a pre-1.3 document.
    #[must_use]
    pub fn exit_policy(&self) -> Option<&ExitPolicy> {
        self.exits.as_ref()
    }

    /// The holding period this definition *declares*, in bars.
    ///
    /// Only an [`exits::ExitRule::MaxHoldingBars`] produces one. Everything else
    /// — a stop, a target, a trailing stop, a reversal signal — bounds price or
    /// events, not time, and a strategy whose holding period is emergent cannot
    /// be checked against the tradeable frontier before it runs. That is not a
    /// reason to guess: it returns `None` and the caller decides.
    #[must_use]
    pub fn declared_holding_bars(&self) -> Option<u32> {
        self.exits
            .as_ref()?
            .rules()
            .iter()
            .filter_map(exits::ExitRule::holding_bound)
            .min()
    }
}

fn default_min_trust_tier() -> TrustTier {
    TrustTier::CentralizedExchange
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical example from DATA-004 §3.1 must deserialize and re-serialize stably.
    #[test]
    fn canonical_example_round_trips() {
        let json = r#"{
            "strategy_id": "ema_cross_v1",
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "min_trust_tier": "centralized_exchange",
            "inputs": [
                { "lane": "market.bars.1m", "instrument": "$bound_at_init" },
                { "lane": "features.technical", "instrument": "$bound_at_init", "features": ["ema_7", "ema_21"] }
            ],
            "nodes": [
                { "id": "n1", "type": "condition", "expr": "feature('ema_7') > feature('ema_21')" },
                { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }
            ],
            "actions": [
                {
                    "on_signal": "long",
                    "type": "place_order",
                    "order": { "side": "buy", "size_mode": "fixed", "size": "0.01" }
                }
            ],
            "risk_overrides": { "max_position": "0.5" }
        }"#;

        let def: StrategyDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(def.definition_version, DEFINITION_VERSION);
        assert_eq!(def.strategy_id, "ema_cross_v1");
        assert_eq!(def.inputs.len(), 2);
        assert_eq!(def.nodes.len(), 2);
        assert_eq!(def.actions.len(), 1);
        assert!(def.risk_overrides.max_position.is_some());

        // Re-serialize and re-parse — stable round-trip.
        let json2 = serde_json::to_string(&def).unwrap();
        let def2: StrategyDefinition = serde_json::from_str(&json2).unwrap();
        assert_eq!(def, def2);
    }

    /// Three documents copied out of the live `strategy_definitions` table: one
    /// at each version the column actually holds (9 rows are 1.0, 1 is 1.1, 4 are
    /// 1.2 — all 14 with `definition_version = '1.0'` in the *column*, which is
    /// why the version inside the JSON is the one anything must read).
    ///
    /// 1.4 adds a field to an exit rule. The proof that it is additive is that
    /// these come back out with the same keys and the same values, and that the
    /// new `unit` key appears nowhere: none of them has an `exits` block at all.
    #[test]
    fn the_definitions_in_the_live_table_round_trip_unchanged() {
        const STORED: [&str; 3] = [
            // ema_cross_v1, definition_json.definition_version = "1.0"
            r#"{"nodes": [{"id": "c1", "expr": "feature('ema_7') > feature('ema_21')", "type": "condition"}, {"id": "s1", "emit": "long", "type": "signal", "when": "c1"}], "inputs": [{"lane": "market.bars.1h", "instrument": "$bound_at_init"}, {"lane": "features.technical", "features": ["ema_7", "ema_21"], "instrument": "$bound_at_init"}], "actions": [{"type": "place_order", "order": {"side": "buy", "size": "0.01", "size_mode": "fixed"}, "on_signal": "long"}], "asset_class": "crypto_spot_cex", "strategy_id": "ema_cross_v1", "min_trust_tier": "centralized_exchange", "risk_overrides": {}, "definition_version": "1.0"}"#,
            // ai_model_smoke, "1.1" — the ModelForecast node
            r#"{"nodes": [{"id": "m1", "type": "model_forecast", "direction": "bullish", "model_ref": "btc_forecaster", "min_confidence": 0.6}, {"id": "s1", "emit": "entry", "type": "signal", "when": "m1"}], "inputs": [{"lane": "market.bars.1m", "instrument": "$bound_at_init"}], "actions": [{"type": "place_order", "order": {"side": "buy", "size": "0.01", "size_mode": "fixed"}, "on_signal": "entry"}], "asset_class": "crypto_spot_cex", "strategy_id": "ai_model_smoke", "min_trust_tier": "centralized_exchange", "risk_overrides": {}, "definition_version": "1.1"}"#,
            // model_signal_v1, "1.2" — note the parameter literally named `exit`,
            // which is what a strategy did before 1.3 gave exits a home.
            r#"{"nodes": [{"id": "n1", "expr": "feature('model_signal') > 0.5", "type": "condition"}, {"id": "s1", "emit": "long", "type": "signal", "when": "n1"}], "inputs": [{"lane": "market.bars.1h", "instrument": "$bound_at_init"}, {"lane": "features.inference", "instrument": "$bound_at_init"}], "actions": [{"type": "place_order", "order": {"side": "buy", "size": "0.01", "size_mode": "fixed"}, "on_signal": "long"}], "parameters": {"exit": {"type": "enum", "choices": ["trail", "fixed"], "default": "trail"}}, "asset_class": "crypto_spot_cex", "strategy_id": "model_signal_v1", "min_trust_tier": "centralized_exchange", "risk_overrides": {}, "definition_version": "1.2"}"#,
        ];

        for stored in STORED {
            let before: serde_json::Value = serde_json::from_str(stored).expect("stored JSON");
            let def: StrategyDefinition = serde_json::from_str(stored)
                .unwrap_or_else(|e| panic!("a stored definition must still load: {e}"));
            assert!(
                def.exits.is_none(),
                "no stored document has an exit policy; reading one in would be inventing it"
            );
            let after = serde_json::to_value(&def).expect("serializes");
            assert_eq!(
                before, after,
                "1.4 must not add, drop or alter a key in a document written before it"
            );
            assert!(
                !serde_json::to_string(&after).unwrap().contains("\"unit\""),
                "the 1.4 field must not appear in a document that never used it"
            );
        }
    }

    #[test]
    fn definition_version_is_1_0() {
        let json = r#"{
            "strategy_id": "s",
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "inputs": [],
            "nodes": [],
            "actions": []
        }"#;
        let def: StrategyDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(def.definition_version, "1.0");
    }
}
