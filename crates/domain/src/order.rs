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

/// How long an order remains active.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimeInForce {
    /// Good-Till-Cancelled — stays open across sessions.
    #[default]
    Gtc,
    /// Day — expires at end of trading session.
    Day,
    /// Immediate-or-Cancel — cancel any unfilled portion immediately.
    Ioc,
    /// Fill-or-Kill — fill entirely or cancel entirely.
    Fok,
}

/// Order type sent to the broker.
///
/// Until 2026-10-02 this had no `Stop` and no `TrailingStop`, and `StopLimit` had
/// nowhere to carry its trigger: `OrderIntent` had a `limit_price` and nothing
/// else, so every stop-limit in the platform was in fact a plain limit — the
/// paper engine rested it as one, and the Tradier adapter sent it to the venue as
/// one. `stop_price` and `trail` below are what make these types mean what they
/// say (Set U U-2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderType {
    Market,
    Limit,
    /// Becomes a market order once `stop_price` trades.
    Stop,
    /// Becomes a limit order at `limit_price` once `stop_price` trades.
    StopLimit,
    /// A stop that follows the best price since placement at `trail` behind it.
    TrailingStop,
}

impl OrderType {
    /// Its wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Market => "market",
            Self::Limit => "limit",
            Self::Stop => "stop",
            Self::StopLimit => "stop_limit",
            Self::TrailingStop => "trailing_stop",
        }
    }

    /// Waits for a trigger before it can fill.
    #[must_use]
    pub const fn is_triggered(self) -> bool {
        matches!(self, Self::Stop | Self::StopLimit | Self::TrailingStop)
    }
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
    #[serde(default)]
    pub time_in_force: TimeInForce,
    /// Trigger price for `Stop` and `StopLimit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_price: Option<Price>,
    /// Trail distance, in price units, for `TrailingStop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trail: Option<Price>,
    /// May only reduce an existing position — never open or reverse one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reduce_only: bool,
    /// A limit that would take liquidity is rejected rather than filled.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub post_only: bool,
    /// One-cancels-other group: when any order in the group fills, the venue
    /// cancels the rest. A bracket's stop and take-profit share one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oco_group: Option<String>,
}

/// Why an intent's fields do not describe an order of its type.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OrderShapeError {
    #[error("a {0} order needs a limit price")]
    MissingLimit(&'static str),
    #[error("a {0} order needs a stop (trigger) price")]
    MissingStop(&'static str),
    #[error("a trailing-stop order needs a trail distance")]
    MissingTrail,
    #[error("post-only applies to limit orders only, not {0}")]
    PostOnlyNotLimit(&'static str),
}

impl OrderIntent {
    /// Whether the fields this order type needs are present.
    ///
    /// # Errors
    /// The first missing field.
    pub fn check_shape(&self) -> Result<(), OrderShapeError> {
        let name = self.order_type.as_str();
        match self.order_type {
            OrderType::Market => {}
            OrderType::Limit => {
                if self.limit_price.is_none() {
                    return Err(OrderShapeError::MissingLimit(name));
                }
            }
            OrderType::Stop => {
                if self.stop_price.is_none() {
                    return Err(OrderShapeError::MissingStop(name));
                }
            }
            OrderType::StopLimit => {
                if self.stop_price.is_none() {
                    return Err(OrderShapeError::MissingStop(name));
                }
                if self.limit_price.is_none() {
                    return Err(OrderShapeError::MissingLimit(name));
                }
            }
            OrderType::TrailingStop => {
                if self.trail.is_none() {
                    return Err(OrderShapeError::MissingTrail);
                }
            }
        }
        if self.post_only && self.order_type != OrderType::Limit {
            return Err(OrderShapeError::PostOnlyNotLimit(name));
        }
        Ok(())
    }

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
            time_in_force: TimeInForce::default(),
            stop_price: None,
            trail: None,
            reduce_only: false,
            post_only: false,
            oco_group: None,
        }
    }

    #[must_use]
    pub fn with_stop(mut self, stop: Price) -> Self {
        self.stop_price = Some(stop);
        self
    }

    #[must_use]
    pub fn with_trail(mut self, trail: Price) -> Self {
        self.trail = Some(trail);
        self
    }

    #[must_use]
    pub fn reduce_only(mut self) -> Self {
        self.reduce_only = true;
        self
    }

    #[must_use]
    pub fn post_only(mut self) -> Self {
        self.post_only = true;
        self
    }

    #[must_use]
    pub fn in_oco(mut self, group: impl Into<String>) -> Self {
        self.oco_group = Some(group.into());
        self
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
    fn every_order_type_needs_its_own_fields() {
        let base =
            |t| OrderIntent::new("X", Side::Sell, t, Size::from_str("1").unwrap(), None, None);
        let p = || Price::from_str("100").unwrap();
        assert_eq!(base(OrderType::Market).check_shape(), Ok(()));
        assert!(base(OrderType::Limit).check_shape().is_err());
        assert!(base(OrderType::Stop).check_shape().is_err());
        assert_eq!(base(OrderType::Stop).with_stop(p()).check_shape(), Ok(()));
        // A stop-limit needs both, which is what it never had before.
        assert_eq!(
            base(OrderType::StopLimit).with_stop(p()).check_shape(),
            Err(OrderShapeError::MissingLimit("stop_limit"))
        );
        assert_eq!(
            base(OrderType::TrailingStop).check_shape(),
            Err(OrderShapeError::MissingTrail)
        );
        assert_eq!(
            base(OrderType::TrailingStop).with_trail(p()).check_shape(),
            Ok(())
        );
        let mut l = base(OrderType::Market).post_only();
        assert!(l.check_shape().is_err(), "post-only market is meaningless");
        l.order_type = OrderType::Limit;
        l.limit_price = Some(p());
        assert_eq!(l.check_shape(), Ok(()));
    }

    #[test]
    fn an_old_intent_without_the_new_fields_still_deserialises() {
        // Journals and NATS messages written before 2026-10-02 carry none of them.
        let old = r#"{"idempotency_key":"00000000-0000-0000-0000-000000000000","instrument_id":"X","side":"buy",
            "order_type":"market","size":"1","limit_price":null,"strategy_id":null,"created_at":"2026-01-01T00:00:00Z"}"#;
        let i: OrderIntent = serde_json::from_str(old).unwrap();
        assert!(!i.reduce_only && !i.post_only && i.stop_price.is_none() && i.oco_group.is_none());
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
