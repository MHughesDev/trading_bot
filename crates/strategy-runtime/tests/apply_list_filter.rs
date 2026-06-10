//! P3-T03 acceptance tests — apply-list compatibility filtering.

use domain::data_type::DataType;
use domain::instrument::AssetClass;
use domain::strategy_def::{
    inputs::InputDeclaration,
    nodes::{Node, NodeKind},
    risk_overrides::RiskOverrides,
    StrategyDefinition,
};
use domain::TrustTier;
use strategy_runtime::compatibility::{is_compatible, InstrumentCapabilities};
use strategy_runtime::manifest::compile_manifest;

fn ohlcv_only_def(id: &str) -> StrategyDefinition {
    StrategyDefinition {
        strategy_id: id.into(),
        definition_version: "1.0".into(),
        asset_class: "crypto_spot_cex".into(),
        min_trust_tier: TrustTier::CentralizedExchange,
        inputs: vec![InputDeclaration {
            lane: "market.bars.1m".into(),
            instrument: "$bound_at_init".into(),
            features: vec!["ema_7".into(), "ema_21".into()],
        }],
        nodes: vec![Node {
            id: "n1".into(),
            kind: NodeKind::Condition {
                expr: "feature('ema_7') > feature('ema_21')".into(),
            },
        }],
        actions: vec![],
        risk_overrides: RiskOverrides::default(),
    }
}

fn funding_rate_def() -> StrategyDefinition {
    StrategyDefinition {
        strategy_id: "funding_rate_strat".into(),
        definition_version: "1.0".into(),
        asset_class: "perpetual_swap".into(),
        min_trust_tier: TrustTier::CentralizedExchange,
        inputs: vec![
            InputDeclaration {
                lane: "market.bars.1m".into(),
                instrument: "$bound_at_init".into(),
                features: vec![],
            },
            InputDeclaration {
                lane: "market.funding_rate".into(),
                instrument: "$bound_at_init".into(),
                features: vec![],
            },
        ],
        nodes: vec![],
        actions: vec![],
        risk_overrides: RiskOverrides::default(),
    }
}

/// A strategy requiring only market.ohlcv is compatible with every asset class.
#[test]
fn ema_strategy_compatible_with_every_asset_class() {
    let def = ohlcv_only_def("default_ema_7_21_discovery");
    let manifest = compile_manifest(&def);

    let all_classes = [
        AssetClass::CryptoSpotCex,
        AssetClass::Equity,
        AssetClass::Fx,
        AssetClass::PredictionMarket,
        AssetClass::Option,
        AssetClass::CryptoSpotDex,
        AssetClass::PerpetualSwap,
        AssetClass::FuturesExpiring,
    ];

    for ac in &all_classes {
        let caps = InstrumentCapabilities::from_asset_class(*ac);
        assert!(
            is_compatible(&manifest, &caps),
            "EMA strategy requiring only market.ohlcv must be compatible with {ac:?}"
        );
    }
}

/// A strategy requiring market.funding_rate is omitted for spot-equity instruments.
#[test]
fn funding_rate_strategy_incompatible_with_equity() {
    let def = funding_rate_def();
    let manifest = compile_manifest(&def);
    let equity_caps = InstrumentCapabilities::from_asset_class(AssetClass::Equity);
    assert!(
        !is_compatible(&manifest, &equity_caps),
        "funding_rate strategy must be incompatible with Equity (no market.funding_rate)"
    );
}

/// The same funding-rate strategy IS compatible with perpetual swaps.
#[test]
fn funding_rate_strategy_compatible_with_perpetual_swap() {
    let def = funding_rate_def();
    let manifest = compile_manifest(&def);
    let perp_caps = InstrumentCapabilities::from_asset_class(AssetClass::PerpetualSwap);
    assert!(
        is_compatible(&manifest, &perp_caps),
        "funding_rate strategy must be compatible with PerpetualSwap"
    );
}

/// All provided lanes for Equity include market.ohlcv.
#[test]
fn equity_provides_market_ohlcv() {
    use strategy_runtime::compatibility::default_provided_lanes;
    let lanes = default_provided_lanes(AssetClass::Equity);
    assert!(lanes.contains(&DataType::MarketOhlcv));
}

/// Perpetual swaps provide market.funding_rate.
#[test]
fn perpetual_swap_provides_funding_rate() {
    use strategy_runtime::compatibility::default_provided_lanes;
    let lanes = default_provided_lanes(AssetClass::PerpetualSwap);
    assert!(lanes.contains(&DataType::MarketFundingRate));
}
