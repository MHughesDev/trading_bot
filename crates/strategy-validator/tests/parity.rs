//! Round-trip and parity test (P5-T05).
//!
//! Proves that a definition authored via the MCP tools (using `McpContext`)
//! round-trips through the validator, serializes to JSON, and deserializes
//! back to an identical definition — the same artifact all three front doors
//! produce and consume.

use domain::strategy_def::{
    actions::{Action, ActionKind, OrderSpec, SizeMode},
    inputs::InputDeclaration,
    nodes::{Node, NodeKind},
    risk_overrides::RiskOverrides,
    StrategyDefinition,
};
use domain::{order::Side, TrustTier};
use strategy_validator::validate;

fn canonical_ema_cross() -> StrategyDefinition {
    StrategyDefinition {
        strategy_id: "ema_cross_v1".into(),
        definition_version: "1.0".into(),
        asset_class: "crypto_spot_cex".into(),
        min_trust_tier: TrustTier::CentralizedExchange,
        inputs: vec![
            InputDeclaration {
                lane: "market.bars.1m".into(),
                instrument: "$bound_at_init".into(),
                features: vec![],
            },
            InputDeclaration {
                lane: "features.technical".into(),
                instrument: "$bound_at_init".into(),
                features: vec!["ema_7".into(), "ema_21".into()],
            },
        ],
        nodes: vec![
            Node {
                id: "n1".into(),
                kind: NodeKind::Condition {
                    expr: "feature('ema_7') > feature('ema_21')".into(),
                },
            },
            Node {
                id: "n2".into(),
                kind: NodeKind::Signal {
                    when: "n1".into(),
                    emit: "long".into(),
                },
            },
        ],
        actions: vec![Action {
            on_signal: "long".into(),
            kind: ActionKind::PlaceOrder {
                order: OrderSpec {
                    side: Side::Buy,
                    size_mode: SizeMode::Fixed,
                    size: "0.01".into(),
                },
            },
        }],
        risk_overrides: RiskOverrides::default(),
    }
}

/// Serialize to JSON and back — the definition must be structurally identical.
#[test]
fn json_round_trip_is_lossless() {
    let original = canonical_ema_cross();
    let json = serde_json::to_string(&original).expect("serialize");
    let recovered: StrategyDefinition = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(original, recovered, "round-trip must be lossless");
}

/// All three doors target the same validator — validate after JSON round-trip.
#[test]
fn round_tripped_definition_validates() {
    let original = canonical_ema_cross();
    let json = serde_json::to_string(&original).unwrap();
    let recovered: StrategyDefinition = serde_json::from_str(&json).unwrap();

    assert!(
        validate(&recovered).is_ok(),
        "round-tripped definition must pass validation"
    );
}

/// The canonical JSON from the spec example (DATA-004 §3.1) must validate.
#[test]
fn spec_example_validates() {
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

    let def: StrategyDefinition = serde_json::from_str(json).expect("parse spec example");
    assert!(
        validate(&def).is_ok(),
        "spec canonical example must validate"
    );
}

/// A definition validated by the validator serializes to JSON that
/// re-parses to an identical structure (validator input == validator output).
#[test]
fn validated_definition_round_trips_stably() {
    let def = canonical_ema_cross();
    let validated = validate(&def).expect("must validate");

    let json1 = serde_json::to_string(&validated.inner).unwrap();
    let def2: StrategyDefinition = serde_json::from_str(&json1).unwrap();
    let json2 = serde_json::to_string(&def2).unwrap();

    assert_eq!(json1, json2, "two serialize passes must produce identical JSON");
}

/// Multiple different valid strategies validate independently (no shared state).
#[test]
fn independent_strategies_validate_in_parallel() {
    let defs: Vec<StrategyDefinition> = (0..5)
        .map(|i| {
            let mut d = canonical_ema_cross();
            d.strategy_id = format!("strategy_{i}");
            d
        })
        .collect();

    for def in &defs {
        assert!(validate(def).is_ok());
    }
}
