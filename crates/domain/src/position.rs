//! `Position` and `Balance` domain types.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::money::Price;
use crate::order::Side;

/// Current open position for one instrument on one account.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Position {
    pub account_id: String,
    pub instrument_id: String,
    /// Positive = long, negative = short.
    pub quantity: Decimal,
    /// Volume-weighted average entry price.
    pub average_entry_price: Price,
    /// Unrealized P&L at the most recent mark price.
    pub unrealized_pnl: Decimal,
    pub last_updated: DateTime<Utc>,
}

impl Position {
    pub fn is_flat(&self) -> bool {
        self.quantity == Decimal::ZERO
    }

    pub fn side(&self) -> Option<Side> {
        if self.quantity > Decimal::ZERO {
            Some(Side::Buy)
        } else if self.quantity < Decimal::ZERO {
            Some(Side::Sell)
        } else {
            None
        }
    }
}

/// Available cash / collateral in one currency on one account.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Balance {
    pub account_id: String,
    /// ISO 4217 currency code or crypto ticker (e.g. `"USD"`, `"USDT"`).
    pub currency: String,
    pub available: Decimal,
    /// Total including amounts locked in open orders.
    pub total: Decimal,
    pub last_updated: DateTime<Utc>,
}

/// Notional value of a position at a given mark price.
pub fn notional_value(position: &Position, mark_price: Price) -> Decimal {
    position.quantity * mark_price.inner()
}

/// P&L given a mark price.
pub fn unrealized_pnl(position: &Position, mark_price: Price) -> Decimal {
    (mark_price.inner() - position.average_entry_price.inner()) * position.quantity
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn flat_position_detected() {
        let pos = Position {
            account_id: "acc1".into(),
            instrument_id: "BTC-USDT".into(),
            quantity: Decimal::ZERO,
            average_entry_price: Price::from_str("50000").unwrap(),
            unrealized_pnl: Decimal::ZERO,
            last_updated: Utc::now(),
        };
        assert!(pos.is_flat());
        assert!(pos.side().is_none());
    }

    #[test]
    fn long_position_side() {
        let pos = Position {
            account_id: "acc1".into(),
            instrument_id: "BTC-USDT".into(),
            quantity: Decimal::from_str("0.5").unwrap(),
            average_entry_price: Price::from_str("50000").unwrap(),
            unrealized_pnl: Decimal::ZERO,
            last_updated: Utc::now(),
        };
        assert_eq!(pos.side(), Some(Side::Buy));
    }
}
