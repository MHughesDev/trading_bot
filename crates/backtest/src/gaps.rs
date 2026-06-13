//! Pure coverage / gap analysis over per-day bar counts.
//!
//! Day-level granularity keeps the ClickHouse query cheap (one GROUP BY) and
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
            Self::Weekday24h | Self::WeekdaySession => {
                !matches!(day.weekday(), Weekday::Sat | Weekday::Sun)
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
        missing_ranges: merge_days(&missing_days),
    }
}

/// Merges consecutive missing days into contiguous UTC ranges.
fn merge_days(days: &[NaiveDate]) -> Vec<MissingRange> {
    let mut out: Vec<MissingRange> = Vec::new();
    for &day in days {
        let start = day.and_hms_opt(0, 0, 0).expect("midnight").and_utc();
        let end = start + Duration::days(1);
        match out.last_mut() {
            // Extend when adjacent (weekend holes inside a weekday-only
            // schedule still merge: the gap range simply spans the weekend).
            Some(prev) if (day - prev.to.date_naive()).num_days().abs() <= 2 => {
                prev.to = end;
            }
            _ => out.push(MissingRange { from: start, to: end }),
        }
    }
    out
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
