//! Adversarial test: replaying the same fill is a no-op.

use std::str::FromStr;

use chrono::Utc;
use domain::money::Price;
use execution::fills::{FillEvent, FillProcessor, FillResult};
use rust_decimal::Decimal;
use uuid::Uuid;

fn fill(id_key: Uuid, broker_id: &str, qty: &str, price: &str) -> FillEvent {
    FillEvent {
        idempotency_key: id_key,
        broker_order_id: broker_id.to_owned(),
        filled_qty: Decimal::from_str(qty).unwrap(),
        fill_price: Price::from_str(price).unwrap(),
        commission: Decimal::ZERO,
        filled_at: Utc::now(),
    }
}

#[test]
fn replayed_fill_is_noop() {
    let mut fp = FillProcessor::new();
    let key = Uuid::new_v4();
    let f = fill(key, "broker-order-1", "1.0", "50000");

    assert_eq!(fp.apply(&f), FillResult::Applied);
    assert_eq!(fp.apply(&f), FillResult::Duplicate);
    // Apply 10 more times — all duplicates.
    for _ in 0..10 {
        assert_eq!(fp.apply(&f), FillResult::Duplicate);
    }
}

#[test]
fn partial_fills_processed_once_each() {
    let mut fp = FillProcessor::new();
    let key = Uuid::new_v4();

    let partial1 = fill(key, "broker-1", "0.5", "50000");
    let partial2 = fill(key, "broker-1", "0.5", "50100"); // different price = different fill

    assert_eq!(fp.apply(&partial1), FillResult::Applied);
    assert_eq!(fp.apply(&partial2), FillResult::Applied);
    // Replay of each is a no-op.
    assert_eq!(fp.apply(&partial1), FillResult::Duplicate);
    assert_eq!(fp.apply(&partial2), FillResult::Duplicate);
}

#[test]
fn different_orders_have_independent_fill_dedup() {
    let mut fp = FillProcessor::new();

    let f1 = fill(Uuid::new_v4(), "broker-1", "1.0", "50000");
    let f2 = fill(Uuid::new_v4(), "broker-2", "1.0", "50000");

    assert_eq!(fp.apply(&f1), FillResult::Applied);
    assert_eq!(fp.apply(&f2), FillResult::Applied);
}
