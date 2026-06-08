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
pub mod inputs;
pub mod nodes;
pub mod risk_overrides;

use serde::{Deserialize, Serialize};

use crate::trust::TrustTier;
use actions::Action;
use inputs::InputDeclaration;
use nodes::Node;
use risk_overrides::RiskOverrides;

/// The format version this module implements.  This string is frozen.
/// Future breaking changes introduce a new version string value, not an
/// in-place edit of the v1.0 schema.
pub const DEFINITION_VERSION: &str = "1.0";

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
