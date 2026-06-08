//! Order domain types.
//!
//! Every order that flows through the system — whether from a strategy or a
//! manual UI command — starts as an `OrderIntent` and transitions through
//! `OrderState` as the broker and risk gate process it.
//!
//! The `idempotency_key` on `OrderIntent` is **mandatory**: it ensures that a
//! redelivered NATS message or a reconnect does not submit the same order twice.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::money::{Price, Size};

/// Buy or sell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

/// Order type sent to the broker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderType {
    Market,
    Limit,
    StopLimit,
}

/// Order lifecycle states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderState {
    /// Received by the risk gate, awaiting evaluation.
    Accepted,
    /// Passed the risk gate and submitted to the broker.
    Submitted,
    /// Some quantity has been filled; order remains open.
    PartiallyFilled,
    /// Fully filled — terminal state.
    Filled,
    /// Cancelled by user or system — terminal state.
    Cancelled,
    /// Rejected by risk gate or broker — terminal state.
    Rejected,
}

/// A request from the UI or an external source (pre-risk-gate).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrderRequest {
    pub instrument_id: String,
    pub side: Side,
    pub order_type: OrderType,
    pub size: Size,
    pub limit_price: Option<Price>,
}

/// An intent to trade, produced after risk evaluation and ready to submit.
///
/// `idempotency_key` is **mandatory** — it is the NATS message ID or a
/// deterministic key derived from the strategy signal + sequence number.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrderIntent {
    /// Stable unique key for idempotent submission.  Redelivery of this message
    /// at the broker adapter is a no-op if the key has already been processed.
    pub idempotency_key: Uuid,
    pub instrument_id: String,
    pub side: Side,
    pub order_type: OrderType,
    pub size: Size,
    pub limit_price: Option<Price>,
    /// Originating strategy (None for manual orders).
    pub strategy_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl OrderIntent {
    pub fn new(
        instrument_id: impl Into<String>,
        side: Side,
        order_type: OrderType,
        size: Size,
        limit_price: Option<Price>,
        strategy_id: Option<String>,
    ) -> Self {
        Self {
            idempotency_key: Uuid::new_v4(),
            instrument_id: instrument_id.into(),
            side,
            order_type,
            size,
            limit_price,
            strategy_id,
            created_at: Utc::now(),
        }
    }
}

/// A single fill (partial or full execution at the broker).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fill {
    /// Matches the `OrderIntent.idempotency_key` — makes fill processing idempotent.
    pub idempotency_key: Uuid,
    /// Broker-assigned order ID.
    pub broker_order_id: String,
    pub instrument_id: String,
    pub side: Side,
    pub filled_size: Size,
    pub fill_price: Price,
    /// Commission charged by the broker, expressed in the quote currency.
    pub commission: Decimal,
    pub filled_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn order_intent_has_idempotency_key() {
        let intent = OrderIntent::new(
            "BTC-USDT",
            Side::Buy,
            OrderType::Market,
            Size::from_str("0.01").unwrap(),
            None,
            None,
        );
        // Key must be a valid non-nil UUID.
        assert!(!intent.idempotency_key.is_nil());
    }

    #[test]
    fn serde_round_trip() {
        let intent = OrderIntent::new(
            "AAPL",
            Side::Sell,
            OrderType::Limit,
            Size::from_str("10").unwrap(),
            Some(Price::from_str("175.50").unwrap()),
            Some("ema_cross_v1".into()),
        );
        let json = serde_json::to_string(&intent).unwrap();
        let back: OrderIntent = serde_json::from_str(&json).unwrap();
        assert_eq!(intent.idempotency_key, back.idempotency_key);
        assert_eq!(intent.instrument_id, back.instrument_id);
    }
}
