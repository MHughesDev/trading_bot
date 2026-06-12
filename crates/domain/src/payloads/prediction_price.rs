//! `PredictionPricePayload` — Kalshi YES/NO binary market price in [0, 1].

use serde::{Deserialize, Serialize};

use crate::money::{AsDecimalBytes, Price};
use crate::payloads::Payload;

/// YES/NO binary prediction-market price snapshot.
///
/// `yes_price` is the probability/price of the YES outcome, in [0, 1].
/// `no_price` = 1 - `yes_price` (barring rounding).
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
pub struct PredictionPricePayload {
    /// YES outcome price in [0, 1] — stored as a `Price` decimal.
    #[rkyv(with = AsDecimalBytes)]
    pub yes_price: Price,
    /// NO outcome price in [0, 1].
    #[rkyv(with = AsDecimalBytes)]
    pub no_price: Price,
    /// Open interest / volume in the market (optional).
    #[rkyv(with = rkyv::with::Map<AsDecimalBytes>)]
    pub volume: Option<Price>,
}

impl PredictionPricePayload {
    pub fn new(yes_price: Price, no_price: Price, volume: Option<Price>) -> Self {
        Self {
            yes_price,
            no_price,
            volume,
        }
    }
}

impl Payload for PredictionPricePayload {
    fn event_type() -> &'static str {
        "prediction.price.v1"
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
    fn rkyv_round_trip_with_volume() {
        let p = PredictionPricePayload::new(
            Price::from_str("0.55").unwrap(),
            Price::from_str("0.45").unwrap(),
            Some(Price::from_str("10000.0").unwrap()),
        );
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&p).unwrap();
        // SAFETY: bytes were produced by rkyv::to_bytes immediately above.
        #[allow(unsafe_code)]
        let archived = unsafe {
            rkyv::access_unchecked::<rkyv::Archived<PredictionPricePayload>>(bytes.as_ref())
        };
        let back: PredictionPricePayload =
            rkyv::deserialize::<_, rkyv::rancor::Error>(archived).unwrap();
        assert_eq!(p.yes_price, back.yes_price);
        assert_eq!(p.volume, back.volume);
    }

    #[test]
    fn rkyv_round_trip_no_volume() {
        let p = PredictionPricePayload::new(
            Price::from_str("0.6").unwrap(),
            Price::from_str("0.4").unwrap(),
            None,
        );
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&p).unwrap();
        // SAFETY: bytes were produced by rkyv::to_bytes immediately above.
        #[allow(unsafe_code)]
        let archived = unsafe {
            rkyv::access_unchecked::<rkyv::Archived<PredictionPricePayload>>(bytes.as_ref())
        };
        let back: PredictionPricePayload =
            rkyv::deserialize::<_, rkyv::rancor::Error>(archived).unwrap();
        assert_eq!(p.yes_price, back.yes_price);
        assert!(back.volume.is_none());
    }
}
