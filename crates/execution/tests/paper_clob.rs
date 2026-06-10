//! P1-T05: CLOB paper fill simulator integration tests.
//!
//! Proves a market buy fills at mark+slippage and a resting limit fills only
//! after the mark crosses.

use domain::money::{Price, Size};
use domain::order::{OrderIntent, OrderType, Side};
use execution::paper::clob::ClobFillSimulator;
use execution::paper::PaperFillSimulator;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

fn buy_market(instrument_id: &str, qty: Decimal) -> OrderIntent {
    OrderIntent::new(
        instrument_id,
        Side::Buy,
        OrderType::Market,
        Size::from_decimal(qty),
        None,
        None,
    )
}

fn buy_limit(instrument_id: &str, qty: Decimal, limit: Price) -> OrderIntent {
    OrderIntent::new(
        instrument_id,
        Side::Buy,
        OrderType::Limit,
        Size::from_decimal(qty),
        Some(limit),
        None,
    )
}

#[test]
fn market_buy_fills_at_mark_plus_slippage() {
    let sim = ClobFillSimulator::default();
    let mark = Price::from_decimal(dec!(50000));
    let intent = buy_market("BTC-USDT", dec!(0.5));
    let fill = sim.simulate_fill(&intent, mark);

    assert_eq!(fill.filled_qty, dec!(0.5), "market buy must fill full qty");
    assert!(
        fill.fill_price.inner() > mark.inner(),
        "buy fill price must exceed mark (spread + slippage)"
    );
}

#[test]
fn resting_limit_does_not_fill_when_mark_above_limit() {
    let sim = ClobFillSimulator::default();
    let limit = Price::from_decimal(dec!(49000));
    let mark_above = Price::from_decimal(dec!(50000));
    let intent = buy_limit("BTC-USDT", dec!(1), limit);

    let fill = sim.simulate_fill(&intent, mark_above);
    assert_eq!(
        fill.filled_qty,
        Decimal::ZERO,
        "must not fill when mark is above limit"
    );
}

#[test]
fn resting_limit_fills_when_mark_crosses_to_limit() {
    let sim = ClobFillSimulator::default();
    let limit = Price::from_decimal(dec!(49000));
    let mark_at = Price::from_decimal(dec!(49000));
    let intent = buy_limit("BTC-USDT", dec!(1), limit);

    let fill = sim.simulate_fill(&intent, mark_at);
    assert!(
        fill.filled_qty > Decimal::ZERO,
        "must fill when mark reaches limit"
    );
    assert_eq!(
        fill.fill_price, limit,
        "limit fill should be at the limit price"
    );
}

#[test]
fn resting_limit_fills_when_mark_drops_below_limit() {
    let sim = ClobFillSimulator::default();
    let limit = Price::from_decimal(dec!(49000));
    let mark_below = Price::from_decimal(dec!(48500));
    let intent = buy_limit("BTC-USDT", dec!(2), limit);

    let fill = sim.simulate_fill(&intent, mark_below);
    assert!(
        fill.filled_qty > Decimal::ZERO,
        "must fill when mark falls below limit"
    );
}
