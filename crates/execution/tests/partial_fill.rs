//! Adversarial test: partial fills aggregate correctly into the position.

use chrono::Utc;
use domain::{money::Price, order::Side, position::Position};
use execution::{fills::FillEvent, positions::apply_fill_to_position};
use rust_decimal::Decimal;
use std::str::FromStr;
use uuid::Uuid;

fn fill(qty: &str, price: &str) -> FillEvent {
    FillEvent {
        idempotency_key: Uuid::new_v4(),
        broker_order_id: "broker-1".to_owned(),
        filled_qty: Decimal::from_str(qty).unwrap(),
        fill_price: Price::from_str(price).unwrap(),
        commission: Decimal::ZERO,
        filled_at: Utc::now(),
    }
}

fn flat_position() -> Position {
    Position {
        account_id: "acc1".to_owned(),
        instrument_id: "BTC-USD".to_owned(),
        quantity: Decimal::ZERO,
        average_entry_price: Price::from_str("0").unwrap(),
        unrealized_pnl: Decimal::ZERO,
        last_updated: Utc::now(),
    }
}

#[test]
fn two_partial_fills_sum_to_full_position() {
    let mut pos = flat_position();

    // First partial: 0.5 @ 100
    apply_fill_to_position(&mut pos, &fill("0.5", "100"), Side::Buy);
    assert_eq!(pos.quantity, Decimal::from_str("0.5").unwrap());
    assert_eq!(
        pos.average_entry_price.inner(),
        Decimal::from_str("100").unwrap()
    );

    // Second partial: 0.5 @ 200
    apply_fill_to_position(&mut pos, &fill("0.5", "200"), Side::Buy);
    assert_eq!(pos.quantity, Decimal::from_str("1.0").unwrap());
    // VWAP = (0.5*100 + 0.5*200) / 1.0 = 150
    assert_eq!(
        pos.average_entry_price.inner(),
        Decimal::from_str("150").unwrap()
    );
}

#[test]
fn three_partial_fills_produce_correct_vwap() {
    let mut pos = flat_position();

    apply_fill_to_position(&mut pos, &fill("1", "100"), Side::Buy);
    apply_fill_to_position(&mut pos, &fill("1", "120"), Side::Buy);
    apply_fill_to_position(&mut pos, &fill("2", "130"), Side::Buy);

    // VWAP = (1*100 + 1*120 + 2*130) / 4 = 480 / 4 = 120
    assert_eq!(pos.quantity, Decimal::from_str("4").unwrap());
    assert_eq!(
        pos.average_entry_price.inner(),
        Decimal::from_str("120").unwrap()
    );
}

#[test]
fn sell_reduces_position_correctly() {
    let mut pos = flat_position();

    apply_fill_to_position(&mut pos, &fill("2", "100"), Side::Buy);
    apply_fill_to_position(&mut pos, &fill("1", "150"), Side::Sell);

    assert_eq!(pos.quantity, Decimal::from_str("1").unwrap());
    // Avg price unchanged while long.
    assert_eq!(
        pos.average_entry_price.inner(),
        Decimal::from_str("100").unwrap()
    );
}
