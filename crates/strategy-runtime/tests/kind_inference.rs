//! P3-T01 acceptance tests — strategy kind inference.

use domain::strategy_def::{
    actions::{Action, ActionKind, OrderSpec, SizeMode},
    inputs::InputDeclaration,
    nodes::{Node, NodeKind},
    risk_overrides::RiskOverrides,
    StrategyDefinition,
};
use domain::{order::Side, TrustTier};
use strategy_runtime::kind::{infer_kind, StrategyKind};

fn base_def() -> StrategyDefinition {
    StrategyDefinition {
        strategy_id: "test".into(),
        definition_version: "1.0".into(),
        asset_class: "crypto_spot_cex".into(),
        min_trust_tier: TrustTier::CentralizedExchange,
        inputs: vec![InputDeclaration {
            lane: "market.bars.1m".into(),
            instrument: "$bound_at_init".into(),
            features: vec![],
        }],
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
        actions: vec![],
        risk_overrides: RiskOverrides::default(),
    }
}

fn place_order_action() -> Action {
    Action {
        on_signal: "long".into(),
        kind: ActionKind::PlaceOrder {
            order: OrderSpec {
                side: Side::Buy,
                size_mode: SizeMode::Fixed,
                size: "0.01".into(),
            },
        },
    }
}

/// A strategy with a PlaceOrder execution block → Execution.
#[test]
fn execution_block_infers_execution() {
    let mut def = base_def();
    def.actions.push(place_order_action());
    assert_eq!(infer_kind(&def), StrategyKind::Execution);
}

/// A strategy with no actions → Discovery.
#[test]
fn no_actions_infers_discovery() {
    let def = base_def();
    assert_eq!(infer_kind(&def), StrategyKind::Discovery);
}

/// A strategy with only a SurfaceAction node and no PlaceOrder → Discovery.
#[test]
fn surface_action_node_without_place_order_is_discovery() {
    let mut def = base_def();
    def.nodes.push(Node {
        id: "n3".into(),
        kind: NodeKind::SurfaceAction { input: "n2".into() },
    });
    // Still no PlaceOrder action.
    assert_eq!(infer_kind(&def), StrategyKind::Discovery);
}

/// A stored `strategy_type` JSON field (legacy) does not override inferred kind.
/// We prove this by deserializing JSON that includes a `strategy_type` key and
/// confirming `infer_kind` still returns the correct value.
#[test]
fn stored_strategy_type_field_does_not_override_inferred_kind() {
    // Inject a spurious "strategy_type" field that disagrees with inferred kind.
    let json = r#"{
        "strategy_id": "test",
        "definition_version": "1.0",
        "asset_class": "crypto_spot_cex",
        "strategy_type": "execution",
        "inputs": [{ "lane": "market.bars.1m", "instrument": "$bound_at_init" }],
        "nodes": [],
        "actions": []
    }"#;
    // The StrategyDefinition format does not declare a `strategy_type` field, so
    // serde ignores it (unknown fields are allowed by default).
    let def: StrategyDefinition =
        serde_json::from_str(json).expect("definition with unknown strategy_type must parse");
    // No PlaceOrder → Discovery regardless of the stored `strategy_type` value.
    assert_eq!(infer_kind(&def), StrategyKind::Discovery);
}
