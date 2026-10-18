//! The coverage census (Set P, P-0.1; DATA-005 §7 "Coverage", §12).
//!
//! *How much history does the platform actually have?* Until this module existed the
//! answer was an anecdote — "BTC-USD holds 22 one-hour bars", observed once on
//! 2026-09-08 — and the whole programme's sequencing rested on it. The census turns
//! that into a number, per `(instrument, timeframe, venue)`, recomputed nightly and
//! kept as a time series rather than a snapshot that overwrites its own history.
//!
//! Pure logic, by the crate's rule: it is handed the bar *open* times a
//! point-in-time read returned and the instrument's calendar, and it computes.
//! It never names a bar table and never writes one — reading bars is
//! `BarStore`'s job and only `BarStore`'s (INV-01, AT-01).
//!
//! Two properties this module is built around:
//!
//! * **Determinism.** Two runs over an unchanged database produce byte-identical
//!   output except `as_of`. There is no clock read, no float, and no iteration over
//!   a hash map anywhere in here.
//! * **Honest knowledge provenance.** `derived_knowledge_bars` counts rows whose
//!   `knowledge_time` was *inferred* (the `BACKFILLED_KNOWLEDGE_TIME` sentinel)
//!   rather than observed at ingest. A census that reported only "how many bars"
//!   would flatter a series whose entire history is a backfill stamped as if it had
//!   been watched live — which is precisely the bug that shipped in
//!   `market_bars_v2` (ADR-P2-24, `clickhouse/07_canonical_market.sql`).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::calendar::Calendar;

/// Gate 9 of `strict_v1` wants five calendar years of history.
///
/// Expressed in days so a shortfall can be stated in days, which is the unit a
/// human plans in. Five years including the leap days a five-year window always
/// contains at least one of.
pub const STRICT_V1_FLOOR_DAYS: i64 = 1826;

/// `paper_v1` (ADR-P2-14) is the profile that carries the programme while depth
/// accrues. Its floor is one calendar year.
pub const PAPER_V1_FLOOR_DAYS: i64 = 365;

/// One bar, as the census needs it: when the bar opened, and whether the platform
/// genuinely observed when it learned of it.
///
/// Deliberately not [`crate::bar::CanonicalBar`] — the census reads millions of
/// rows and needs two fields of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CensusBar {
    /// Bar OPEN, Unix nanoseconds (the stored `event_time`; INV-06).
    pub open_ns: i64,
    /// `true` when `knowledge_time` was derived from a declared vendor lag rather
    /// than observed at ingest — the `BACKFILLED_KNOWLEDGE_TIME` flag.
    pub knowledge_derived: bool,
    /// `true` when this row's `knowledge_time` is *earlier* than the instant its
    /// source could have published it (`close + declared_vendor_lag`) **and** it
    /// carries no `BACKFILLED_KNOWLEDGE_TIME` flag admitting it.
    ///
    /// That combination is not a hole, it is a fiction: everything downstream —
    /// embargo arithmetic, PIT filtering, a dataset's `max_knowledge_lag_ms` —
    /// reads it as an observed fact. It is the shape of the bug that put 158,259
    /// rows into `market_bars_v2` stamped as if they had been watched live, and the
    /// reason this is a counted alarm rather than a number on a page.
    ///
    /// See [`crate::bar::is_unflagged_sentinel_violation`], which is the predicate.
    pub unflagged_sentinel: bool,
}

impl CensusBar {
    #[must_use]
    pub const fn observed(open_ns: i64) -> Self {
        Self {
            open_ns,
            knowledge_derived: false,
            unflagged_sentinel: false,
        }
    }

    #[must_use]
    pub const fn derived(open_ns: i64) -> Self {
        Self {
            open_ns,
            knowledge_derived: true,
            unflagged_sentinel: false,
        }
    }

    /// A row whose knowledge time is too early for its source and which does not
    /// admit it. Counted as observed — which is precisely the problem.
    #[must_use]
    pub const fn unflagged_sentinel(open_ns: i64) -> Self {
        Self {
            open_ns,
            knowledge_derived: false,
            unflagged_sentinel: true,
        }
    }
}

/// What a series' depth permits, stated in the vocabulary of the gate funnel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FloorVerdict {
    /// The continuous span already clears the floor.
    Satisfied,
    /// It does not, and this many more days of collection at the observed cadence
    /// would clear it.
    Short {
        shortfall_days: i64,
        days_at_current_rate: i64,
    },
    /// The series has no bars at all, so no rate can be observed and no date named.
    NoData,
}

impl FloorVerdict {
    #[must_use]
    pub const fn is_satisfied(self) -> bool {
        matches!(self, Self::Satisfied)
    }
}

