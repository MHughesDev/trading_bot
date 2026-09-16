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
    #[error(
        "knowledge_time {knowledge} is only {actual_ms} ms after the bar close {close}, but source \
         {source_id} declares a vendor lag of {declared_ms} ms: the vendor had not published this \
         bar yet, so nothing could have known it (INV-01, ADR-P2-24)"
    )]
    KnownBeforeVendorPublished {
        knowledge: DateTime<Utc>,
        close: DateTime<Utc>,
        actual_ms: i64,
        declared_ms: i64,
        source_id: i32,
    },
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
///
/// An observed row's knowledge time is its receipt — but never earlier than the
/// instant its vendor published the bar. If the two disagree the later wins, which
/// is the direction that cannot invent knowledge: claiming to have held a candle
/// before its vendor released it is lookahead however the stamp got there.
pub fn stamp_knowledge_time(bar: &mut CanonicalBar, provenance: KnowledgeProvenance, lag: &VendorLag) {
    match provenance {
        KnowledgeProvenance::Observed => {
            bar.knowledge_time = bar.ingest_time.max(earliest_knowable(bar, lag));
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

// ─────────────────────────────────────────────────────────────────────────────
// The receipt axis (ADR-P2-24 / OQ-01, and the correction in `source.rs`)
// ─────────────────────────────────────────────────────────────────────────────

/// How much later than the vendor's own publication a forward receipt may land and
/// still be read as one.
///
/// A candle is published by the vendor at `close + declared_lag`. A collector that
/// was watching picks it up within a poll interval or two of that; a collector that
/// pulled it out of a history endpoint picks it up days or weeks later. Five minutes
/// separates those two populations by orders of magnitude, so the exact figure is
/// not load-bearing — what is load-bearing is that the window is measured from the
/// vendor's publication and not from the bar close, because a source with a 900 s
/// declared delay physically cannot deliver sooner and must not be punished for it.
///
/// The value matches the `LIVE_TOLERANCE_SECS` the canonical migration has always
/// used, so this is a change of *axis*, not of threshold.
pub const RECEIPT_GRACE: Duration = Duration::minutes(5);

/// What an already-stored row's ingest stamp says about how that row was collected.
///
/// This is the per-row question, and it is the only one that can be answered from
/// data the platform already holds. It is deliberately *not* "what kind of source is
/// this" — keying provenance on transport marks every forward poll derived and
/// replaces a real receipt with a sentinel that can land later than the truth (see
/// [`crate::source::Collection`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptVerdict {
    /// The ingest stamp is credible as a receipt: the platform held the bytes
    /// between the bar's close and the vendor's publication plus the grace window.
    /// This row was watched arrive, and it is forward evidence (Set P D-P3).
    Receipt { at: DateTime<Utc> },
    /// Ingested long after the vendor published it. Whatever we hold is a receipt
    /// for *our pull*, not for the fact becoming knowable, and recording it as a
    /// knowledge time would post-date the truth. Sentinel plus flag.
    HistoryPull { late_by: Duration },
    /// The ingest stamp precedes the bar's own close. Nothing is knowable before it
    /// happens, so this stamp is clock skew, a parse error, or — as in
    /// `market_bars_v2` — a column that was never really written. It is never
    /// promoted to an observation: sentinel plus flag, and it is counted so the
    /// population is visible rather than laundered into the honest one.
    Impossible { ingest: DateTime<Utc>, close: DateTime<Utc> },
}

impl ReceiptVerdict {
    /// The provenance this verdict implies. Only a genuine receipt is observed.
    #[must_use]
    pub const fn provenance(self) -> KnowledgeProvenance {
        match self {
            Self::Receipt { .. } => KnowledgeProvenance::Observed,
            Self::HistoryPull { .. } | Self::Impossible { .. } => KnowledgeProvenance::Backfilled,
        }
    }

    /// The same answer in `source.rs`'s vocabulary, so the repair of existing rows
    /// and the stamping of new ones cannot drift apart.
    #[must_use]
    pub const fn collection(self) -> crate::source::Collection {
        match self {
            Self::Receipt { at } => crate::source::Collection::Forward { received_at: at },
            Self::HistoryPull { .. } | Self::Impossible { .. } => crate::source::Collection::Historical,
        }
    }

    /// Gate 15 takes forward-test evidence from these rows and only these (D-P3).
    #[must_use]
    pub const fn is_forward_evidence(self) -> bool {
        matches!(self, Self::Receipt { .. })
    }
}

/// Classify a stored row's ingest stamp against its bar close and its source's
/// declared lag.
///
/// This is the whole repair, as one pure function. It answers the only question the
/// data can answer — *was this row watched arrive?* — and it answers it per row,
/// because one source supplies both a forward poll and a history pull.
#[must_use]
pub fn classify_receipt(
    close: DateTime<Utc>,
    ingest: DateTime<Utc>,
    declared_lag: std::time::Duration,
    grace: Duration,
) -> ReceiptVerdict {
    if ingest < close {
        return ReceiptVerdict::Impossible { ingest, close };
    }
    let lag = Duration::from_std(declared_lag).unwrap_or_else(|_| Duration::zero());
    let published = close + lag;
    let deadline = published + grace;
    if ingest <= deadline {
        ReceiptVerdict::Receipt { at: ingest }
    } else {
        ReceiptVerdict::HistoryPull { late_by: ingest - deadline }
    }
}

/// [`classify_receipt`] for a bar that already carries its stamps.
#[must_use]
pub fn receipt_of(bar: &CanonicalBar, lag: &VendorLag, grace: Duration) -> ReceiptVerdict {
    classify_receipt(bar.close_time(), bar.ingest_time, lag.lag, grace)
}

/// The floor a bar's `knowledge_time` may never go below: the instant its vendor
/// published it.
#[must_use]
pub fn earliest_knowable(bar: &CanonicalBar, lag: &VendorLag) -> DateTime<Utc> {
    bar.close_time() + Duration::from_std(lag.lag).unwrap_or_else(|_| Duration::zero())
}

/// The write-time rule that makes the `market_bars_v2` failure structurally
/// impossible to repeat.
///
/// [`validate`] already refuses a bar knowable before it closed. That was not
/// enough: every one of the 158,259 rows in `market_bars_v2` carried
/// `available_time = event_time` — *exactly* the close — and would have passed it,
/// including the rows that came from REST candle endpoints declaring a 60 s lag.
/// A row cannot be knowable before its vendor published it either, and the vendor's
/// publication is `close + declared_vendor_lag`.
///
/// There is no exemption for a flagged row: the sentinel *is* `close + declared_lag`
/// and satisfies the rule with equality, and an observed row's receipt cannot
/// precede the publication it is a receipt for. One rule, no side door.
///
/// # Errors
/// Everything [`validate`] refuses, plus
/// [`BarError::KnownBeforeVendorPublished`].
pub fn validate_against_source(bar: &CanonicalBar, lag: &VendorLag) -> Result<(), BarError> {
    validate(bar)?;
    let floor = earliest_knowable(bar, lag);
    if bar.knowledge_time < floor {
        let close = bar.close_time();
        return Err(BarError::KnownBeforeVendorPublished {
            knowledge: bar.knowledge_time,
            close,
            actual_ms: (bar.knowledge_time - close).num_milliseconds(),
            declared_ms: i64::try_from(lag.lag.as_millis()).unwrap_or(i64::MAX),
            source_id: lag.source_id,
        });
    }
    Ok(())
}

/// Classify a row's receipt and stamp it in one step — the call the write path and
/// the repair of existing rows both make, so neither can drift from the other.
///
/// Returns the verdict so a caller can count the three populations, which is the
/// number this whole exercise exists to produce.
pub fn stamp_from_receipt(bar: &mut CanonicalBar, lag: &VendorLag, grace: Duration) -> ReceiptVerdict {
    let verdict = receipt_of(bar, lag, grace);
    stamp_knowledge_time(bar, verdict.provenance(), lag);
    verdict
}

/// Would this stored row be refused by [`validate_against_source`] *and* carry no
/// flag admitting it?
///
/// This is the census's alarm predicate, and it is the shape of the bug as it
/// actually shipped: a `knowledge_time` that is too early for its source to have
/// published, presented as fact. A row that is too early but *flagged* is an honest
/// hole; a row that is too early and unflagged is a fiction everything downstream
/// reads as true.
#[must_use]
pub fn is_unflagged_sentinel_violation(bar: &CanonicalBar, lag: &VendorLag) -> bool {
    bar.knowledge_time < earliest_knowable(bar, lag)
        && !bar.quality_flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME)
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

    // ── the receipt axis ──────────────────────────────────────────────────────

    /// 60 s, as every REST candle endpoint declares in `dataplane.source`.
    fn rest_lag() -> VendorLag {
        VendorLag { source_id: 2, lag: std::time::Duration::from_secs(60) }
    }

    fn live_lag() -> VendorLag {
        VendorLag { source_id: 9, lag: std::time::Duration::ZERO }
    }

    /// **The definition-of-done test.** An observed row and a backfilled row for the
    /// *same bar close* must be distinguishable — in their knowledge time, in their
    /// flags, and in whether Gate 15 may read them (Set P D-P2, D-P3).
    ///
    /// `market_bars_v2` failed exactly here: both populations were stamped at the
    /// close, so nothing downstream could tell them apart.
    #[test]
    fn an_observed_and_a_backfilled_row_for_one_bar_close_are_distinguishable() {
        let mut watched = bar(5);
        watched.ingest_time = watched.close_time() + Duration::seconds(70);
        let mut pulled = bar(5);
        pulled.ingest_time = pulled.close_time() + Duration::days(21);
        assert_eq!(watched.close_time(), pulled.close_time(), "same bar, same close");

        let vw = stamp_from_receipt(&mut watched, &rest_lag(), RECEIPT_GRACE);
        let vp = stamp_from_receipt(&mut pulled, &rest_lag(), RECEIPT_GRACE);

        assert!(matches!(vw, ReceiptVerdict::Receipt { .. }));
        assert!(matches!(vp, ReceiptVerdict::HistoryPull { .. }));
        assert!(vw.is_forward_evidence());
        assert!(!vp.is_forward_evidence(), "the sentinel must never reach Gate 15 (D-P3)");

        assert_ne!(watched.knowledge_time, pulled.knowledge_time);
        assert_eq!(watched.knowledge_time, watched.ingest_time, "the real receipt is kept");
        assert_eq!(pulled.knowledge_time, pulled.close_time() + Duration::seconds(60), "sentinel = close + declared lag");
        assert!(!watched.quality_flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
        assert!(pulled.quality_flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));

        assert_eq!(validate_against_source(&watched, &rest_lag()), Ok(()));
        assert_eq!(validate_against_source(&pulled, &rest_lag()), Ok(()));
    }

    /// The axis is the receipt, not the transport. A REST poll the platform watched
    /// arrive is observed; keying on `kind = 'rest_history'` would mark it derived
    /// and leave a forward collector supplying Gate 15 with nothing.
    #[test]
    fn a_forward_rest_poll_is_observed_however_the_source_is_classified() {
        let mut b = bar(5);
        b.source_id = 2; // coinbase_rest — a `rest_history` source
        b.ingest_time = b.close_time() + Duration::seconds(65);
        let v = stamp_from_receipt(&mut b, &rest_lag(), RECEIPT_GRACE);
        assert!(v.is_forward_evidence(), "transport is not provenance");
        assert!(!b.quality_flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
    }

    /// And the converse: a live-stream source whose row only reached the store weeks
    /// later is *not* observed. One source, both legs, decided per row.
    #[test]
    fn a_live_source_row_pulled_late_is_not_observed() {
        let mut b = bar(5);
        b.source_id = 9; // kraken_ws — a `live_stream` source
        b.ingest_time = b.close_time() + Duration::days(9);
        let v = stamp_from_receipt(&mut b, &live_lag(), RECEIPT_GRACE);
        assert!(!v.is_forward_evidence());
        assert!(b.quality_flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
        assert_eq!(b.knowledge_time, b.close_time(), "live streams declare no lag, so the sentinel is the close");
    }

    /// The window is measured from the vendor's publication, not the bar close, so a
    /// source that is *declared* 15 minutes delayed is not punished for delivering
    /// 15 minutes late. Observed and observed-late are different facts.
    #[test]
    fn the_receipt_window_moves_with_the_declared_lag() {
        let delayed = VendorLag { source_id: 5, lag: std::time::Duration::from_secs(900) };
        let mut b = bar(5);
        b.ingest_time = b.close_time() + Duration::seconds(950);
        let v = stamp_from_receipt(&mut b, &delayed, RECEIPT_GRACE);
        assert!(v.is_forward_evidence(), "a 950 s receipt from a 900 s-delayed vendor is a forward poll");
        // The same receipt against a 60 s vendor is far too late to be one.
        let mut c = bar(5);
        c.ingest_time = c.close_time() + Duration::seconds(950);
        assert!(!stamp_from_receipt(&mut c, &rest_lag(), RECEIPT_GRACE).is_forward_evidence());
    }

    /// Nothing is observed before it happens. An ingest stamp that precedes the bar's
    /// own close is never promoted to an observation — it gets the sentinel and the
    /// flag, and it stays countable as its own population.
    #[test]
    fn an_impossible_receipt_is_never_laundered_into_an_observation() {
        let mut b = bar(5);
        b.ingest_time = b.close_time() - Duration::seconds(1);
        let v = stamp_from_receipt(&mut b, &rest_lag(), RECEIPT_GRACE);
        assert!(matches!(v, ReceiptVerdict::Impossible { .. }));
        assert!(!v.is_forward_evidence());
        assert!(b.quality_flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
        assert_eq!(b.knowledge_time, b.close_time() + Duration::seconds(60));
    }

    /// **The write-time rule, as the bug it exists to stop.** Every one of the
    /// 158,259 rows in `market_bars_v2` carried `available_time = event_time` — the
    /// bar close exactly — including rows from REST candle endpoints declaring 60 s.
    /// `validate` passed all of them. `validate_against_source` refuses them.
    #[test]
    fn a_zero_lag_row_from_a_lagged_source_is_refused_at_write_time() {
        let mut b = bar(5);
        b.source_id = 2;
        b.ingest_time = b.close_time();
        b.knowledge_time = b.close_time(); // exactly what v2 stored, for every row
        assert_eq!(validate(&b), Ok(()), "the old validator accepted this — that is the bug");
        let err = validate_against_source(&b, &rest_lag()).unwrap_err();
        assert!(matches!(err, BarError::KnownBeforeVendorPublished { declared_ms: 60_000, actual_ms: 0, .. }));
        assert!(err.to_string().contains("had not published"));
    }

    /// And the rule has no side door: flagging a row does not buy it an exemption,
    /// because the sentinel already satisfies the rule with equality. A row that
    /// claims to be knowable before its vendor published is refused flagged or not.
    #[test]
    fn the_backfill_flag_does_not_exempt_a_row_from_the_vendor_lag_floor() {
        let mut b = bar(5);
        b.knowledge_time = b.close_time();
        b.ingest_time = b.close_time();
        b.quality_flags |= QualityFlags::BACKFILLED_KNOWLEDGE_TIME;
        assert!(matches!(
            validate_against_source(&b, &rest_lag()),
            Err(BarError::KnownBeforeVendorPublished { .. })
        ));
        // The honest sentinel passes.
        b.knowledge_time = b.close_time() + Duration::seconds(60);
        assert_eq!(validate_against_source(&b, &rest_lag()), Ok(()));
    }

    /// A lag-0 source degenerates to the rule `validate` already had, so live
    /// aggregation that stamps exactly the close is still legal.
    #[test]
    fn a_live_source_may_still_stamp_the_close_itself() {
        let mut b = bar(5);
        b.ingest_time = b.close_time();
        b.knowledge_time = b.close_time();
        assert_eq!(validate_against_source(&b, &live_lag()), Ok(()));
    }

    /// The census's alarm predicate: too early *and* unflagged. Too early with a flag
    /// is an honest hole; too early without one is a fiction.
    #[test]
    fn the_alarm_fires_only_on_the_unflagged_population() {
        let mut b = bar(5);
        b.knowledge_time = b.close_time();
        assert!(is_unflagged_sentinel_violation(&b, &rest_lag()));
        b.quality_flags |= QualityFlags::BACKFILLED_KNOWLEDGE_TIME;
        assert!(!is_unflagged_sentinel_violation(&b, &rest_lag()), "a flagged hole is not the alarm's business");
        assert!(!is_unflagged_sentinel_violation(&bar(5), &live_lag()));
    }

    /// An observed receipt never back-dates knowledge below the vendor's publication.
    /// If a poll somehow lands before the vendor could have published, the later of
    /// the two wins — the direction that cannot invent knowledge.
    #[test]
    fn an_early_receipt_is_raised_to_the_vendor_publication() {
        let mut b = bar(5);
        b.ingest_time = b.close_time() + Duration::seconds(10);
        stamp_from_receipt(&mut b, &rest_lag(), RECEIPT_GRACE);
        assert_eq!(b.knowledge_time, b.close_time() + Duration::seconds(60));
        assert_eq!(validate_against_source(&b, &rest_lag()), Ok(()));
    }

    /// The repair and the write path must agree, so the verdict speaks `source.rs`'s
    /// vocabulary rather than a parallel one.
    #[test]
    fn the_verdict_maps_onto_the_collection_axis() {
        use crate::source::Collection;
        let mut watched = bar(5);
        watched.ingest_time = watched.close_time() + Duration::seconds(70);
        let recv = watched.ingest_time;
        assert_eq!(receipt_of(&watched, &rest_lag(), RECEIPT_GRACE).collection(), Collection::Forward { received_at: recv });
        let mut pulled = bar(5);
        pulled.ingest_time = pulled.close_time() + Duration::days(3);
        assert_eq!(receipt_of(&pulled, &rest_lag(), RECEIPT_GRACE).collection(), Collection::Historical);
    }

    #[test]
    fn inconsistent_ohlc_rejected() {
        let mut b = bar(5);
        b.high = dec!(99.5);
        assert!(matches!(validate(&b), Err(BarError::OhlcInconsistent { .. })));
    }
}
