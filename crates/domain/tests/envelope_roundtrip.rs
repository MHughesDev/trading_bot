//! Adversarial test: `EventEnvelope<T>` round-trips for every v1 payload type.

use chrono::Utc;
use domain::{
    EventEnvelope, TrustTier,
    payloads::{
        Payload,
        bar::{BarPayload, Timeframe},
        orderbook::{BookLevel, OrderBookPayload},
        quote::QuotePayload,
        trade::{TradeSide, TradePayload},
    },
    money::{Price, Size},
};
use uuid::Uuid;

fn make_envelope<T: Payload + Clone>(payload: T) -> EventEnvelope<T> {
    let now = Utc::now();
    EventEnvelope::new(
        Uuid::new_v4(),
        "market.test",
        "BTC-USDT",
        "coinbase",
        "test_source",
        TrustTier::CentralizedExchange,
        Some(now),
        now,
        now,
        now,
        1,
        payload,
    )
}

fn p(s: &str) -> Price { s.parse().unwrap() }
fn sz(s: &str) -> Size { s.parse().unwrap() }

#[test]
fn trade_envelope_round_trip() {
    let payload = TradePayload::new(p("50000.00"), sz("0.01"), TradeSide::Buy, "trade-123");
    let env = make_envelope(payload);
    let json = serde_json::to_string(&env).unwrap();
    let back: EventEnvelope<TradePayload> = serde_json::from_str(&json).unwrap();
    assert_eq!(env.event_id, back.event_id);
    assert_eq!(env.payload.price, back.payload.price);
    assert_eq!(env.payload.exchange_trade_id, back.payload.exchange_trade_id);
}

#[test]
fn quote_envelope_round_trip() {
    let payload = QuotePayload::new(p("49999.99"), sz("0.5"), p("50000.01"), sz("0.3"));
    let env = make_envelope(payload);
    let json = serde_json::to_string(&env).unwrap();
    let back: EventEnvelope<QuotePayload> = serde_json::from_str(&json).unwrap();
    assert_eq!(env.event_id, back.event_id);
    assert_eq!(env.payload.bid_price, back.payload.bid_price);
}

#[test]
fn orderbook_envelope_round_trip() {
    let payload = OrderBookPayload::new_snapshot(
        vec![BookLevel { price: p("49900"), size: sz("1.0") }],
        vec![BookLevel { price: p("50100"), size: sz("0.5") }],
        999,
    );
    let env = make_envelope(payload);
    let json = serde_json::to_string(&env).unwrap();
    let back: EventEnvelope<OrderBookPayload> = serde_json::from_str(&json).unwrap();
    assert_eq!(env.event_id, back.event_id);
    assert_eq!(env.payload.sequence, back.payload.sequence);
}

#[test]
fn bar_envelope_round_trip() {
    let payload = BarPayload::new(
        Timeframe::Minutes1,
        p("100"), p("110"), p("95"), p("105"),
        sz("500"),
        200,
    );
    let env = make_envelope(payload);
    let json = serde_json::to_string(&env).unwrap();
    let back: EventEnvelope<BarPayload> = serde_json::from_str(&json).unwrap();
    assert_eq!(env.event_id, back.event_id);
    assert_eq!(env.payload.revision, 0);
    assert_eq!(env.payload.open, back.payload.open);
}
