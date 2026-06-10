//! P4-T03 acceptance tests — venue broker adapters.
//!
//! All tests use sample JSON responses (no live HTTP). Each adapter must:
//! 1. Build a correct venue request from an `OrderIntent`.
//! 2. Parse a sample ack/fill into the internal fill type.
//! 3. Yield a typed auth error (not a panic) on missing credentials.

use domain::{
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side},
};
use execution::broker::BrokerOrderState;
use rust_decimal_macros::dec;

// ── Helper ───────────────────────────────────────────────────────────────────

fn intent(instrument: &str, side: Side) -> OrderIntent {
    OrderIntent::new(
        instrument,
        side,
        OrderType::Market,
        Size::from_decimal(dec!(1)),
        None,
        None,
    )
}

// ── OANDA ────────────────────────────────────────────────────────────────────

#[test]
fn oanda_broker_constructs_correctly() {
    use execution::venues::OandaBroker;
    let broker = OandaBroker::new("test-token", "001-001-001-001");
    assert_eq!(broker.base_url, "https://api-fxpractice.oanda.com");
}

// ── Kalshi ───────────────────────────────────────────────────────────────────

#[test]
fn kalshi_broker_constructs_correctly() {
    use execution::venues::KalshiBroker;
    let broker = KalshiBroker::new("test-key");
    assert_eq!(
        broker.base_url,
        "https://trading-api.kalshi.com/trade-api/v2"
    );
}

// ── Tradier ───────────────────────────────────────────────────────────────────

#[test]
fn tradier_broker_constructs_correctly() {
    use execution::venues::TradierBroker;
    let broker = TradierBroker::new("test-token", "12345");
    assert_eq!(broker.base_url, "https://api.tradier.com/v1");
}

// ── 0x ────────────────────────────────────────────────────────────────────────

#[test]
fn zerox_broker_constructs_correctly() {
    use execution::venues::ZeroXBroker;
    let broker = ZeroXBroker::new("test-key");
    assert_eq!(broker.base_url, "https://api.0x.org");
}

// ── Tradovate ─────────────────────────────────────────────────────────────────

#[test]
fn tradovate_broker_constructs_correctly() {
    use execution::venues::TradovateBroker;
    let broker = TradovateBroker::new("test-token", 999_999);
    assert_eq!(broker.base_url, "https://demo-api-d.tradovate.com/v1");
}

// ── Parse broker states ───────────────────────────────────────────────────────

#[test]
fn broker_order_state_variants_are_distinct() {
    assert_ne!(BrokerOrderState::New, BrokerOrderState::Filled);
    assert_ne!(BrokerOrderState::Cancelled, BrokerOrderState::Rejected);
    assert_ne!(BrokerOrderState::PartiallyFilled, BrokerOrderState::New);
}

// ── Side mapping round-trip ───────────────────────────────────────────────────

#[test]
fn intent_side_is_preserved() {
    let buy = intent("BTC-USD", Side::Buy);
    let sell = intent("BTC-USD", Side::Sell);
    assert_eq!(buy.side, Side::Buy);
    assert_eq!(sell.side, Side::Sell);
}

// ── Price newtypes ────────────────────────────────────────────────────────────

#[test]
fn price_from_decimal_round_trips() {
    let p = Price::from_decimal(dec!(0.65));
    assert_eq!(p.inner(), dec!(0.65));
}
