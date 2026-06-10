//! `PredictionPricePayload` — Kalshi YES/NO binary market price in [0, 1].

use serde::{Deserialize, Serialize};

use crate::money::Price;
use crate::payloads::Payload;

/// YES/NO binary prediction-market price snapshot.
///
/// `yes_price` is the probability/price of the YES outcome, in [0, 1].
/// `no_price` = 1 - `yes_price` (barring rounding).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PredictionPricePayload {
    pub schema_version: String,
    /// YES outcome price in [0, 1] — stored as a `Price` decimal.
    pub yes_price: Price,
    /// NO outcome price in [0, 1].
    pub no_price: Price,
    /// Open interest / volume in the market (optional).
    pub volume: Option<Price>,
}

impl PredictionPricePayload {
    pub fn new(yes_price: Price, no_price: Price, volume: Option<Price>) -> Self {
        Self {
            schema_version: Self::schema_version().into(),
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
