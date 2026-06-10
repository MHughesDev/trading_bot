//! P3-T04 acceptance tests — default EMA discovery strategy seed.

use api::accounts::seed::default_ema_strategy;
use domain::data_type::DataType;
use strategy_runtime::kind::{infer_kind, StrategyKind};
use strategy_runtime::manifest::compile_manifest;

/// A freshly seeded account's strategy requires only market.ohlcv.
#[test]
fn seeded_strategy_requires_only_ohlcv() {
    let def = default_ema_strategy();
    let manifest = compile_manifest(&def);
    assert_eq!(
        manifest.required_lanes,
        vec![DataType::MarketOhlcv],
        "default EMA strategy is cross-asset: only market.ohlcv required"
    );
}

/// The seeded strategy kind is Discovery (no PlaceOrder action).
#[test]
fn seeded_strategy_kind_is_discovery() {
    let def = default_ema_strategy();
    assert_eq!(
        infer_kind(&def),
        StrategyKind::Discovery,
        "default EMA strategy has no execution block → Discovery"
    );
}

/// The seeded strategy has definition_version 1.0.
#[test]
fn seeded_strategy_version_is_1_0() {
    let def = default_ema_strategy();
    assert_eq!(def.definition_version, "1.0");
}

/// The seeded strategy strategy_id is non-empty.
#[test]
fn seeded_strategy_id_is_non_empty() {
    let def = default_ema_strategy();
    assert!(!def.strategy_id.trim().is_empty());
}

/// The seeded strategy passes the standard validator.
#[test]
fn seeded_strategy_passes_validator() {
    let def = default_ema_strategy();
    assert!(
        strategy_validator::validate(&def).is_ok(),
        "default EMA strategy must pass strategy_validator::validate"
    );
}
