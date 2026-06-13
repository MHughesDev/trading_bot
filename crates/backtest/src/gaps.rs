//! Pure coverage / gap analysis over per-day bar counts.
//!
//! Day-level granularity keeps the `ClickHouse` query cheap (one GROUP BY) and
//! is the natural unit for REST backfills.  A day counts as "covered" when it
//! holds at least [`COVERAGE_THRESHOLD_PCT`] percent of the bars its trading
//! schedule predicts; sparse days are re-collected rather than trusted.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc, Weekday};
use domain::payloads::bar::Timeframe;

use crate::types::{DataCoverage, MissingRange, TimeframeExt};

/// Minimum fraction of expected bars for a day to count as covered.
pub const COVERAGE_THRESHOLD_PCT: u64 = 60;

/// Trading schedule shape used for expectations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleKind {
    /// 24/7 markets (crypto).
    Continuous,
    /// 24/5 markets (FX) — weekdays only, full day.
    Weekday24h,
    /// Session markets (equities/ETFs/options) — weekdays, ~6.5 h sessions.
    WeekdaySession,
}

impl ScheduleKind {
    pub fn for_asset_class(asset_class: &str) -> Self {
        match asset_class {
            "equity" | "etf" | "option" | "bond" => Self::WeekdaySession,
            "fx" | "futures_expiring" => Self::Weekday24h,
            _ => Self::Continuous,
        }
    }

    fn trades_on(self, day: NaiveDate) -> bool {
        match self {
            Self::Continuous => true,
            // FX is 24/5 and trades through most US public holidays, so only
            // weekends are excluded.
            Self::Weekday24h => !matches!(day.weekday(), Weekday::Sat | Weekday::Sun),
            // Session markets (equities/ETFs/options) observe the exchange
            // holiday calendar in addition to weekends, so closed days aren't
            // counted as missing data or wasted on collection attempts (#13).
            Self::WeekdaySession => {
                !matches!(day.weekday(), Weekday::Sat | Weekday::Sun) && !is_us_market_holiday(day)
            }
        }
    }

    fn session_seconds(self) -> u64 {
        match self {
            Self::Continuous | Self::Weekday24h => 86_400,
            // US cash session 09:30–16:00.
            Self::WeekdaySession => 23_400,
        }
    }
}

/// NYSE/Nasdaq full-day market holidays (observed dates), `(year, month, day)`.
///
/// Session markets are closed on these dates, so they are neither expected in
/// coverage nor worth a collection attempt.  Half-days (e.g. the day after
/// Thanksgiving) still trade and are intentionally omitted — they hold a
/// partial session, which the coverage threshold already tolerates.  Extend
/// this table as the calendar is published for future years.
const US_MARKET_HOLIDAYS: &[(i32, u32, u32)] = &[
    // 2023
    (2023, 1, 2),
    (2023, 1, 16),
    (2023, 2, 20),
    (2023, 4, 7),
    (2023, 5, 29),
    (2023, 6, 19),
    (2023, 7, 4),
    (2023, 9, 4),
    (2023, 11, 23),
    (2023, 12, 25),
    // 2024
    (2024, 1, 1),
    (2024, 1, 15),
    (2024, 2, 19),
    (2024, 3, 29),
    (2024, 5, 27),
    (2024, 6, 19),
    (2024, 7, 4),
    (2024, 9, 2),
    (2024, 11, 28),
    (2024, 12, 25),
    // 2025
    (2025, 1, 1),
    (2025, 1, 20),
    (2025, 2, 17),
    (2025, 4, 18),
    (2025, 5, 26),
    (2025, 6, 19),
    (2025, 7, 4),
    (2025, 9, 1),
    (2025, 11, 27),
    (2025, 12, 25),
    // 2026
    (2026, 1, 1),
    (2026, 1, 19),
    (2026, 2, 16),
    (2026, 4, 3),
    (2026, 5, 25),
    (2026, 6, 19),
    (2026, 7, 3),
    (2026, 9, 7),
    (2026, 11, 26),
    (2026, 12, 25),
    // 2027
    (2027, 1, 1),
    (2027, 1, 18),
    (2027, 2, 15),
    (2027, 3, 26),
    (2027, 5, 31),
    (2027, 6, 18),
    (2027, 7, 5),
    (2027, 9, 6),
    (2027, 11, 25),
    (2027, 12, 24),
];