/// The census for one `(instrument, timeframe, venue)` at one `as_of`.
///
/// Counts are `u64` and times are Unix nanoseconds so the row round-trips through
/// ClickHouse and Postgres without a float anywhere near it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageCensus {
    pub instrument: String,
    pub venue: String,
    pub timeframe: String,
    pub period_secs: u32,
    /// When the census was taken. The only field that differs between two runs over
    /// an unchanged database.
    pub as_of: DateTime<Utc>,

    /// First bar OPEN, Unix ns. `None` for an empty series.
    pub first_bar_ns: Option<i64>,
    /// Last bar OPEN, Unix ns. `None` for an empty series.
    pub last_bar_ns: Option<i64>,

    /// Bars the calendar says should exist between `first_bar` and `last_bar`
    /// inclusive. For a 24/7 calendar this is the arithmetic span; for a session
    /// calendar, only the periods inside a session count.
    pub expected_bars: u64,
    /// Distinct bar opens actually present.
    pub present_bars: u64,
    /// `expected_bars - present_bars`, floored at zero.
    pub missing_bars: u64,
    /// Missing share in basis points — an integer, so the census hashes stably.
    /// 10_000 bps = 100%.
    pub missing_bps: u32,

    /// The longest run of consecutive *expected* bars with none present, counted in
    /// periods. A perfectly dense series reports 0.
    pub longest_gap_periods: u64,
    /// The longest stretch with no gap longer than one period, in bars. This is the
    /// number that decides whether a walk-forward study is possible at all — total
    /// bar count over a swiss-cheese span does not.
    pub longest_continuous_bars: u64,
    /// The same stretch in whole seconds.
    pub longest_continuous_secs: i64,

    /// Bars whose `knowledge_time` was derived, not observed.
    pub derived_knowledge_bars: u64,
    /// Bars whose `knowledge_time` was observed at ingest. Gate 15 takes forward
    /// evidence from these and only these (D-P3).
    pub observed_knowledge_bars: u64,
    /// Bars counted above as *observed* whose knowledge time is nonetheless earlier
    /// than their source could have published, with no flag admitting it.
    ///
    /// A subset of `observed_knowledge_bars`, and the one number in this struct that
    /// is an **alarm rather than a measurement**: any value above zero means the
    /// series' observed count is overstated by that much, and that whatever wrote
    /// those rows is still writing them. Zero is the only acceptable reading; see
    /// [`CoverageCensus::provenance_alarm`].
    pub unflagged_sentinel_bars: u64,

    /// Whether the continuous span clears `strict_v1` Gate 9's five-year floor.
    pub strict_v1: FloorVerdict,
    /// Whether it clears `paper_v1`'s one-year floor.
    pub paper_v1: FloorVerdict,
}

/// A series whose knowledge provenance is not what it claims.
///
/// Returned rather than logged so a caller has to do something with it. The census
/// job raises it; the silence alarm's siblings in P-0.6 carry it to an operator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceAlarm {
    pub instrument: String,
    pub venue: String,
    pub timeframe: String,
    /// Rows claiming an observed knowledge time their source could not have
    /// supported, carrying no flag.
    pub unflagged_sentinel_bars: u64,
    /// What the series reports as observed, including the overstatement.
    pub observed_knowledge_bars: u64,
}

impl ProvenanceAlarm {
    /// Observed rows that survive the alarm — the series' true stock of
    /// Gate-15-admissible history (D-P3).
    #[must_use]
    pub const fn defensibly_observed_bars(&self) -> u64 {
        self.observed_knowledge_bars
            .saturating_sub(self.unflagged_sentinel_bars)
    }
}

impl std::fmt::Display for ProvenanceAlarm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}@{} {}: {} of {} rows claim an observed knowledge_time earlier than their source could \
             have published, with no BACKFILLED_KNOWLEDGE_TIME flag. Only {} are defensibly observed \
             and admissible as Gate 15 forward evidence (INV-01, ADR-P2-24, Set P D-P3)",
            self.instrument,
            self.venue,
            self.timeframe,
            self.unflagged_sentinel_bars,
            self.observed_knowledge_bars,
            self.defensibly_observed_bars(),
        )
    }
}

impl CoverageCensus {
    /// Longest continuous coverage in whole days — the headline number.
    #[must_use]
    pub const fn continuous_days(&self) -> i64 {
        self.longest_continuous_secs / 86_400
    }

