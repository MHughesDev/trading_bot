//! `DexQuotePayload` — 0x firm swap quote snapshot.

use serde::{Deserialize, Serialize};

use crate::money::{Price, Size};
use crate::payloads::Payload;

/// Firm DEX/AMM swap quote from an aggregator (e.g. 0x).
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
pub struct DexQuotePayload {
    /// Token being sold (e.g. `"WETH"`).
    pub sell_token: String,
    /// Token being bought (e.g. `"USDC"`).
    pub buy_token: String,
    /// Amount of sell token offered.
    pub sell_amount: Size,
    /// Amount of buy token returned (the "firm" quote).
    pub buy_amount: Size,
    /// Implied price: buy_amount / sell_amount.
    pub price: Price,
    /// Estimated gas cost in native units (e.g. ETH wei as a decimal string).
    pub estimated_gas: Option<String>,
}

impl DexQuotePayload {
    pub fn new(
        sell_token: impl Into<String>,
        buy_token: impl Into<String>,
        sell_amount: Size,
        buy_amount: Size,
        price: Price,
        estimated_gas: Option<String>,
    ) -> Self {
        Self {
            sell_token: sell_token.into(),
            buy_token: buy_token.into(),
            sell_amount,
            buy_amount,
            price,
            estimated_gas,
        }
    }
}

impl Payload for DexQuotePayload {
    fn event_type() -> &'static str {
        "dex.quote.v1"
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
    fn rkyv_round_trip() {
        let p = DexQuotePayload::new(
            "WETH",
            "USDC",
            Size::from_str("1.0").unwrap(),
            Size::from_str("2500.0").unwrap(),
            Price::from_str("2500.0").unwrap(),
            Some("120000".to_owned()),
        );
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&p).unwrap();
        // SAFETY: bytes were produced by rkyv::to_bytes immediately above.
        #[allow(unsafe_code)]
        let archived =
            unsafe { rkyv::access_unchecked::<rkyv::Archived<DexQuotePayload>>(bytes.as_ref()) };
        let back: DexQuotePayload = rkyv::deserialize::<_, rkyv::rancor::Error>(archived).unwrap();
        assert_eq!(p.price, back.price);
        assert_eq!(p.estimated_gas, back.estimated_gas);
    }
}