/// Whether `day` is a full-day US equity-market holiday (see
/// [`US_MARKET_HOLIDAYS`]).
fn is_us_market_holiday(day: NaiveDate) -> bool {
    let key = (day.year(), day.month(), day.day());
    US_MARKET_HOLIDAYS.contains(&key)
}

/// Bars a fully covered trading day should hold.
pub fn expected_bars_per_day(timeframe: Timeframe, schedule: ScheduleKind) -> u64 {
    (schedule.session_seconds() / timeframe.seconds()).max(1)
}

/// Computes coverage for `[from, to)` given per-day bar counts from storage.
pub fn analyze(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    counts: &HashMap<NaiveDate, u64>,
    timeframe: Timeframe,
    schedule: ScheduleKind,
) -> DataCoverage {
    let per_day = expected_bars_per_day(timeframe, schedule);
    let threshold = per_day * COVERAGE_THRESHOLD_PCT / 100;

    let mut expected = 0u64;
    let mut present = 0u64;
    let mut missing_days: Vec<NaiveDate> = Vec::new();

    let mut day = from.date_naive();
    // `to` is exclusive: a window ending at midnight does not include that day.
    let last = (to - Duration::nanoseconds(1)).date_naive();
    while day <= last {
        if schedule.trades_on(day) {
            expected += per_day;
            let have = counts.get(&day).copied().unwrap_or(0);
            present += have.min(per_day);
            if have < threshold.max(1) {
                missing_days.push(day);
            }
        }
        day += Duration::days(1);
    }

    DataCoverage {
        expected_bars: expected,
        present_bars: present,
        collected_bars: 0,
        missing_ranges: merge_days(&missing_days, schedule),
    }
}

/// Merges consecutive missing days into contiguous UTC ranges.
///
/// Two missing days are bridged into one range only when every calendar day
/// between them is a *non-trading* day for the schedule (weekend or holiday).
/// On continuous (24/7) markets nothing is non-trading, so a single present
/// day between two gaps keeps them as two ranges instead of swallowing the
/// present day (#14).  On session markets the intervening weekend/holiday is
/// skipped, so Fri+Mon still merge.
fn merge_days(days: &[NaiveDate], schedule: ScheduleKind) -> Vec<MissingRange> {
    let mut out: Vec<MissingRange> = Vec::new();
    for &day in days {
        let start = day.and_hms_opt(0, 0, 0).expect("midnight").and_utc();
        let end = start + Duration::days(1);
        match out.last_mut() {
            // `prev.to.date_naive()` is the first calendar day after the last
            // merged gap day; bridge only if [that, day) is all non-trading.
            Some(prev) if all_non_trading_between(prev.to.date_naive(), day, schedule) => {
                prev.to = end;
            }
            _ => out.push(MissingRange {
                from: start,
                to: end,
            }),
        }
    }
    out
}

