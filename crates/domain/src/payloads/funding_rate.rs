//! `FundingRatePayload` — perpetual-swap funding rate snapshot.

use serde::{Deserialize, Serialize};

use crate::money::Price;
use crate::payloads::Payload;

/// Funding rate for a perpetual-swap contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FundingRatePayload {
    pub schema_version: String,
    /// Funding rate as a fraction (e.g. `0.0001` = 0.01%).
    pub rate: Price,
    /// Next funding timestamp as Unix millis.
    pub next_funding_ms: Option<i64>,
}

impl FundingRatePayload {
    pub fn new(rate: Price, next_funding_ms: Option<i64>) -> Self {
        Self {
            schema_version: Self::schema_version().into(),
            rate,
            next_funding_ms,
        }
    }
}

impl Payload for FundingRatePayload {
    fn event_type() -> &'static str {
        "market.funding_rate.v1"
    }

    fn schema_version() -> &'static str {
        "1"
    }
}
