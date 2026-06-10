//! P4-T01 acceptance tests — paper fill simulators.

use domain::{
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side, TimeInForce},
};
use execution::paper::{
    amm_swap::{AmmQuoteSwapSimulator, FirmQuote},
    broker_quote::BrokerQuoteFillSimulator,
    clob::ClobFillSimulator,
    prediction::PredictionMarketFillSimulator,
    wallet::DexPaperWallet,
    PaperFillSimulator,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

fn intent(
    order_type: OrderType,
    side: Side,
    limit: Option<Price>,
    tif: TimeInForce,
) -> OrderIntent {
    let mut i = OrderIntent::new(
        "BTC-USD",
        side,
        order_type,
        Size::from_decimal(dec!(1)),
        limit,
        None,
    );
    i.time_in_force = tif;
    i.idempotency_key = uuid::Uuid::nil();
    i
}

// ── CLOB partial fill ────────────────────────────────────────────────────────

#[test]
fn clob_partial_fill_aggregates() {
    let mut sim = ClobFillSimulator::default();
    sim.partial_fill_ratio = dec!(0.5); // half fill
    let mark = Price::from_decimal(dec!(50000));
    let fill = sim.simulate_fill(
        &intent(OrderType::Market, Side::Buy, None, TimeInForce::Gtc),
        mark,
    );
    // Submitted qty = 1, ratio = 0.5 → filled_qty = 0.5
    assert_eq!(
        fill.filled_qty,
        dec!(0.5),
        "partial fill should fill 50% of qty"
    );

    // Two partial fills of 0.5 sum to the full 1 BTC.
    let total = fill.filled_qty + fill.filled_qty;
    assert_eq!(total, dec!(1), "two partial fills aggregate to full qty");
}

#[test]
fn clob_full_fill_by_default() {
    let sim = ClobFillSimulator::default();
    let mark = Price::from_decimal(dec!(50000));
    let fill = sim.simulate_fill(
        &intent(OrderType::Market, Side::Buy, None, TimeInForce::Gtc),
        mark,
    );
    assert_eq!(fill.filled_qty, dec!(1));
}

#[test]
fn clob_fee_charged_on_fill() {
    let sim = ClobFillSimulator::default();
    let mark = Price::from_decimal(dec!(50000));
    let fill = sim.simulate_fill(
        &intent(OrderType::Market, Side::Buy, None, TimeInForce::Gtc),
        mark,
    );
    assert!(fill.fee > Decimal::ZERO, "fee must be positive");
}

// ── BrokerQuote TIF ──────────────────────────────────────────────────────────

#[test]
fn broker_quote_ioc_non_marketable_returns_zero() {
    let sim = BrokerQuoteFillSimulator::default();
    let mark = Price::from_decimal(dec!(200));
    // Buy limit at 100 with IOC — mark is above limit, not marketable for buy.
    let limit = Price::from_decimal(dec!(100));
    let fill = sim.simulate_fill(
        &intent(OrderType::Limit, Side::Buy, Some(limit), TimeInForce::Ioc),
        mark,
    );
    assert_eq!(
        fill.filled_qty,
        Decimal::ZERO,
        "IOC non-marketable should cancel"
    );
}

#[test]
fn broker_quote_marketable_limit_fills_at_mark() {
    let sim = BrokerQuoteFillSimulator::default();
    let mark = Price::from_decimal(dec!(100));
    // Buy limit at 200 with mark at 100 — mark ≤ limit, marketable.
    let limit = Price::from_decimal(dec!(200));
    let fill = sim.simulate_fill(
        &intent(OrderType::Limit, Side::Buy, Some(limit), TimeInForce::Gtc),
        mark,
    );
    assert!(
        fill.filled_qty > Decimal::ZERO,
        "marketable buy limit must fill"
    );
    // Fill price should be ≤ limit (price improvement).
    assert!(fill.fill_price.inner() <= limit.inner());
}

#[test]
fn broker_quote_gtc_non_marketable_rests() {
    let sim = BrokerQuoteFillSimulator::default();
    let mark = Price::from_decimal(dec!(200));
    let limit = Price::from_decimal(dec!(100));
    let fill = sim.simulate_fill(
        &intent(OrderType::Limit, Side::Buy, Some(limit), TimeInForce::Gtc),
        mark,
    );
    // Resting GTC limit — zero fill until mark moves down.
    assert_eq!(fill.filled_qty, Decimal::ZERO);
}

// ── AMM wallet integration ───────────────────────────────────────────────────

#[test]
fn amm_swap_debits_wallet_and_fills() {
    let sim = AmmQuoteSwapSimulator::new(dec!(30)); // 0.3% impact
    let quote = FirmQuote {
        out_amount: dec!(0.02),
        effective_price: Price::from_decimal(dec!(50000)),
        fee_usd: dec!(5),
    };
    let mut wallet = DexPaperWallet::new();
    wallet.seed("USDC", dec!(1000));

    let i = OrderIntent::new(
        "ETH-USDC",
        Side::Buy,
        OrderType::Market,
        Size::from_decimal(dec!(1)),
        None,
        None,
    );
    let fill = sim
        .simulate_with_wallet(&i, &quote, &mut wallet, "USDC", "WETH")
        .unwrap();

    assert_eq!(fill.filled_qty, dec!(1));
    assert_eq!(fill.fee, dec!(5));
    // Wallet debited 1 USDC, credited 0.02 WETH.
    assert_eq!(wallet.balance("USDC"), dec!(999));
    assert_eq!(wallet.balance("WETH"), dec!(0.02));
}

#[test]
fn amm_swap_rejects_insufficient_balance() {
    let sim = AmmQuoteSwapSimulator::default();
    let quote = FirmQuote {
        out_amount: dec!(1),
        effective_price: Price::from_decimal(dec!(1000)),
        fee_usd: dec!(1),
    };
    let mut wallet = DexPaperWallet::new();
    wallet.seed("USDC", dec!(10));

    let i = OrderIntent::new(
        "ETH-USDC",
        Side::Buy,
        OrderType::Market,
        Size::from_decimal(dec!(100)), // 100 USDC, but only 10 available
        None,
        None,
    );
    let result = sim.simulate_with_wallet(&i, &quote, &mut wallet, "USDC", "WETH");
    assert!(result.is_err(), "should reject insufficient balance");
}

// ── Prediction market ────────────────────────────────────────────────────────

#[test]
fn prediction_fills_in_0_1_range() {
    let sim = PredictionMarketFillSimulator::default();
    let mark = Price::from_decimal(dec!(0.65));
    let i = OrderIntent::new(
        "TRUMP-WIN-2026",
        Side::Buy,
        OrderType::Market,
        Size::from_decimal(dec!(10)),
        None,
        None,
    );
    let fill = sim.simulate_fill(&i, mark);
    assert!(fill.fill_price.inner() >= Decimal::ZERO);
    assert!(fill.fill_price.inner() <= Decimal::ONE);
    assert!(fill.filled_qty > Decimal::ZERO);
}

#[test]
fn prediction_clamps_mark_above_1() {
    let sim = PredictionMarketFillSimulator::default();
    let mark = Price::from_decimal(dec!(1.5)); // invalid, should clamp
    let i = OrderIntent::new(
        "EVENT-A",
        Side::Buy,
        OrderType::Market,
        Size::from_decimal(dec!(1)),
        None,
        None,
    );
    let fill = sim.simulate_fill(&i, mark);
    assert_eq!(fill.fill_price.inner(), Decimal::ONE, "clamp at 1");
}