/// True when every calendar day in the half-open range `[from, to)` is a
/// non-trading day for `schedule` (so an empty range is trivially bridgeable —
/// i.e. genuinely adjacent missing days always merge).
fn all_non_trading_between(from: NaiveDate, to: NaiveDate, schedule: ScheduleKind) -> bool {
    let mut day = from;
    while day < to {
        if schedule.trades_on(day) {
            return false;
        }
        day += Duration::days(1);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn t(s: &str) -> DateTime<Utc> {
        d(s).and_hms_opt(0, 0, 0).unwrap().and_utc()
    }

    #[test]
    fn expected_counts_per_schedule() {
        assert_eq!(
            expected_bars_per_day(Timeframe::Minutes1, ScheduleKind::Continuous),
            1440
        );
        assert_eq!(
            expected_bars_per_day(Timeframe::Minutes1, ScheduleKind::WeekdaySession),
            390
        );
        assert_eq!(
            expected_bars_per_day(Timeframe::Daily, ScheduleKind::Continuous),
            1
        );
    }

    #[test]
    fn full_coverage_has_no_gaps() {
        let mut counts = HashMap::new();
        counts.insert(d("2026-01-05"), 1440);
        counts.insert(d("2026-01-06"), 1440);
        let cov = analyze(
            t("2026-01-05"),
            t("2026-01-07"),
            &counts,
            Timeframe::Minutes1,
            ScheduleKind::Continuous,
        );
        assert!(cov.missing_ranges.is_empty());
        assert_eq!(cov.expected_bars, 2880);
        assert_eq!(cov.present_bars, 2880);
    }

    #[test]
    fn missing_and_sparse_days_become_ranges() {
        let mut counts = HashMap::new();
        counts.insert(d("2026-01-01"), 1440);
        // Jan 2 missing entirely, Jan 3 sparse (10%), Jan 4 full.
        counts.insert(d("2026-01-03"), 144);
        counts.insert(d("2026-01-04"), 1440);
        let cov = analyze(
            t("2026-01-01"),
            t("2026-01-04"),
            &counts,
            Timeframe::Minutes1,
            ScheduleKind::Continuous,
        );
        assert_eq!(cov.missing_ranges.len(), 1);
        assert_eq!(cov.missing_ranges[0].from, t("2026-01-02"));
        assert_eq!(cov.missing_ranges[0].to, t("2026-01-04"));
    }

    #[test]
    fn weekends_are_not_expected_for_session_markets() {
        // 2026-01-03/04 are Sat/Sun.
        let counts = HashMap::new();
        let cov = analyze(
            t("2026-01-03"),
            t("2026-01-04"),
            &counts,
            Timeframe::Minutes1,
            ScheduleKind::WeekdaySession,
        );
        assert_eq!(cov.expected_bars, 0);
        assert!(cov.missing_ranges.is_empty());
    }

    #[test]
    fn continuous_market_does_not_swallow_a_present_day() {
        // Jan 1 missing, Jan 2 PRESENT (full), Jan 3 missing on a 24/7 market.
        // The present day must split the gaps into two ranges (#14).
        let mut counts = HashMap::new();
        counts.insert(d("2026-01-02"), 1440);
        let cov = analyze(
            t("2026-01-01"),
            t("2026-01-04"),
            &counts,
            Timeframe::Minutes1,
            ScheduleKind::Continuous,
        );
        assert_eq!(
            cov.missing_ranges.len(),
            2,
            "present day must not be bridged"
        );
        assert_eq!(cov.missing_ranges[0].from, t("2026-01-01"));
        assert_eq!(cov.missing_ranges[0].to, t("2026-01-02"));
        assert_eq!(cov.missing_ranges[1].from, t("2026-01-03"));
        assert_eq!(cov.missing_ranges[1].to, t("2026-01-04"));
    }

    #[test]
    fn session_market_skips_holidays() {
        // 2026-01-19 is MLK Day (a Monday) — a session-market holiday that must
        // not be counted as expected or reported as a gap.
        let mlk = d("2026-01-19");
        assert_eq!(mlk.weekday(), Weekday::Mon);
        let counts = HashMap::new();
        let cov = analyze(
            mlk.and_hms_opt(0, 0, 0).unwrap().and_utc(),
            (mlk + Duration::days(1))
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc(),
            &counts,
            Timeframe::Minutes1,
            ScheduleKind::WeekdaySession,
        );
        assert_eq!(cov.expected_bars, 0, "holiday is not a trading day");
        assert!(cov.missing_ranges.is_empty());
    }

    #[test]
    fn session_gaps_bridge_across_a_holiday() {
        // Fri 2026-01-16 missing and Tue 2026-01-20 missing, with the weekend
        // plus MLK Monday (01-19) in between — one merged range.
        let counts = HashMap::new();
        let cov = analyze(
            t("2026-01-16"),
            t("2026-01-21"),
            &counts,
            Timeframe::Minutes1,
            ScheduleKind::WeekdaySession,
        );
        assert_eq!(cov.missing_ranges.len(), 1);
        assert_eq!(cov.missing_ranges[0].from, t("2026-01-16"));
        assert_eq!(cov.missing_ranges[0].to, t("2026-01-21"));
    }

    #[test]
    fn weekday_gaps_merge_across_weekends() {
        // Fri 2026-01-02 missing, Mon 2026-01-05 missing → one range.
        let counts = HashMap::new();
        let cov = analyze(
            t("2026-01-02"),
            t("2026-01-06"),
            &counts,
            Timeframe::Minutes1,
            ScheduleKind::WeekdaySession,
        );
        assert_eq!(cov.missing_ranges.len(), 1);
        assert_eq!(cov.missing_ranges[0].from, t("2026-01-02"));
        assert_eq!(cov.missing_ranges[0].to, t("2026-01-06"));
    }
}