    /// Rows this series may honestly offer Gate 15 as forward evidence: observed,
    /// minus the observed rows that cannot defend the claim.
    ///
    /// On `market_bars_v2` as measured on 2026-09-15 this was zero for every series,
    /// and that was the correct answer for *that table* — its entire history was
    /// stamped at the bar close.
    ///
    /// **That reading is void.** v2 is frozen (ADR-D-29) and has been stale since
    /// 2026-09-14; the canonical `market_bar` is what the platform holds now. Counted
    /// against `market_bar` on 2026-09-22, 160,715 of its 170,924 rows carry
    /// `BACKFILLED_KNOWLEDGE_TIME` honestly and roughly 10,194 survive this
    /// subtraction — nearly all of them BTC-USD/kraken/1m over `kraken_ws`, which
    /// alone contributes 10,094 with no unflagged sentinel among them. The platform
    /// does now hold forward evidence; it simply held none in the table the earlier
    /// census was reading.
    ///
    /// The residue is 15 rows, spread over BTC-USD/coinbase/1m (9),
    /// BTC-USD/kraken/1m via `kraken_rest` (5) and ETH-USD/coinbase/1m (1): rows
    /// claiming observed knowledge stamped no later than `close + declared_lag`,
    /// which is exactly the sentinel. Small, non-zero, and precisely what this
    /// subtraction exists to remove.
    #[must_use]
    pub const fn defensibly_observed_bars(&self) -> u64 {
        self.observed_knowledge_bars
            .saturating_sub(self.unflagged_sentinel_bars)
    }

    /// `Some` when any row claims observed knowledge it cannot support.
    ///
    /// A census that merely *reported* the derived/observed split would have shown
    /// 158,259 observed rows and looked healthy. The split is only meaningful next
    /// to the count of rows whose claim does not survive inspection, so this is the
    /// reading callers are expected to act on.
    #[must_use]
    pub fn provenance_alarm(&self) -> Option<ProvenanceAlarm> {
        (self.unflagged_sentinel_bars > 0).then(|| ProvenanceAlarm {
            instrument: self.instrument.clone(),
            venue: self.venue.clone(),
            timeframe: self.timeframe.clone(),
            unflagged_sentinel_bars: self.unflagged_sentinel_bars,
            observed_knowledge_bars: self.observed_knowledge_bars,
        })
    }
}

/// Compute the census for one series.
///
/// `bars` need not be sorted or deduplicated; this function sorts a copy and
/// collapses repeats, so a caller that read two revisions of the same bar gets one
/// bar. A bar is counted as derived if *any* copy of it was derived: a series is no
/// more trustworthy than the least honest stamp in it.
///
/// `calendar` decides `expected_bars`. For crypto that is
/// [`Calendar::Continuous`]; for an equity series it is the venue's session
/// calendar, so a weekend is not reported as a 48-hour hole.
#[must_use]
pub fn census(
    instrument: &str,
    venue: &str,
    timeframe: &str,
    period_secs: u32,
    bars: &[CensusBar],
    calendar: &Calendar,
    as_of: DateTime<Utc>,
) -> CoverageCensus {
    let period_ns = i64::from(period_secs.max(1)) * 1_000_000_000;

    // Sort and collapse duplicates. `CensusBar` orders on `open_ns` first, so a
    // dedup by open time is a linear pass.
    let mut sorted: Vec<CensusBar> = bars.to_vec();
    sorted.sort_unstable();
    let mut opens: Vec<i64> = Vec::with_capacity(sorted.len());
    let mut derived: Vec<bool> = Vec::with_capacity(sorted.len());
    let mut unflagged: Vec<bool> = Vec::with_capacity(sorted.len());
    for b in sorted {
        if opens.last() == Some(&b.open_ns) {
            // Any derived copy taints the bar.
            if let Some(last) = derived.last_mut() {
                *last = *last || b.knowledge_derived;
            }
            // And any copy that claims more than it can support taints it too: a
            // bar is no more defensible than its least defensible revision.
            if let Some(last) = unflagged.last_mut() {
                *last = *last || b.unflagged_sentinel;
            }
            continue;
        }
        opens.push(b.open_ns);
        derived.push(b.knowledge_derived);
        unflagged.push(b.unflagged_sentinel);
    }

    let present_bars = opens.len() as u64;
    let derived_knowledge_bars = derived.iter().filter(|d| **d).count() as u64;
    let observed_knowledge_bars = present_bars - derived_knowledge_bars;
    // A flagged row is an honest hole and is already counted as derived, so the
    // alarm counts only rows still presented as observed.
    let unflagged_sentinel_bars = unflagged
        .iter()
        .zip(derived.iter())
        .filter(|(u, d)| **u && !**d)
        .count() as u64;

    let (first_bar_ns, last_bar_ns) = match (opens.first(), opens.last()) {
        (Some(f), Some(l)) => (Some(*f), Some(*l)),
        _ => (None, None),
    };

    let expected_bars = match (first_bar_ns, last_bar_ns) {
        (Some(f), Some(l)) => expected_between(f, l, period_ns, calendar),
        _ => 0,
    };
    let missing_bars = expected_bars.saturating_sub(present_bars);
    // Integer arithmetic on purpose: a float here would make two runs of the census
    // differ in the last digit on different hardware. `checked_div` carries the
    // "nothing was expected" case in the type rather than in a surrounding `if`, so
    // the divide-by-zero cannot be reintroduced by editing the guard away.
    let missing_bps = match missing_bars
        .saturating_mul(10_000)
        .checked_div(expected_bars)
    {
        // Nothing expected means nothing can be missing.
        None => 0,
        Some(bps) => u32::try_from(bps).unwrap_or(10_000),
    };

    let tolerance = no_trade_tolerance_periods(period_secs);
    let (longest_gap_periods, longest_continuous_bars) = runs(&opens, period_ns, calendar, tolerance);
    let longest_continuous_secs =
        longest_continuous_bars.saturating_mul(u64::from(period_secs.max(1)));
    let longest_continuous_secs = i64::try_from(longest_continuous_secs).unwrap_or(i64::MAX);

    let continuous_days = longest_continuous_secs / 86_400;
    let rate = observed_daily_rate(first_bar_ns, last_bar_ns, period_secs, present_bars);
    let strict_v1 = verdict(continuous_days, STRICT_V1_FLOOR_DAYS, present_bars, rate);
    let paper_v1 = verdict(continuous_days, PAPER_V1_FLOOR_DAYS, present_bars, rate);

    CoverageCensus {
        instrument: instrument.to_string(),
        venue: venue.to_string(),
        timeframe: timeframe.to_string(),
        period_secs,
        as_of,
        first_bar_ns,
        last_bar_ns,
        expected_bars,
        present_bars,
        missing_bars,
        missing_bps,
        longest_gap_periods,
        longest_continuous_bars,
        longest_continuous_secs,
        derived_knowledge_bars,
        observed_knowledge_bars,
        unflagged_sentinel_bars,
        strict_v1,
        paper_v1,
    }
}

