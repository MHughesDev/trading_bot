//! `BarPayload` — OHLCV candlestick data.
//!
//! `revision` defaults to `0` for the first publish.  A late-arriving event
//! triggers a **new** `EventEnvelope` on `market.bars.1m.revised` with
//! `revision = 1` (or higher for subsequent corrections).  The original bar is
//! never mutated in place.

use serde::{Deserialize, Serialize};

use crate::money::{AsDecimalBytes, Price, Size};
use crate::payloads::Payload;

/// Bar interval / timeframe.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize,
    rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[rkyv(derive(Debug, PartialEq))]
#[serde(rename_all = "snake_case")]
pub enum Timeframe {
    Seconds1,
    Minutes1,
    Minutes5,
    Minutes15,
    Hours1,
    Hours4,
    Daily,
}

/// OHLCV candlestick.
#[derive(
    Clone, Debug, PartialEq, Serialize, Deserialize,
    rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[rkyv(derive(Debug))]
pub struct BarPayload {
    pub schema_version: String,
    pub timeframe: Timeframe,
    #[rkyv(with = AsDecimalBytes)]
    pub open: Price,
    #[rkyv(with = AsDecimalBytes)]
    pub high: Price,
    #[rkyv(with = AsDecimalBytes)]
    pub low: Price,
    #[rkyv(with = AsDecimalBytes)]
    pub close: Price,
    #[rkyv(with = AsDecimalBytes)]
    pub volume: Size,
    pub trade_count: u64,
    /// `0` for the initial publish; incremented on each late-data revision.
    #[serde(default)]
    pub revision: u32,
}

impl BarPayload {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        timeframe: Timeframe,
        open: Price,
        high: Price,
        low: Price,
        close: Price,
        volume: Size,
        trade_count: u64,
    ) -> Self {
        Self {
            schema_version: Self::schema_version().into(),
            timeframe,
            open,
            high,
            low,
            close,
            volume,
            trade_count,
            revision: 0,
        }
    }
}

impl Payload for BarPayload {
    fn event_type() -> &'static str {
        "market.bar.v1"
    }

    fn schema_version() -> &'static str {
        "1"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn p(s: &str) -> Price {
        Price::from_str(s).unwrap()
    }
    fn sz(s: &str) -> Size {
        Size::from_str(s).unwrap()
    }

    #[test]
    fn revision_defaults_to_zero() {
        let bar = BarPayload::new(
            Timeframe::Minutes1,
            p("100"),
            p("110"),
            p("95"),
            p("105"),
            sz("500"),
            200,
        );
        assert_eq!(bar.revision, 0);
    }

    #[test]
    fn ohlcv_are_price_and_size_types() {
        let bar = BarPayload::new(
            Timeframe::Minutes1,
            p("100"),
            p("110"),
            p("95"),
            p("105"),
            sz("500"),
            200,
        );
        let _: Price = bar.open;
        let _: Price = bar.close;
        let _: Size = bar.volume;
    }

    #[test]
    fn serde_round_trip() {
        let bar = BarPayload::new(
            Timeframe::Minutes1,
            p("100"),
            p("110"),
            p("95"),
            p("105"),
            sz("500"),
            200,
        );
        let json = serde_json::to_string(&bar).unwrap();
        let back: BarPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(bar.open, back.open);
        assert_eq!(bar.revision, back.revision);
    }

    #[test]
    fn rkyv_round_trip() {
        let bar = BarPayload::new(
            Timeframe::Minutes1,
            p("100"),
            p("110"),
            p("95"),
            p("105"),
            sz("500"),
            200,
        );
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&bar).unwrap();
        // SAFETY: bytes were produced by rkyv::to_bytes immediately above.
        #[allow(unsafe_code)]
        let archived = unsafe {
            rkyv::access_unchecked::<rkyv::Archived<BarPayload>>(bytes.as_ref())
        };
        let back: BarPayload = rkyv::deserialize::<_, rkyv::rancor::Error>(archived).unwrap();
        assert_eq!(bar.open, back.open);
        assert_eq!(bar.trade_count, back.trade_count);
    }
}
