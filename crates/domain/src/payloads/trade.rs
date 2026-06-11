//! `TradePayload` — a single executed trade event.

use serde::{Deserialize, Serialize};

use crate::money::{AsDecimalBytes, Price, Size};
use crate::payloads::Payload;

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[rkyv(derive(Debug, PartialEq))]
#[serde(rename_all = "snake_case")]
pub enum TradeSide {
    Buy,
    Sell,
    /// Side is unknown — some venues do not report it.
    Unknown,
}

/// A single matched trade from an exchange.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[rkyv(derive(Debug))]
pub struct TradePayload {
    pub schema_version: String,
    #[rkyv(with = AsDecimalBytes)]
    pub price: Price,
    #[rkyv(with = AsDecimalBytes)]
    pub size: Size,
    pub side: TradeSide,
    /// Opaque trade ID from the originating exchange, used as the dedup key.
    pub exchange_trade_id: String,
    /// xxh3-64 dedup key: hash of (timestamp_ns LE || price_raw LE || size_raw LE).
    /// 8 bytes, no heap allocation, replaces UUID v5 / SHA-1 dedup identity.
    pub dedup_key: u64,
}

impl TradePayload {
    pub fn new(
        price: Price,
        size: Size,
        side: TradeSide,
        exchange_trade_id: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: Self::schema_version().into(),
            price,
            size,
            side,
            exchange_trade_id: exchange_trade_id.into(),
            dedup_key: 0,
        }
    }

    /// Construct with an explicit xxh3 dedup key.
    pub fn with_dedup_key(
        price: Price,
        size: Size,
        side: TradeSide,
        exchange_trade_id: impl Into<String>,
        dedup_key: u64,
    ) -> Self {
        Self {
            schema_version: Self::schema_version().into(),
            price,
            size,
            side,
            exchange_trade_id: exchange_trade_id.into(),
            dedup_key,
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
        let p = TradePayload::new(
            Price::from_str("0.1").unwrap(),
            Size::from_str("1").unwrap(),
            TradeSide::Sell,
            "t1",
        );
        assert_eq!(p.price.to_string(), "0.1");
    }

    #[test]
    fn rkyv_round_trip() {
        let p = TradePayload::new(
            Price::from_str("50000.00").unwrap(),
            Size::from_str("0.001").unwrap(),
            TradeSide::Buy,
            "trade-rkyv",
        );
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&p).unwrap();
        // SAFETY: bytes were produced by rkyv::to_bytes immediately above.
        #[allow(unsafe_code)]
        let archived =
            unsafe { rkyv::access_unchecked::<rkyv::Archived<TradePayload>>(bytes.as_ref()) };
        let back: TradePayload = rkyv::deserialize::<_, rkyv::rancor::Error>(archived).unwrap();
        assert_eq!(p.price, back.price);
        assert_eq!(p.exchange_trade_id, back.exchange_trade_id);
    }
}