/// Bars the calendar expects in `[first, last]` inclusive.
fn expected_between(first_ns: i64, last_ns: i64, period_ns: i64, calendar: &Calendar) -> u64 {
    match calendar {
        Calendar::Continuous { .. } => {
            let span = last_ns.saturating_sub(first_ns);
            (span / period_ns).unsigned_abs().saturating_add(1)
        }
        Calendar::Sessions { .. } => {
            let mut n: u64 = 0;
            let mut t = first_ns;
            while t <= last_ns {
                if calendar.is_open(DateTime::from_timestamp_nanos(t)) {
                    n += 1;
                }
                t = match t.checked_add(period_ns) {
                    Some(next) => next,
                    None => break,
                };
            }
            n
        }
    }
}

/// A missing stretch this short, in wall-clock seconds, is a *no-trade interval*,
/// not a gap (ADR-P1-7c, 2026-10-04).
///
/// Coinbase publishes a 1m candle only for a minute in which something traded, so
/// a quiet minute is simply absent. 50 of the 58 holes in three years of BTC-USD
/// 1m are 1-12 minutes long; the other eight are venue outages (the largest, 391
/// minutes on 2026-05-08, is not served by Coinbase's candle endpoint even now).
/// Counting a quiet minute as a break reset the run at every one of them and
/// reported a 305-day "continuous" span for a series that is present for 3.09
/// years. Fifteen minutes is the smallest round figure above the longest
/// no-trade hole seen (12), and below the first outage (25).
///
/// It is wall-clock, not a count of periods, so it forgives nothing at 1h or 1d:
/// a missing hourly bar on a 24/7 pair is not a quiet hour. The missing bars are
/// still counted as missing, in coverage and in the grade; only the *run* is
/// carried across them.
pub const NO_TRADE_TOLERANCE_SECS: i64 = 900;

/// The tolerance in whole periods for a timeframe.
#[must_use]
pub fn no_trade_tolerance_periods(period_secs: u32) -> u64 {
    u64::try_from(NO_TRADE_TOLERANCE_SECS / i64::from(period_secs.max(1))).unwrap_or(0)
}

/// `(longest gap in periods, longest continuous run in bars)`.
///
/// A "gap" counts only periods the calendar says should have had a bar, so an
/// overnight session break is not a gap. The run is broken by any gap longer than
/// `tolerance` periods; a shorter hole is a no-trade interval, carried across, and
/// its slots count toward the run's length because the run is a *span*.
/// `longest_gap` still reports every hole, tolerated or not.
fn runs(opens: &[i64], period_ns: i64, calendar: &Calendar, tolerance: u64) -> (u64, u64) {
    if opens.is_empty() {
        return (0, 0);
    }
    let mut longest_gap: u64 = 0;
    let mut longest_run: u64 = 1;
    let mut run: u64 = 1;
    for w in opens.windows(2) {
        let (prev, cur) = (w[0], w[1]);
        let missing = expected_strictly_between(prev, cur, period_ns, calendar);
        longest_gap = longest_gap.max(missing);
        if missing <= tolerance {
            run += 1 + missing;
            longest_run = longest_run.max(run);
        } else {
            run = 1;
        }
    }
    (longest_gap, longest_run)
}

