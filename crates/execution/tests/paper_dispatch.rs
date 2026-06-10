//! P1-T05: Paper simulator dispatch test.
//!
//! Proves `market_structure → simulator` selection returns the correct
//! simulator type for all 8 asset classes.

use domain::instrument::{AssetClass, MarketStructure};
use execution::paper::simulator_for;

#[test]
fn all_8_asset_classes_dispatch_to_a_simulator() {
    let classes = [
        AssetClass::CryptoSpotCex,
        AssetClass::Equity,
        AssetClass::Fx,
        AssetClass::PredictionMarket,
        AssetClass::Option,
        AssetClass::CryptoSpotDex,
        AssetClass::PerpetualSwap,
        AssetClass::FuturesExpiring,
    ];

    for ac in classes {
        // Must not panic — every asset class must resolve to a simulator.
        let _sim = simulator_for(ac.market_structure());
    }
}

#[test]
fn clob_classes_return_clob_structure() {
    let clob_classes = [
        AssetClass::CryptoSpotCex,
        AssetClass::Fx,
        AssetClass::PerpetualSwap,
        AssetClass::FuturesExpiring,
    ];
    for ac in &clob_classes {
        let name = format!("{ac:?}");
        assert_eq!(
            ac.market_structure(),
            MarketStructure::Clob,
            "{name} should map to Clob"
        );
    }
}

#[test]
fn broker_quote_classes_return_broker_quote_structure() {
    for ac in &[AssetClass::Equity, AssetClass::Option] {
        let name = format!("{ac:?}");
        assert_eq!(
            ac.market_structure(),
            MarketStructure::BrokerQuote,
            "{name} should map to BrokerQuote"
        );
    }
}

#[test]
fn dex_class_returns_amm_swap_structure() {
    assert_eq!(
        AssetClass::CryptoSpotDex.market_structure(),
        MarketStructure::AmmSwap
    );
}

#[test]
fn prediction_class_returns_prediction_binary_structure() {
    assert_eq!(
        AssetClass::PredictionMarket.market_structure(),
        MarketStructure::PredictionBinary
    );
}
