//! P3-T08 acceptance tests — v1.5 builder nodes (backend semantics).

use std::collections::HashMap;

use domain::data_type::DataType;
use domain::strategy_def::{
    nodes::{Node, NodeKind},
    risk_overrides::RiskOverrides,
    StrategyDefinition,
};
use domain::TrustTier;
use strategy_runtime::manifest::compile_manifest;
use strategy_runtime::nodes::{evaluate_universe_pipeline, Universe, UniverseEntry};

fn entry(id: &str, ema7: f64) -> UniverseEntry {
    UniverseEntry {
        instrument_id: id.into(),
        features: HashMap::from([("ema_7".to_string(), ema7)]),
    }
}

fn twenty_instrument_universe() -> Universe {
    (1..=20)
        .map(|i| entry(&format!("INST-{i:02}"), i as f64))
        .collect()
}

fn pipeline_nodes() -> Vec<Node> {
    vec![
        Node {
            id: "ds".into(),
            kind: NodeKind::DataSource {
                data_type: "market.ohlcv".into(),
            },
        },
        Node {
            id: "rank".into(),
            kind: NodeKind::Rank {
                input: "ds".into(),
                feature: "ema_7".into(),
                ascending: false, // descending: highest ema_7 first
            },
        },
        Node {
            id: "top5".into(),
            kind: NodeKind::TakeTopN {
                input: "rank".into(),
                n: 5,
            },
        },
        Node {
            id: "surface".into(),
            kind: NodeKind::SurfaceAction {
                input: "top5".into(),
            },
        },
    ]
}

/// A pipeline graph surfaces exactly the top 5 instruments by ranked feature.
#[test]
fn pipeline_surfaces_top_5_by_feature() {
    let universe = twenty_instrument_universe();
    let nodes = pipeline_nodes();

    let surfaced = evaluate_universe_pipeline(&nodes, universe);
    assert_eq!(surfaced.len(), 5, "exactly 5 instruments surfaced");

    // ema_7 values are 1..=20; descending top 5 are INST-20 through INST-16.
    for expected in ["INST-20", "INST-19", "INST-18", "INST-17", "INST-16"] {
        assert!(
            surfaced.contains(&expected.to_string()),
            "{expected} must be in top 5"
        );
    }
}

/// The manifest for a pipeline strategy reflects the DataSource lane.
#[test]
fn pipeline_manifest_reflects_data_source_lane() {
    let def = StrategyDefinition {
        strategy_id: "pipeline_top5".into(),
        definition_version: "1.0".into(),
        asset_class: "crypto_spot_cex".into(),
        min_trust_tier: TrustTier::CentralizedExchange,
        inputs: vec![],
        nodes: pipeline_nodes(),
        actions: vec![],
        risk_overrides: RiskOverrides::default(),
    };
    let manifest = compile_manifest(&def);
    assert!(
        manifest.required_lanes.contains(&DataType::MarketOhlcv),
        "manifest must include market.ohlcv from DataSource node"
    );
}

/// Filter node removes instruments not meeting the predicate.
#[test]
fn filter_node_removes_non_matching_instruments() {
    let universe: Universe = vec![entry("A", 5.0), entry("B", 15.0), entry("C", 25.0)];
    let nodes = vec![
        Node {
            id: "ds".into(),
            kind: NodeKind::DataSource {
                data_type: "market.ohlcv".into(),
            },
        },
        Node {
            id: "filt".into(),
            kind: NodeKind::Filter {
                input: "ds".into(),
                expr: "feature('ema_7') > 10.0".into(),
            },
        },
        Node {
            id: "surface".into(),
            kind: NodeKind::SurfaceAction {
                input: "filt".into(),
            },
        },
    ];

    let surfaced = evaluate_universe_pipeline(&nodes, universe);
    assert_eq!(surfaced.len(), 2);
    assert!(surfaced.contains(&"B".to_string()));
    assert!(surfaced.contains(&"C".to_string()));
    assert!(!surfaced.contains(&"A".to_string()));
}

/// An empty universe always surfaces nothing.
#[test]
fn empty_universe_surfaces_nothing() {
    let surfaced = evaluate_universe_pipeline(&pipeline_nodes(), vec![]);
    assert!(surfaced.is_empty());
}

/// A graph with no SurfaceAction node returns an empty list.
#[test]
fn no_surface_action_returns_empty() {
    let universe = twenty_instrument_universe();
    let nodes = vec![Node {
        id: "ds".into(),
        kind: NodeKind::DataSource {
            data_type: "market.ohlcv".into(),
        },
    }];
    let surfaced = evaluate_universe_pipeline(&nodes, universe);
    assert!(surfaced.is_empty());
}

/// Rank ascending: lowest value first.
#[test]
fn rank_ascending_orders_lowest_first() {
    let universe: Universe = vec![entry("X", 10.0), entry("Y", 1.0), entry("Z", 5.0)];
    let nodes = vec![
        Node {
            id: "ds".into(),
            kind: NodeKind::DataSource {
                data_type: "market.ohlcv".into(),
            },
        },
        Node {
            id: "rank".into(),
            kind: NodeKind::Rank {
                input: "ds".into(),
                feature: "ema_7".into(),
                ascending: true,
            },
        },
        Node {
            id: "top1".into(),
            kind: NodeKind::TakeTopN {
                input: "rank".into(),
                n: 1,
            },
        },
        Node {
            id: "s".into(),
            kind: NodeKind::SurfaceAction {
                input: "top1".into(),
            },
        },
    ];
    let surfaced = evaluate_universe_pipeline(&nodes, universe);
    assert_eq!(surfaced, vec!["Y"]);
}
