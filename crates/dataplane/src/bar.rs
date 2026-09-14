//! The canonical minute bar contract (SPEC §0.1, §1.2; INV-01, INV-05, INV-06).
//!
//! Four timestamps, written at ingest and never derived later:
//! `event_time` (bar OPEN, UTC), `venue_ts` (exchange/chain clock), `ingest_time`
//! (bytes received) and `knowledge_time` (queryable by a strategy).

use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::identity::{InstrumentKey, VenueKey};
use crate::quality::QualityFlags;

/// Price scale of the canonical bar: `DECIMAL(38,18)` (ClickHouse `Decimal128(18)`).
pub const PRICE_SCALE: u32 = 18;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanonicalBar {
    pub instrument_id: InstrumentKey,
    pub venue_id: VenueKey,
    /// Bar length in seconds (60 for `bar_1m`).
    pub period_secs: u32,
    /// The bar OPEN. UTC, nanosecond precision.
    pub event_time: DateTime<Utc>,
    pub venue_ts: Option<DateTime<Utc>>,
    pub ingest_time: DateTime<Utc>,
    pub knowledge_time: DateTime<Utc>,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
    pub trade_count: Option<u32>,
    pub vwap: Option<Decimal>,
    pub bid_close: Option<Decimal>,
    pub ask_close: Option<Decimal>,
    pub quality_flags: QualityFlags,
    pub revision_seq: u32,
    pub source_id: i32,
}

impl CanonicalBar {
    #[must_use]
    pub fn close_time(&self) -> DateTime<Utc> {
        self.event_time + Duration::seconds(i64::from(self.period_secs))
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BarError {
    #[error("event_time {0} is not aligned to a bar open for period {1}s (bar timestamps are the OPEN)")]
    NotOpenAligned(DateTime<Utc>, u32),
    #[error("knowledge_time {knowledge} precedes the bar close {close}: a bar cannot be known before it ends")]
    KnownBeforeClose { knowledge: DateTime<Utc>, close: DateTime<Utc> },
    #[error("knowledge_time {knowledge} precedes ingest_time {ingest} without BACKFILLED_KNOWLEDGE_TIME")]
    KnownBeforeIngest { knowledge: DateTime<Utc>, ingest: DateTime<Utc> },
    #[error("OHLC inconsistent: low {low} / high {high} do not bound open/close")]
    OhlcInconsistent { low: String, high: String },
    #[error("negative volume")]
    NegativeVolume,
    #[error("price has more than {PRICE_SCALE} fractional digits: {0}")]
    PrecisionLoss(String),
    #[error("zero period")]
    ZeroPeriod,
}

/// Ingest validator: the single enforced bar convention.
///
/// # Errors
/// Returns the first violation found.
pub fn validate(bar: &CanonicalBar) -> Result<(), BarError> {
    if bar.period_secs == 0 {
        return Err(BarError::ZeroPeriod);
    }
    let period_ns = i64::from(bar.period_secs) * 1_000_000_000;
    let ns = bar.event_time.timestamp_nanos_opt().unwrap_or(0);
    if ns.rem_euclid(period_ns) != 0 {
        return Err(BarError::NotOpenAligned(bar.event_time, bar.period_secs));
    }
    if bar.knowledge_time < bar.close_time() {
        return Err(BarError::KnownBeforeClose {
            knowledge: bar.knowledge_time,
            close: bar.close_time(),
        });
    }
    // Observed knowledge_time is when the platform could query the fact, which can
    // never precede receipt. The only exception is a declared backfill sentinel.
    if bar.knowledge_time < bar.ingest_time
        && !bar.quality_flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME)
    {
        return Err(BarError::KnownBeforeIngest {
            knowledge: bar.knowledge_time,
            ingest: bar.ingest_time,
        });
    }
    let lo = bar.low;
    let hi = bar.high;
    if lo > hi || bar.open < lo || bar.open > hi || bar.close < lo || bar.close > hi {
        return Err(BarError::OhlcInconsistent {
            low: lo.to_string(),
            high: hi.to_string(),
        });
    }
    if bar.volume.is_sign_negative() && !bar.volume.is_zero() {
        return Err(BarError::NegativeVolume);
    }
    for p in [bar.open, bar.high, bar.low, bar.close] {
        if p.scale() > PRICE_SCALE {
            return Err(BarError::PrecisionLoss(p.to_string()));
        }
    }
    Ok(())
}

/// Per-source declared vendor lag, used only to backfill `knowledge_time` for
/// history whose knowledge time was never observed (CLAUDE.md §6, OQ-01).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VendorLag {
    pub source_id: i32,
    pub lag: std::time::Duration,
}

