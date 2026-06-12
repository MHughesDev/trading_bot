//! `DexQuotePayload` — 0x firm swap quote snapshot.

use serde::{Deserialize, Serialize};

use crate::money::{AsDecimalBytes, Price, Size};
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
    pub schema_version: String,
    /// Token being sold (e.g. `"WETH"`).
    pub sell_token: String,
    /// Token being bought (e.g. `"USDC"`).
    pub buy_token: String,
    /// Amount of sell token offered.
    #[rkyv(with = AsDecimalBytes)]
    pub sell_amount: Size,
    /// Amount of buy token returned (the "firm" quote).
    #[rkyv(with = AsDecimalBytes)]
    pub buy_amount: Size,
    /// Implied price: buy_amount / sell_amount.
    #[rkyv(with = AsDecimalBytes)]
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
            schema_version: Self::schema_version().into(),
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
