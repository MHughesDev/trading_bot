//! P3-T02 acceptance tests — capability manifest compiler.

use domain::data_type::DataType;
use domain::strategy_def::{
    actions::{Action, ActionKind, OrderSpec, SizeMode},
    inputs::InputDeclaration,
    nodes::{Node, NodeKind},
    risk_overrides::RiskOverrides,
    StrategyDefinition,
};
use domain::{order::Side, TrustTier};
use strategy_runtime::kind::StrategyKind;
use strategy_runtime::manifest::{compile_manifest, EvaluationTrigger};

/// Default 7/21 EMA discovery strategy.
fn ema_discovery_def() -> StrategyDefinition {
    StrategyDefinition {
        strategy_id: "default_ema_7_21_discovery".into(),
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
                    emit: "bullish_cross".into(),
                },
            },
        ],
        actions: vec![],
        risk_overrides: RiskOverrides::default(),
    }
}

/// Default EMA strategy compiles to the expected manifest values.
#[test]
fn default_ema_manifest_required_lanes() {
    let manifest = compile_manifest(&ema_discovery_def());
    assert_eq!(
        manifest.required_lanes,
        vec![DataType::MarketOhlcv],
        "default EMA strategy requires only market.ohlcv"
    );
}

#[test]
fn default_ema_manifest_trigger_is_bar_close() {
    let manifest = compile_manifest(&ema_discovery_def());
    assert_eq!(manifest.evaluation_trigger, EvaluationTrigger::BarClose);
}

#[test]
fn default_ema_manifest_kind_is_discovery() {
    let manifest = compile_manifest(&ema_discovery_def());
    assert_eq!(manifest.strategy_kind, StrategyKind::Discovery);
}

#[test]
fn default_ema_manifest_required_features() {
    let manifest = compile_manifest(&ema_discovery_def());
    let mut features = manifest.required_features.clone();
    features.sort();
    assert_eq!(features, vec!["ema_21", "ema_7"]);
}

/// A strategy with a PlaceOrder action compiles to Execution kind.
#[test]
fn execution_strategy_manifest_kind_is_execution() {
    let mut def = ema_discovery_def();
    def.nodes.push(Node {
        id: "n3".into(),
        kind: NodeKind::Signal {
            when: "n1".into(),
            emit: "long".into(),
        },
    });
    def.actions.push(Action {
        on_signal: "long".into(),
        kind: ActionKind::PlaceOrder {
            order: OrderSpec {
                side: Side::Buy,
                size_mode: SizeMode::Fixed,
                size: "0.01".into(),
            },
        },
    });
    let manifest = compile_manifest(&def);
    assert_eq!(manifest.strategy_kind, StrategyKind::Execution);
}

/// A v1.5 DataSource node contributes its data_type to required_lanes.
#[test]
fn data_source_node_contributes_required_lane() {
    let def = StrategyDefinition {
        strategy_id: "pipeline_test".into(),
        definition_version: "1.0".into(),
        asset_class: "crypto_spot_cex".into(),
        min_trust_tier: TrustTier::CentralizedExchange,
        inputs: vec![],
        nodes: vec![Node {
            id: "ds1".into(),
            kind: NodeKind::DataSource {
                data_type: "market.ohlcv".into(),
            },
        }],
        actions: vec![],
        risk_overrides: RiskOverrides::default(),
    };
    let manifest = compile_manifest(&def);
    assert!(manifest.required_lanes.contains(&DataType::MarketOhlcv));
}

/// Deduplication: `market.bars.1m` input and a `DataSource { market.ohlcv }` node
/// both map to `MarketOhlcv`; the manifest must contain it exactly once.
#[test]
fn duplicate_lanes_are_deduplicated() {
    let def = StrategyDefinition {
        strategy_id: "dedup_test".into(),
        definition_version: "1.0".into(),
        asset_class: "crypto_spot_cex".into(),
        min_trust_tier: TrustTier::CentralizedExchange,
        inputs: vec![InputDeclaration {
            lane: "market.bars.1m".into(),
            instrument: "$bound_at_init".into(),
            features: vec![],
        }],
        nodes: vec![Node {
            id: "ds1".into(),
            kind: NodeKind::DataSource {
                data_type: "market.ohlcv".into(),
            },
        }],
        actions: vec![],
        risk_overrides: RiskOverrides::default(),
    };
    let manifest = compile_manifest(&def);
    assert_eq!(manifest.required_lanes.len(), 1);
    assert_eq!(manifest.required_lanes[0], DataType::MarketOhlcv);
}