/// How `knowledge_time` is set for a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnowledgeProvenance {
    /// Observed at ingest: the platform received a closed bar at `ingest_time`.
    Observed,
    /// History collected after the fact: sentinel `close + vendor_lag`, flagged.
    Backfilled,
}

/// Stamp `knowledge_time` according to its provenance. Backfilled rows are marked
/// `BACKFILLED_KNOWLEDGE_TIME` so every dataset touching them can see the hole.
pub fn stamp_knowledge_time(bar: &mut CanonicalBar, provenance: KnowledgeProvenance, lag: &VendorLag) {
    match provenance {
        KnowledgeProvenance::Observed => {
            bar.knowledge_time = bar.ingest_time.max(bar.close_time());
        }
        KnowledgeProvenance::Backfilled => {
            let lag = Duration::from_std(lag.lag).unwrap_or_else(|_| Duration::zero());
            bar.knowledge_time = bar.close_time() + lag;
            bar.quality_flags |= QualityFlags::BACKFILLED_KNOWLEDGE_TIME;
        }
    }
}

/// Decide provenance from how far after the bar close it arrived. Anything ingested
/// more than `live_tolerance` after close did not come from the live path.
#[must_use]
pub fn provenance_for(bar: &CanonicalBar, live_tolerance: Duration) -> KnowledgeProvenance {
    if bar.ingest_time <= bar.close_time() + live_tolerance {
        KnowledgeProvenance::Observed
    } else {
        KnowledgeProvenance::Backfilled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rust_decimal_macros::dec;

    fn bar(open_min: u32) -> CanonicalBar {
        let et = Utc.with_ymd_and_hms(2026, 1, 1, 10, open_min, 0).unwrap();
        CanonicalBar {
            instrument_id: InstrumentKey(1),
            venue_id: VenueKey(1),
            period_secs: 60,
            event_time: et,
            venue_ts: None,
            ingest_time: et + Duration::seconds(61),
            knowledge_time: et + Duration::seconds(61),
            open: dec!(100),
            high: dec!(101),
            low: dec!(99),
            close: dec!(100.5),
            volume: dec!(3),
            trade_count: Some(10),
            vwap: None,
            bid_close: None,
            ask_close: None,
            quality_flags: QualityFlags::NONE,
            revision_seq: 0,
            source_id: 1,
        }
    }

    #[test]
    fn valid_bar_passes() {
        assert_eq!(validate(&bar(5)), Ok(()));
    }

    #[test]
    fn close_stamped_bar_is_rejected() {
        let mut b = bar(5);
        b.event_time += Duration::seconds(30);
        assert!(matches!(validate(&b), Err(BarError::NotOpenAligned(..))));
    }

    #[test]
    fn knowledge_before_close_is_lookahead() {
        let mut b = bar(5);
        b.knowledge_time = b.event_time + Duration::seconds(30);
        b.ingest_time = b.knowledge_time;
        assert!(matches!(validate(&b), Err(BarError::KnownBeforeClose { .. })));
    }

    #[test]
    fn knowledge_before_ingest_requires_the_backfill_flag() {
        let mut b = bar(5);
        b.ingest_time = b.event_time + Duration::days(3);
        b.knowledge_time = b.close_time();
        assert!(matches!(validate(&b), Err(BarError::KnownBeforeIngest { .. })));
        b.quality_flags |= QualityFlags::BACKFILLED_KNOWLEDGE_TIME;
        assert_eq!(validate(&b), Ok(()));
    }

    #[test]
    fn backfill_sentinel_is_flagged_not_fabricated() {
        let mut b = bar(5);
        b.ingest_time = b.event_time + Duration::days(10);
        let prov = provenance_for(&b, Duration::minutes(5));
        assert_eq!(prov, KnowledgeProvenance::Backfilled);
        stamp_knowledge_time(&mut b, prov, &VendorLag { source_id: 1, lag: std::time::Duration::from_secs(90) });
        assert_eq!(b.knowledge_time, b.close_time() + Duration::seconds(90));
        assert!(b.quality_flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
        assert_eq!(validate(&b), Ok(()));
    }

    #[test]
    fn live_bar_knowledge_is_receipt() {
        let mut b = bar(5);
        b.ingest_time = b.close_time() + Duration::seconds(2);
        let prov = provenance_for(&b, Duration::minutes(5));
        assert_eq!(prov, KnowledgeProvenance::Observed);
        stamp_knowledge_time(&mut b, prov, &VendorLag { source_id: 1, lag: std::time::Duration::ZERO });
        assert_eq!(b.knowledge_time, b.ingest_time);
        assert!(!b.quality_flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
    }

    #[test]
    fn inconsistent_ohlc_rejected() {
        let mut b = bar(5);
        b.high = dec!(99.5);
        assert!(matches!(validate(&b), Err(BarError::OhlcInconsistent { .. })));
    }
}
