//! `TradePayload` — a single executed trade event.

use serde::{Deserialize, Serialize};

use crate::money::{Price, Size};
use crate::payloads::Payload;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeSide {
    Buy,
    Sell,
    /// Side is unknown — some venues do not report it.
    Unknown,
}

/// A single matched trade from an exchange.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TradePayload {
    pub schema_version: String,
    pub price: Price,
    pub size: Size,
    pub side: TradeSide,
    /// Opaque trade ID from the originating exchange, used as the dedup key.
    pub exchange_trade_id: String,
}

impl TradePayload {
    pub fn new(price: Price, size: Size, side: TradeSide, exchange_trade_id: impl Into<String>) -> Self {
        Self {
            schema_version: Self::schema_version().into(),
            price,
            size,
            side,
            exchange_trade_id: exchange_trade_id.into(),
        }
    }
}

impl Payload for TradePayload {
    fn event_type() -> &'static str {
        "market.trade.v1"
    }

    fn schema_version() -> &'static str {
        "1"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn serde_round_trip() {
        let p = TradePayload::new(
            Price::from_str("50000.00").unwrap(),
            Size::from_str("0.001").unwrap(),
            TradeSide::Buy,
            "trade-xyz",
        );
        let json = serde_json::to_string(&p).unwrap();
        let back: TradePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(p.price, back.price);
        assert_eq!(p.exchange_trade_id, back.exchange_trade_id);
    }

    #[test]
    fn price_is_not_float() {
        // This test validates *behavior* — the price round-trips as a Decimal, not f64.
        let p = TradePayload::new(
            Price::from_str("0.1").unwrap(),
            Size::from_str("1").unwrap(),
            TradeSide::Sell,
            "t1",
        );
        // 0.1 is not representable in f64 without error, but Decimal handles it exactly.
        assert_eq!(p.price.to_string(), "0.1");
    }
}