/// Expected bar slots strictly between two present bars.
fn expected_strictly_between(prev: i64, cur: i64, period_ns: i64, calendar: &Calendar) -> u64 {
    match calendar {
        Calendar::Continuous { .. } => {
            let periods = (cur.saturating_sub(prev) / period_ns).unsigned_abs();
            periods.saturating_sub(1)
        }
        Calendar::Sessions { .. } => {
            let mut n: u64 = 0;
            let mut t = match prev.checked_add(period_ns) {
                Some(t) => t,
                None => return 0,
            };
            while t < cur {
                if calendar.is_open(DateTime::from_timestamp_nanos(t)) {
                    n += 1;
                }
                t = match t.checked_add(period_ns) {
                    Some(next) => next,
                    None => break,
                };
            }
            n
        }
    }
}

/// Coverage-days accrued per calendar day over the series' observed span.
///
/// `1.0` is a collector that never misses a bar; `0.5` is one that misses half of
/// them and therefore needs twice as long to reach a floor. Returns `None` when
/// there is no span to measure a rate over, in which case the projection falls back
/// to the shortfall itself.
///
/// Deliberately measured on *present* bars against wall-clock span, so a series
/// full of holes projects the slower date it has earned rather than the optimistic
/// one its start and end dates imply. The rate is a projection, not a stored fact,
/// which is why a float is acceptable here and nowhere else in this module — it
/// never reaches a persisted count.
fn observed_daily_rate(
    first_ns: Option<i64>,
    last_ns: Option<i64>,
    period_secs: u32,
    present: u64,
) -> Option<f64> {
    let (f, l) = (first_ns?, last_ns?);
    let span_secs = (l - f) / 1_000_000_000;
    if span_secs <= 0 || present == 0 {
        return None;
    }
    let covered_secs = (present as f64) * f64::from(period_secs.max(1));
    Some((covered_secs / span_secs as f64).clamp(f64::MIN_POSITIVE, 1.0))
}

fn verdict(continuous_days: i64, floor_days: i64, present: u64, rate: Option<f64>) -> FloorVerdict {
    if present == 0 {
        return FloorVerdict::NoData;
    }
    if continuous_days >= floor_days {
        return FloorVerdict::Satisfied;
    }
    let shortfall_days = floor_days - continuous_days;
    // At the observed rate: a collector accruing one coverage-day per calendar day
    // needs exactly `shortfall_days` more days. A slower one needs proportionally
    // more. `rate` is never zero here (it is `None` instead).
    let days_at_current_rate = match rate {
        Some(r) if r > 0.0 => ((shortfall_days as f64) / r).ceil() as i64,
        _ => shortfall_days,
    };
    FloorVerdict::Short {
        shortfall_days,
        days_at_current_rate,
    }
}

