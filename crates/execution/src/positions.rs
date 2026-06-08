//! Position and balance tracking.
//!
//! Positions are updated from fills using volume-weighted average price
//! arithmetic.  All calculations use `Decimal` — never `f64`.

use chrono::Utc;
use rust_decimal::Decimal;

use domain::{money::Price, order::Side, position::Position};

use crate::fills::FillEvent;

/// Update `position` in place from a fill.
///
/// For a buy fill: qty increases; VWAP is recomputed.
/// For a sell fill: qty decreases; avg price is unchanged while long.
pub fn apply_fill_to_position(position: &mut Position, fill: &FillEvent, side: Side) {
    let fill_qty = fill.filled_qty;
    let fill_price = fill.fill_price.inner();
    let now = Utc::now();

    match side {
        Side::Buy => {
            let new_qty = position.quantity + fill_qty;
            if new_qty.is_sign_positive() && !new_qty.is_zero() {
                let old_cost = position.quantity * position.average_entry_price.inner();
                let new_cost = fill_qty * fill_price;
                let vwap = (old_cost + new_cost) / new_qty;
                position.average_entry_price = Price::from_decimal(vwap);
            }
            position.quantity = new_qty;
        }
        Side::Sell => {
            position.quantity -= fill_qty;
            // Average entry price is kept while a net long remains.
            // When going short through zero, reset VWAP to fill price.
            if position.quantity.is_sign_negative() {
                position.average_entry_price = fill.fill_price;
            }
        }
    }

    position.unrealized_pnl = Decimal::ZERO; // mark will be recomputed externally
    position.last_updated = now;
}

/// Compute unrealized P&L given a mark price.
pub fn mark_pnl(position: &Position, mark_price: Price) -> Decimal {
    (mark_price.inner() - position.average_entry_price.inner()) * position.quantity
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use uuid::Uuid;

    fn pos(qty: &str, avg_price: &str) -> Position {
        Position {
            account_id: "acc1".to_owned(),
            instrument_id: "BTC-USD".to_owned(),
            quantity: Decimal::from_str(qty).unwrap(),
            average_entry_price: Price::from_str(avg_price).unwrap(),
            unrealized_pnl: Decimal::ZERO,
            last_updated: Utc::now(),
        }
    }

    fn fill(qty: &str, price: &str) -> FillEvent {
        crate::fills::FillEvent {
            idempotency_key: Uuid::new_v4(),
            broker_order_id: "b1".to_owned(),
            filled_qty: Decimal::from_str(qty).unwrap(),
            fill_price: Price::from_str(price).unwrap(),
            commission: Decimal::ZERO,
            filled_at: Utc::now(),
        }
    }

    #[test]
    fn buy_fill_increases_qty_and_updates_vwap() {
        let mut p = pos("1", "100");
        apply_fill_to_position(&mut p, &fill("1", "200"), Side::Buy);
        assert_eq!(p.quantity, Decimal::from_str("2").unwrap());
        assert_eq!(
            p.average_entry_price.inner(),
            Decimal::from_str("150").unwrap()
        );
    }

    #[test]
    fn sell_fill_decreases_qty() {
        let mut p = pos("2", "100");
        apply_fill_to_position(&mut p, &fill("1", "150"), Side::Sell);
        assert_eq!(p.quantity, Decimal::from_str("1").unwrap());
        // Avg price unchanged while long.
        assert_eq!(
            p.average_entry_price.inner(),
            Decimal::from_str("100").unwrap()
        );
    }

    #[test]
    fn partial_fills_aggregate_correctly() {
        let mut p = pos("0", "0");
        apply_fill_to_position(&mut p, &fill("0.5", "100"), Side::Buy);
        apply_fill_to_position(&mut p, &fill("0.5", "200"), Side::Buy);
        assert_eq!(p.quantity, Decimal::from_str("1").unwrap());
        assert_eq!(
            p.average_entry_price.inner(),
            Decimal::from_str("150").unwrap()
        );
    }
}
