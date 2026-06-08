//! Acceptance tests for P5-T01.
//!
//! Proves:
//! - A loosening `risk_overrides` is rejected with a structured error.
//! - A malformed condition expression is rejected with a structured error.
//! - A valid canonical definition is accepted.

use domain::strategy_def::{
    actions::{Action, ActionKind, OrderSpec, SizeMode},
    inputs::InputDeclaration,
    nodes::{Node, NodeKind},
    risk_overrides::RiskOverrides,
    StrategyDefinition,
};
use domain::{order::Side, TrustTier};
use rust_decimal::Decimal;
use strategy_validator::validate;

fn canonical() -> StrategyDefinition {
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

#[test]
fn valid_definition_validates() {
    assert!(
        validate(&canonical()).is_ok(),
        "canonical definition must validate"
    );
}

#[test]
fn rejects_loosening_max_position() {
    let mut def = canonical();
    def.risk_overrides.max_position = Some(Decimal::from(200)); // global default is 100
    let result = validate(&def);
    assert!(result.is_err(), "loosening max_position must be rejected");
    let errs = result.unwrap_err();
    assert!(
        errs.iter().any(|e| e.path == "risk_overrides.max_position"),
        "error must point at max_position, got: {errs:?}"
    );
}

#[test]
fn accepts_tightening_max_position() {
    let mut def = canonical();
    def.risk_overrides.max_position = Some(Decimal::from(50)); // tightens global 100
    assert!(
        validate(&def).is_ok(),
        "tightening max_position must be accepted"
    );
}

#[test]
fn rejects_loosening_rate_per_minute() {
    let mut def = canonical();
    def.risk_overrides.max_order_rate_per_minute = Some(120); // global default is 60
    let result = validate(&def);
    assert!(result.is_err());
    let errs = result.unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.path == "risk_overrides.max_order_rate_per_minute"),
        "error must point at max_order_rate_per_minute, got: {errs:?}"
    );
}

#[test]
fn rejects_malformed_expression() {
    let mut def = canonical();
    for node in def.nodes.iter_mut() {
        if let NodeKind::Condition { expr } = &mut node.kind {
            *expr = "feature( > garbage".to_owned();
        }
    }
    let result = validate(&def);
    assert!(result.is_err(), "malformed expression must be rejected");
    let errs = result.unwrap_err();
    assert!(
        errs.iter().any(|e| e.path.contains("expr")),
        "error must point at the expression, got: {errs:?}"
    );
}

#[test]
fn rejects_unknown_bar_field_in_expression() {
    let mut def = canonical();
    def.nodes[0].kind = NodeKind::Condition {
        expr: "bar('vwap') > 100.0".into(),
    };
    assert!(validate(&def).is_err());
}

#[test]
fn rejects_wrong_version() {
    let mut def = canonical();
    def.definition_version = "2.0".into();
    let result = validate(&def);
    assert!(result.is_err());
    let errs = result.unwrap_err();
    assert!(errs.iter().any(|e| e.path == "definition_version"));
}

#[test]
fn rejects_dangling_signal_reference() {
    let mut def = canonical();
    def.nodes.push(Node {
        id: "s2".into(),
        kind: NodeKind::Signal {
            when: "nonexistent".into(),
            emit: "exit".into(),
        },
    });
    assert!(validate(&def).is_err());
}

#[test]
fn all_errors_collected_in_one_pass() {
    let mut def = canonical();
    def.risk_overrides.max_position = Some(Decimal::from(999));
    def.risk_overrides.max_order_rate_per_minute = Some(999);
    def.nodes[0].kind = NodeKind::Condition {
        expr: "feature( broken".into(),
    };
    let result = validate(&def);
    assert!(result.is_err());
    let errs = result.unwrap_err();
    assert!(errs.len() >= 3, "expected ≥3 errors, got: {errs:?}");
}
