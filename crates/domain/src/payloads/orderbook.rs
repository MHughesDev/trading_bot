//! `OrderBookPayload` — L2 order-book snapshot or delta update.

use serde::{Deserialize, Serialize};

use crate::money::{Price, Size};
use crate::payloads::Payload;

/// Whether the message is a full snapshot or an incremental delta.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookUpdateKind {
    Snapshot,
    Delta,
}

/// A single price level in the order book.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BookLevel {
    pub price: Price,
    pub size: Size,
}

/// L2 order-book event (snapshot or delta).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderBookPayload {
    pub schema_version: String,
    pub kind: BookUpdateKind,
    pub bids: Vec<BookLevel>,
    pub asks: Vec<BookLevel>,
    /// Monotonically increasing sequence number from the exchange.
    pub sequence: u64,
    /// True when on-chain data has insufficient confirmations.
    pub is_tentative: bool,
}

impl OrderBookPayload {
    pub fn new_snapshot(bids: Vec<BookLevel>, asks: Vec<BookLevel>, sequence: u64) -> Self {
        Self {
            schema_version: Self::schema_version().into(),
            kind: BookUpdateKind::Snapshot,
            bids,
            asks,
            sequence,
            is_tentative: false,
        }
    }

    pub fn new_delta(bids: Vec<BookLevel>, asks: Vec<BookLevel>, sequence: u64) -> Self {
        Self {
            schema_version: Self::schema_version().into(),
            kind: BookUpdateKind::Delta,
            bids,
            asks,
            sequence,
            is_tentative: false,
        }
    }
}

impl Payload for OrderBookPayload {
    fn event_type() -> &'static str {
        "market.orderbook.l2.v1"
    }

    fn schema_version() -> &'static str {
        "1"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn level(price: &str, size: &str) -> BookLevel {
        BookLevel {
            price: Price::from_str(price).unwrap(),
            size: Size::from_str(size).unwrap(),
        }
    }

    #[test]
    fn serde_round_trip_snapshot() {
        let ob = OrderBookPayload::new_snapshot(
            vec![level("49900", "1.0"), level("49800", "2.0")],
            vec![level("50100", "0.5")],
            1234,
        );
        let json = serde_json::to_string(&ob).unwrap();
        let back: OrderBookPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(ob.sequence, back.sequence);
        assert_eq!(ob.bids, back.bids);
    }
}
