//! `FundingRatePayload` — perpetual-swap funding rate snapshot.

use serde::{Deserialize, Serialize};

use crate::money::Price;
use crate::payloads::Payload;

/// Funding rate for a perpetual-swap contract.
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
pub struct FundingRatePayload {
    /// Funding rate as a fraction (e.g. `0.0001` = 0.01%).
    pub rate: Price,
    /// Next funding timestamp as Unix millis.
    pub next_funding_ms: Option<i64>,
}

impl FundingRatePayload {
    pub fn new(rate: Price, next_funding_ms: Option<i64>) -> Self {
        Self {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn rkyv_round_trip() {
        let p =
            FundingRatePayload::new(Price::from_str("0.0001").unwrap(), Some(1_700_000_000_000));
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&p).unwrap();
        // SAFETY: bytes were produced by rkyv::to_bytes immediately above.
        #[allow(unsafe_code)]
        let archived =
            unsafe { rkyv::access_unchecked::<rkyv::Archived<FundingRatePayload>>(bytes.as_ref()) };
        let back: FundingRatePayload =
            rkyv::deserialize::<_, rkyv::rancor::Error>(archived).unwrap();
        assert_eq!(p.rate, back.rate);
        assert_eq!(p.next_funding_ms, back.next_funding_ms);
    }
}
