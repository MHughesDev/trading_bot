//! Round-trip tests for `EventEnvelope` with rkyv-encoded payloads.

use domain::{
    intern_instrument, intern_source, intern_venue,
    money::{Price, Size},
    payloads::{
        bar::{BarPayload, Timeframe},
        trade::{TradePayload, TradeSide},
    },
    EventEnvelope,
};

fn p(s: &str) -> Price {
    s.parse().unwrap()
}
fn sz(s: &str) -> Size {
    s.parse().unwrap()
}

fn make_trade_envelope(seq: u64) -> EventEnvelope {
    let payload = TradePayload::new(p("50000.00"), sz("0.01"), TradeSide::Buy, "trade-123");
    let payload_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&payload)
        .unwrap()
        .into_vec();
    EventEnvelope::new(
        intern_instrument("BTC-USDT"),
        intern_venue("coinbase"),
        intern_source("test_source"),
        seq,
        1_700_000_000_000_000_000,
        payload_bytes,
    )
}

fn make_bar_envelope(seq: u64) -> EventEnvelope {
    let payload = BarPayload::new(
        Timeframe::Minutes1,
        p("100"),
        p("110"),
        p("95"),
        p("105"),
        sz("500"),
        200,
    );
    let payload_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&payload)
        .unwrap()
        .into_vec();
    EventEnvelope::new(
        intern_instrument("BTC-USDT"),
        intern_venue("coinbase"),
        intern_source("test_source"),
        seq,
        1_700_000_000_000_000_000,
        payload_bytes,
    )
}

#[test]
fn trade_envelope_round_trip() {
    let env = make_trade_envelope(1);
    let json = serde_json::to_string(&env).unwrap();
    let back: EventEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(env.instrument_id, back.instrument_id);
    assert_eq!(env.sequence, back.sequence);
    assert_eq!(env.payload, back.payload);

    let trade = env.decode_payload::<TradePayload>().unwrap();
    assert_eq!(trade.price.to_string(), "50000.00");
    assert_eq!(trade.exchange_trade_id, "trade-123");
}

#[test]
fn bar_envelope_round_trip() {
    let env = make_bar_envelope(2);
    let json = serde_json::to_string(&env).unwrap();
    let back: EventEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(env.instrument_id, back.instrument_id);
    assert_eq!(env.payload, back.payload);

    let bar = env.decode_payload::<BarPayload>().unwrap();
    assert_eq!(bar.revision, 0);
    assert_eq!(bar.open.to_string(), "100");
}

#[test]
fn rkyv_binary_round_trip() {
    let env = make_trade_envelope(3);
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&env).unwrap();
    // SAFETY: bytes were produced by rkyv::to_bytes immediately above.
    #[allow(unsafe_code)]
    let archived =
        unsafe { rkyv::access_unchecked::<rkyv::Archived<EventEnvelope>>(bytes.as_ref()) };
    let back: EventEnvelope = rkyv::deserialize::<_, rkyv::rancor::Error>(archived).unwrap();
    assert_eq!(env.sequence, back.sequence);
    assert_eq!(env.timestamp_ns, back.timestamp_ns);
}
