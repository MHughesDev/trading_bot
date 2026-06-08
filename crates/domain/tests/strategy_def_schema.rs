//! Adversarial test: the canonical strategy example from DATA-004 §3.1 deserializes
//! and re-serializes stably, and `definition_version` is `"1.0"`.

use domain::strategy_def::actions::ActionKind;
use domain::strategy_def::inputs::BOUND_AT_INIT;
use domain::strategy_def::nodes::NodeKind;
use domain::strategy_def::{StrategyDefinition, DEFINITION_VERSION};

const CANONICAL_EXAMPLE: &str = r#"{
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

#[test]
fn canonical_example_deserializes() {
    let def: StrategyDefinition =
        serde_json::from_str(CANONICAL_EXAMPLE).expect("canonical example must deserialize");

    assert_eq!(def.definition_version, DEFINITION_VERSION);
    assert_eq!(def.definition_version, "1.0");
    assert_eq!(def.strategy_id, "ema_cross_v1");
    assert_eq!(def.inputs.len(), 2);
    assert!(def.inputs[0].is_bound_at_init());
    assert_eq!(def.inputs[0].instrument, BOUND_AT_INIT);
    assert_eq!(def.inputs[1].features, vec!["ema_7", "ema_21"]);
    assert_eq!(def.nodes.len(), 2);
    assert_eq!(def.actions.len(), 1);
    assert!(def.risk_overrides.max_position.is_some());
}

#[test]
fn canonical_example_round_trips_stably() {
    let def: StrategyDefinition = serde_json::from_str(CANONICAL_EXAMPLE).unwrap();
    let json2 = serde_json::to_string(&def).unwrap();
    let def2: StrategyDefinition = serde_json::from_str(&json2).unwrap();
    assert_eq!(def, def2);
}

#[test]
fn node_types_parse_correctly() {
    let def: StrategyDefinition = serde_json::from_str(CANONICAL_EXAMPLE).unwrap();
    match &def.nodes[0].kind {
        NodeKind::Condition { expr } => {
            assert!(expr.contains("ema_7"));
        }
        other => panic!("expected Condition node, got {other:?}"),
    }
    match &def.nodes[1].kind {
        NodeKind::Signal { when, emit } => {
            assert_eq!(when, "n1");
            assert_eq!(emit, "long");
        }
        other => panic!("expected Signal node, got {other:?}"),
    }
}

#[test]
fn action_is_place_order() {
    let def: StrategyDefinition = serde_json::from_str(CANONICAL_EXAMPLE).unwrap();
    assert_eq!(def.actions[0].on_signal, "long");
    match &def.actions[0].kind {
        ActionKind::PlaceOrder { order } => {
            assert_eq!(order.size, "0.01");
        }
    }
}