/// How old the most recent row is, for the forward-series silence alarm (P-0.6).
#[must_use]
pub fn staleness(
    last_bar_ns: Option<i64>,
    period_secs: u32,
    as_of: DateTime<Utc>,
) -> Option<Duration> {
    let last = last_bar_ns?;
    let close = last + i64::from(period_secs) * 1_000_000_000;
    Some(as_of - DateTime::<Utc>::from_timestamp_nanos(close))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::CalendarVersion;
    use chrono::{NaiveDate, NaiveTime, TimeZone};

    const MIN: i64 = 60 * 1_000_000_000;

    fn crypto() -> Calendar {
        Calendar::Continuous {
            version: CalendarVersion::crypto_24_7(),
        }
    }

    fn at(n: i64) -> DateTime<Utc> {
        Utc.timestamp_nanos(n)
    }

    fn dense(n: i64) -> Vec<CensusBar> {
        (0..n).map(|i| CensusBar::observed(i * MIN)).collect()
    }

    #[test]
    fn dense_series_has_no_gap() {
        let c = census(
            "BTC-USD",
            "coinbase",
            "1m",
            60,
            &dense(100),
            &crypto(),
            at(0),
        );
        assert_eq!(c.present_bars, 100);
        assert_eq!(c.expected_bars, 100);
        assert_eq!(c.missing_bars, 0);
        assert_eq!(c.missing_bps, 0);
        assert_eq!(c.longest_gap_periods, 0);
        assert_eq!(c.longest_continuous_bars, 100);
    }

    /// The planted-gap acceptance: a known 3-bar hole reports exactly that hole and
    /// exactly its length.
    #[test]
    fn planted_three_bar_gap_is_reported_exactly() {
        let mut bars = dense(50);
        // Remove bars 20, 21, 22 — a hole of exactly three periods.
        bars.retain(|b| !(20..23).contains(&(b.open_ns / MIN)));
        let c = census("BTC-USD", "coinbase", "1m", 60, &bars, &crypto(), at(0));
        assert_eq!(c.present_bars, 47);
        assert_eq!(c.expected_bars, 50);
        assert_eq!(c.missing_bars, 3);
        assert_eq!(c.longest_gap_periods, 3, "a 3-bar hole is a 3-period gap");
        // ADR-P1-7c: three missing minutes are a no-trade interval, so the run is
        // carried across them: the whole 50-slot span.
        assert_eq!(c.longest_continuous_bars, 50);
        assert_eq!(c.missing_bps, 600);
    }

    /// A hole longer than the no-trade tolerance is an outage and breaks the run.
    #[test]
    fn a_hole_longer_than_the_no_trade_tolerance_breaks_the_run() {
        let mut bars = dense(100);
        // 20 minutes missing: bars 30..50.
        bars.retain(|b| !(30..50).contains(&(b.open_ns / MIN)));
        let c = census("BTC-USD", "coinbase", "1m", 60, &bars, &crypto(), at(0));
        assert_eq!(c.longest_gap_periods, 20);
        assert_eq!(c.missing_bars, 20);
        // Bars 50..99 (50) against 0..29 (30): the longer side.
        assert_eq!(c.longest_continuous_bars, 50);

        // Exactly at the tolerance (15) is still a no-trade interval.
        let mut edge = dense(100);
        edge.retain(|b| !(30..45).contains(&(b.open_ns / MIN)));
        let c = census("BTC-USD", "coinbase", "1m", 60, &edge, &crypto(), at(0));
        assert_eq!(c.longest_gap_periods, 15);
        assert_eq!(c.longest_continuous_bars, 100);
    }

    /// The tolerance is wall-clock: it forgives nothing at 1h.
    #[test]
    fn the_no_trade_tolerance_does_not_apply_to_hourly_bars() {
        assert_eq!(no_trade_tolerance_periods(60), 15);
        assert_eq!(no_trade_tolerance_periods(3600), 0);
        assert_eq!(no_trade_tolerance_periods(86_400), 0);
    }

    #[test]
    fn two_gaps_report_the_longer() {
        let mut bars = dense(100);
        bars.retain(|b| {
            let i = b.open_ns / MIN;
            !(10..12).contains(&i) && !(40..47).contains(&i)
        });
        let c = census("X", "v", "1m", 60, &bars, &crypto(), at(0));
        assert_eq!(c.longest_gap_periods, 7);
        assert_eq!(c.missing_bars, 9);
    }

    /// A series stamped exactly as `market_bars_v2` stamped every one of its rows:
    /// nothing flagged, so the derived/observed split reports it as wholly observed
    /// — and the alarm says the observed count is worth nothing.
    ///
    /// This is the difference between a census that reports a number and one that
    /// reports a number nobody can be fooled by.
    #[test]
    fn a_wholly_unflagged_sentinel_series_reports_zero_defensibly_observed() {
        let bars: Vec<CensusBar> = (0..500)
            .map(|i| CensusBar::unflagged_sentinel(i * MIN))
            .collect();
        let c = census("BTC-USD", "coinbase", "1m", 60, &bars, &crypto(), at(0));
        assert_eq!(
            c.derived_knowledge_bars, 0,
            "nothing was flagged — that is the bug, not the fix"
        );
        assert_eq!(
            c.observed_knowledge_bars, 500,
            "the naive split calls all 500 observed"
        );
        assert_eq!(c.unflagged_sentinel_bars, 500);
        assert_eq!(
            c.defensibly_observed_bars(),
            0,
            "and none of them may reach Gate 15"
        );
        let alarm = c
            .provenance_alarm()
            .expect("500 unsupportable claims must alarm");
        assert_eq!(alarm.defensibly_observed_bars(), 0);
        assert!(alarm.to_string().contains("BACKFILLED_KNOWLEDGE_TIME"));
    }

    /// The alarm is silent on an honest store: flagged holes are not violations, and
    /// genuinely observed rows are not either.
    #[test]
    fn an_honest_series_raises_no_provenance_alarm() {
        let mut bars: Vec<CensusBar> = (0..10).map(|i| CensusBar::observed(i * MIN)).collect();
        bars.extend((10..20).map(|i| CensusBar::derived(i * MIN)));
        let c = census("X", "v", "1m", 60, &bars, &crypto(), at(0));
        assert_eq!(c.unflagged_sentinel_bars, 0);
        assert_eq!(c.defensibly_observed_bars(), 10);
        assert!(c.provenance_alarm().is_none());
    }

    /// Adversarial: a bar collected twice, once honestly and once with an
    /// unsupportable stamp, must not have the bad copy hidden by the good one. A bar
    /// is no more defensible than its least defensible revision.
    #[test]
    fn one_unsupportable_revision_taints_the_bar() {
        let bars = vec![
            CensusBar::observed(0),
            CensusBar::observed(MIN),
            CensusBar::unflagged_sentinel(MIN),
        ];
        let c = census("X", "v", "1m", 60, &bars, &crypto(), at(0));
        assert_eq!(c.present_bars, 2);
        assert_eq!(c.unflagged_sentinel_bars, 1);
        assert_eq!(c.defensibly_observed_bars(), 1);
        assert!(c.provenance_alarm().is_some());
    }

    /// A flagged row is an honest hole, already counted as derived, so it must not
    /// also be counted as an alarm — otherwise the fix for the bug would look like
    /// the bug.
    #[test]
    fn a_flagged_row_is_never_counted_twice() {
        let bars = vec![
            CensusBar::derived(0),
            CensusBar {
                open_ns: 0,
                knowledge_derived: true,
                unflagged_sentinel: true,
            },
        ];
        let c = census("X", "v", "1m", 60, &bars, &crypto(), at(0));
        assert_eq!(c.derived_knowledge_bars, 1);
        assert_eq!(c.unflagged_sentinel_bars, 0);
        assert_eq!(
            c.derived_knowledge_bars + c.observed_knowledge_bars,
            c.present_bars,
            "the split still partitions the series"
        );
    }

    #[test]
    fn empty_series_is_no_data_not_satisfied() {
        let c = census("X", "v", "1m", 60, &[], &crypto(), at(0));
        assert_eq!(c.present_bars, 0);
        assert_eq!(c.expected_bars, 0);
        assert_eq!(c.missing_bps, 0);
        assert_eq!(c.strict_v1, FloorVerdict::NoData);
        assert_eq!(c.paper_v1, FloorVerdict::NoData);
        assert!(!c.strict_v1.is_satisfied());
    }

    /// Adversarial: knowledge provenance may not be laundered. A backfilled bar and
    /// an observed bar of the same open time must remain distinguishable, and the
    /// backfilled one must win — the census is not allowed to flatter a series.
    #[test]
    fn derived_knowledge_is_never_laundered_into_observed() {
        let bars = vec![
            CensusBar::observed(0),
            CensusBar::derived(MIN),
            // The same bar, collected twice: once honestly, once as a backfill.
            CensusBar::observed(2 * MIN),
            CensusBar::derived(2 * MIN),
        ];
        let c = census("X", "v", "1m", 60, &bars, &crypto(), at(0));
        assert_eq!(c.present_bars, 3, "the duplicate collapses to one bar");
        assert_eq!(
            c.derived_knowledge_bars, 2,
            "the tainted duplicate counts as derived"
        );
        assert_eq!(c.observed_knowledge_bars, 1);
        assert_eq!(
            c.derived_knowledge_bars + c.observed_knowledge_bars,
            c.present_bars,
            "every bar is on exactly one side of the provenance line"
        );
    }

    /// Adversarial: a series that is entirely backfilled must not report a single
    /// observed bar. This is the `market_bars_v2` bug stated as a test — 158k rows
    /// stamped `available_time = event_time` looked observed-live and were not.
    #[test]
    fn a_wholly_backfilled_series_reports_zero_observed() {
        let bars: Vec<CensusBar> = (0..500).map(|i| CensusBar::derived(i * MIN)).collect();
        let c = census("X", "v", "1m", 60, &bars, &crypto(), at(0));
        assert_eq!(c.observed_knowledge_bars, 0);
        assert_eq!(c.derived_knowledge_bars, 500);
    }

    #[test]
    fn unsorted_input_gives_the_same_answer() {
        let mut bars = dense(200);
        bars.retain(|b| b.open_ns / MIN != 77);
        let sorted = census("X", "v", "1m", 60, &bars, &crypto(), at(0));
        let mut shuffled = bars.clone();
        // A fixed, seedless permutation: reverse, then rotate. Deterministic.
        shuffled.reverse();
        shuffled.rotate_left(53);
        let out = census("X", "v", "1m", 60, &shuffled, &crypto(), at(0));
        assert_eq!(sorted, out);
    }

    /// Re-running on an unchanged database is identical except `as_of`.
    #[test]
    fn rerun_is_identical_except_as_of() {
        let bars = dense(300);
        let a = census("X", "v", "1m", 60, &bars, &crypto(), at(0));
        let b = census("X", "v", "1m", 60, &bars, &crypto(), at(999 * MIN));
        assert_ne!(a.as_of, b.as_of);
        let a2 = CoverageCensus {
            as_of: b.as_of,
            ..a.clone()
        };
        assert_eq!(a2, b, "only as_of may differ between two runs");
    }

    #[test]
    fn session_calendar_does_not_call_a_weekend_a_gap() {
        let nyse = Calendar::Sessions {
            version: CalendarVersion::new("XNYS", "2026.1"),
            open_utc: NaiveTime::from_hms_opt(14, 30, 0).unwrap(),
            close_utc: NaiveTime::from_hms_opt(21, 0, 0).unwrap(),
            holidays: vec![NaiveDate::from_ymd_opt(2026, 1, 19).unwrap()],
        };
        // Hourly bars, Friday 15:00 UTC then Monday 15:00 UTC. Continuous would call
        // that a 71-period hole; the session calendar calls it none.
        let fri = Utc
            .with_ymd_and_hms(2026, 1, 16, 15, 0, 0)
            .unwrap()
            .timestamp_nanos_opt()
            .unwrap();
        let mon = Utc
            .with_ymd_and_hms(2026, 1, 20, 15, 0, 0)
            .unwrap()
            .timestamp_nanos_opt()
            .unwrap();
        let hour = 3600 * 1_000_000_000;
        let bars = vec![CensusBar::observed(fri), CensusBar::observed(mon)];
        let c = census("SPY", "alpaca", "1h", 3600, &bars, &nyse, at(0));
        let continuous = census("SPY", "alpaca", "1h", 3600, &bars, &crypto(), at(0));
        assert!(
            c.longest_gap_periods < continuous.longest_gap_periods,
            "a session calendar must not count closed hours as missing bars"
        );
        assert_eq!(
            continuous.longest_gap_periods,
            ((mon - fri) / hour - 1).unsigned_abs()
        );
        // 2026-01-19 is a holiday, so only the 2026-01-16 and 2026-01-20 sessions
        // contribute; the closed hours between them are not expected bars.
        assert!(c.longest_gap_periods <= 12);
    }

    #[test]
    fn floors_are_judged_on_continuous_span_not_total_bars() {
        // Two years of hourly bars, but every 100th bar missing: plenty of bars,
        // no continuous span. strict_v1 must not be satisfied.
        let hour = 3600 * 1_000_000_000;
        let bars: Vec<CensusBar> = (0..24 * 365 * 2)
            .filter(|i| i % 100 != 0)
            .map(|i| CensusBar::observed(i * hour))
            .collect();
        let c = census("X", "v", "1h", 3600, &bars, &crypto(), at(0));
        assert!(c.present_bars > 17_000);
        assert!(!c.strict_v1.is_satisfied());
        assert!(
            !c.paper_v1.is_satisfied(),
            "a 99-bar run is not a year of history"
        );
        assert!(matches!(c.strict_v1, FloorVerdict::Short { .. }));
    }

    #[test]
    fn five_years_continuous_satisfies_strict_v1() {
        let hour = 3600 * 1_000_000_000;
        let bars: Vec<CensusBar> = (0..24 * 1830)
            .map(|i| CensusBar::observed(i * hour))
            .collect();
        let c = census("X", "v", "1h", 3600, &bars, &crypto(), at(0));
        assert!(c.continuous_days() >= STRICT_V1_FLOOR_DAYS);
        assert_eq!(c.strict_v1, FloorVerdict::Satisfied);
        assert_eq!(c.paper_v1, FloorVerdict::Satisfied);
    }

    #[test]
    fn shortfall_is_stated_in_days() {
        let hour = 3600 * 1_000_000_000;
        // 30 days of hourly bars.
        let bars: Vec<CensusBar> = (0..24 * 30)
            .map(|i| CensusBar::observed(i * hour))
            .collect();
        let c = census("BTC-USD", "coinbase", "1h", 3600, &bars, &crypto(), at(0));
        match c.paper_v1 {
            FloorVerdict::Short { shortfall_days, .. } => {
                assert_eq!(shortfall_days, PAPER_V1_FLOOR_DAYS - c.continuous_days());
            }
            other => panic!("expected a shortfall, got {other:?}"),
        }
    }

    #[test]
    fn staleness_measures_from_the_bar_close() {
        let last = 1_000 * MIN;
        let s = staleness(Some(last), 60, at(last + 5 * MIN)).unwrap();
        // The bar closes one period after it opens, so five minutes after the open
        // is four minutes after the close.
        assert_eq!(s.num_minutes(), 4);
        assert!(staleness(None, 60, at(0)).is_none());
    }
}
