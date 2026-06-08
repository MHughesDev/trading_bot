//! `QuotePayload` — L1 best bid/ask snapshot.

use serde::{Deserialize, Serialize};

use crate::money::{Price, Size};
use crate::payloads::Payload;

/// Best bid/ask at a point in time (L1 quote).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuotePayload {
    pub schema_version: String,
    pub bid_price: Price,
    pub bid_size: Size,
    pub ask_price: Price,
    pub ask_size: Size,
}

impl QuotePayload {
    pub fn new(bid_price: Price, bid_size: Size, ask_price: Price, ask_size: Size) -> Self {
        Self {
            schema_version: Self::schema_version().into(),
            bid_price,
            bid_size,
            ask_price,
            ask_size,
        }
    }
}

impl Payload for QuotePayload {
    fn event_type() -> &'static str {
        "market.quote.v1"
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
        let q = QuotePayload::new(
            Price::from_str("49999.99").unwrap(),
            Size::from_str("0.5").unwrap(),
            Price::from_str("50000.01").unwrap(),
            Size::from_str("0.3").unwrap(),
        );
        let json = serde_json::to_string(&q).unwrap();
        let back: QuotePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(q, back);
    }
}
