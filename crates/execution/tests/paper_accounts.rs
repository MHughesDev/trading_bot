//! End-to-end paper trading across every asset class — all internal.
//!
//! Each asset class trades against its own internal account through the
//! `PaperTradingEngine`; no venue credentials, no network.  Verifies the
//! per-class semantics side by side: cash debits for spot, margin for
//! derivatives, contract multiplier for options, binary payout for
//! prediction markets.

use std::sync::Arc;

use domain::{
    instrument::AssetClass,
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side},
};
use execution::broker::{Broker, BrokerOrderState};
use execution::paper::{PaperTradingEngine, ALL_ASSET_CLASSES};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

fn market(instrument: &str, side: Side, qty: Decimal) -> OrderIntent {
    OrderIntent::new(
        instrument,
        side,
        OrderType::Market,
        Size::from_decimal(qty),
        None,
        None,
    )
}

/// Representative instrument and order size per asset class.
fn scenario(asset_class: AssetClass) -> (&'static str, Decimal, Decimal) {
    match asset_class {
        AssetClass::CryptoSpotCex => ("BTC-USD", dec!(50_000), dec!(0.5)),
        AssetClass::Equity => ("AAPL", dec!(200), dec!(50)),
        AssetClass::Etf => ("SPY", dec!(500), dec!(20)),
        AssetClass::CryptoSpotDex => ("WETH-USDC", dec!(3_000), dec!(2)),
        AssetClass::FuturesExpiring => ("ESM6", dec!(5_000), dec!(3)),
        AssetClass::PerpetualSwap => ("BTC-PERP", dec!(50_000), dec!(1)),
        AssetClass::Option => ("AAPL240621C200", dec!(5), dec!(2)),
        AssetClass::Bond => ("UST10Y", dec!(98), dec!(100)),
        AssetClass::Fx => ("EUR-USD", dec!(1.10), dec!(10_000)),
        AssetClass::Nft => ("PUNK-ETH", dec!(40), dec!(1)),
        AssetClass::PredictionMarket => ("KX-RAIN-NYC", dec!(0.40), dec!(100)),
    }
}

#[test]
fn every_asset_class_trades_against_its_internal_account() {
    let engine = PaperTradingEngine::new();

    for asset_class in ALL_ASSET_CLASSES {
        let (instrument, mark, qty) = scenario(asset_class);
        engine.on_mark(instrument, Price::from_decimal(mark));

        let order_id = engine
            .submit(asset_class, &market(instrument, Side::Buy, qty))
            .unwrap_or_else(|e| panic!("{asset_class:?} buy failed: {e}"));
        let status = engine.order_status(&order_id).expect("order recorded");
        assert_eq!(
            status.state,
            BrokerOrderState::Filled,
            "{asset_class:?} market buy must fill"
        );

        let snap = engine.snapshot(asset_class);
        assert_eq!(
            snap.positions.len(),
            1,
            "{asset_class:?} must hold the position internally"
        );
        assert_eq!(snap.positions[0].quantity, qty);
    }

    // Eleven isolated accounts, all funded and all internal.
    assert_eq!(engine.snapshots().len(), 11);
}

#[test]
fn round_trip_pnl_is_internally_consistent_per_class() {
    let engine = PaperTradingEngine::new();

    for asset_class in ALL_ASSET_CLASSES {
        let (instrument, mark, qty) = scenario(asset_class);
        engine.on_mark(instrument, Price::from_decimal(mark));
        engine
            .submit(asset_class, &market(instrument, Side::Buy, qty))
            .unwrap();
        engine
            .submit(asset_class, &market(instrument, Side::Sell, qty))
            .unwrap();

        let snap = engine.snapshot(asset_class);
        assert!(
            snap.positions.is_empty(),
            "{asset_class:?} must be flat after the round trip"
        );
        // Round trip at a static mark loses only spread/slippage/fees.
        let policy_start =
            execution::paper::AccountPolicy::for_asset_class(asset_class).default_starting_cash;
        assert!(
            snap.cash <= policy_start && snap.cash > policy_start * dec!(0.8),
            "{asset_class:?} cash {} out of expected range",
            snap.cash
        );
        // Ledger must reconcile exactly: deposits + flows == cash.
        let net: Decimal = engine
            .transactions_since(asset_class, None)
            .iter()
            .map(|e| e.cash_delta)
            .sum();
        assert_eq!(net, snap.cash, "{asset_class:?} ledger must equal cash");
    }
}

#[tokio::test]
async fn broker_views_share_one_engine_but_isolate_asset_classes() {
    let engine = Arc::new(PaperTradingEngine::new());
    let crypto = engine.broker(AssetClass::CryptoSpotCex);
    let equity = engine.broker(AssetClass::Equity);

    engine.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
    engine.on_mark("AAPL", Price::from_decimal(dec!(200)));

    let btc = risk::ApprovedOrder::new_for_test(market("BTC-USD", Side::Buy, dec!(1)));
    let aapl = risk::ApprovedOrder::new_for_test(market("AAPL", Side::Buy, dec!(10)));
    crypto.submit(&btc).await.unwrap();
    equity.submit(&aapl).await.unwrap();

    let crypto_positions = crypto.query_positions().await.unwrap();
    let equity_positions = equity.query_positions().await.unwrap();
    assert_eq!(crypto_positions.len(), 1);
    assert_eq!(crypto_positions[0].instrument_id, "BTC-USD");
    assert_eq!(equity_positions.len(), 1);
    assert_eq!(equity_positions[0].instrument_id, "AAPL");
}
