//! The eight §7 checks (DATA-005 §7, P-2.1).
//!
//! Each returns a [`CheckResult`] carrying what it found, never a verdict about
//! the series. Grading is [`super::grade`]'s job, and keeping them apart is what
//! lets a threshold change re-grade history without re-running the battery.
//!
//! ## Three states, not two
//!
//! [`Outcome::NotApplicable`] exists because several of these checks cannot run
//! on every series — cross-source needs a second venue, survivorship needs
//! membership history. A check that cannot run and reports `Pass` is the worst
//! possible answer: it converts an absence of evidence into evidence, in a
//! grading function whose whole purpose is to say how much the data can be
//! trusted.
//!
//! The same reasoning as the platform's five-state self-monitor, applied one
//! layer down.

use serde::{Deserialize, Serialize};

use super::config::QcConfig;

/// One bar, as the battery sees it.
///
/// Deliberately self-contained rather than borrowing the storage row type: the
/// battery must be exercisable against planted fixtures with no database, and a
/// check that can only be tested through ClickHouse is a check nobody tests.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct QcBar {
    /// Bar open time, Unix nanoseconds.
    pub event_ns: i64,
    /// When the platform learned it. INV-01: never before `event_ns`.
    pub knowledge_ns: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    /// Whether this bar has been revised since first publication.
    pub revised: bool,
}

/// What a check concluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Fail,
    /// The check does not apply to this series — no second venue, no membership
    /// history. **Never treated as a pass.**
    NotApplicable,
    /// The check applies and could not run: the metadata it needs is absent.
    /// Also never a pass — see the module docs.
    InsufficientMetadata,
}

impl Outcome {
    #[must_use]
    pub const fn is_pass(self) -> bool {
        matches!(self, Self::Pass)
    }

    /// Whether this outcome says anything about the data at all.
    ///
    /// `NotApplicable` and `InsufficientMetadata` do not, and a grade that
    /// counts them as evidence is a grade built on silence.
    #[must_use]
    pub const fn is_evidence(self) -> bool {
        matches!(self, Self::Pass | Self::Fail)
    }
}

/// Which check ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Coverage,
    Revisions,
    BadTicks,
    StaleRuns,
    Volume,
    Timestamps,
    CrossSource,
    Survivorship,
}

impl CheckKind {
    pub const ALL: &'static [Self] = &[
        Self::Coverage,
        Self::Revisions,
        Self::BadTicks,
        Self::StaleRuns,
        Self::Volume,
        Self::Timestamps,
        Self::CrossSource,
        Self::Survivorship,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Coverage => "coverage",
            Self::Revisions => "revisions",
            Self::BadTicks => "bad_ticks",
            Self::StaleRuns => "stale_runs",
            Self::Volume => "volume",
            Self::Timestamps => "timestamps",
            Self::CrossSource => "cross_source",
            Self::Survivorship => "survivorship",
        }
    }
}

/// One check's finding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub kind: CheckKind,
    pub outcome: Outcome,
    /// What was found, in a sentence somebody can act on. A grade with no
    /// reasons is unreviewable and unappealable, so this is never empty.
    pub detail: String,
    /// Bar indices the check objected to, capped. Kept so a reviewer can look at
    /// the bars rather than trusting the summary.
    pub offending: Vec<usize>,
    /// The check's headline number, for the grade function and for trending.
    ///
    /// `None` when the check could not run. Deliberately not `NaN`: a NaN
    /// sentinel compares unequal to itself, so a report containing one is never
    /// equal to a re-run of itself, and `serde_json` writes it as `null` and
    /// then refuses to read it back as a float. Both cost a test each to find.
    pub metric: Option<f64>,
}

/// Cap on `offending`, so one broken series cannot produce a megabyte of report.
const MAX_OFFENDING: usize = 50;

fn result(
    kind: CheckKind,
    outcome: Outcome,
    metric: Option<f64>,
    detail: impl Into<String>,
    offending: Vec<usize>,
) -> CheckResult {
    let mut offending = offending;
    offending.truncate(MAX_OFFENDING);
    CheckResult { kind, outcome, detail: detail.into(), offending, metric }
}

// ── 1. coverage ──────────────────────────────────────────────────────────────

/// Expected versus present, gap count, longest gap, percent missing.
///
/// `expected` comes from the census (P-0.1) rather than being derived here: the
/// number of bars a venue *should* have published over a window depends on its
/// session calendar, and a checker that guesses it will be wrong on every
/// holiday.
#[must_use]
pub fn coverage(bars: &[QcBar], expected: u64, cfg: &QcConfig, period_ns: i64) -> CheckResult {
    if expected == 0 {
        return result(
            CheckKind::Coverage,
            Outcome::InsufficientMetadata,
            None,
            "the expected bar count for this window is zero, so coverage cannot be computed; \
             the census supplies it and has not run for this series",
            vec![],
        );
    }

    let present = bars.len() as u64;
    let missing = expected.saturating_sub(present);
    let share = missing as f64 / expected as f64;

    // Gaps between consecutive bars, in whole periods.
    let mut gaps: Vec<(usize, i64)> = Vec::new();
    for (i, w) in bars.windows(2).enumerate() {
        let delta = w[1].event_ns - w[0].event_ns;
        if period_ns > 0 && delta > period_ns {
            gaps.push((i + 1, delta / period_ns - 1));
        }
    }
    let longest = gaps.iter().map(|(_, n)| *n).max().unwrap_or(0);

    let outcome = if share >= cfg.coverage.d_min_missing {
        Outcome::Fail
    } else {
        Outcome::Pass
    };

    result(
        CheckKind::Coverage,
        outcome,
        Some(share),
        format!(
            "{present} of {expected} expected bars present ({:.3}% missing); {} gap(s), longest {longest} bar(s)",
            share * 100.0,
            gaps.len()
        ),
        gaps.iter().map(|(i, _)| *i).collect(),
    )
}

// ── 2. revisions ─────────────────────────────────────────────────────────────

/// How many bars in the window have been revised since first publication.
///
/// Not a failure on its own — venues legitimately correct data — but a revised
/// bar inside a backtest window is a bar the strategy could not have seen in the
/// form it was tested against, so the count belongs on the report.
#[must_use]
pub fn revisions(bars: &[QcBar]) -> CheckResult {
    let idx: Vec<usize> = bars
        .iter()
        .enumerate()
        .filter(|(_, b)| b.revised)
        .map(|(i, _)| i)
        .collect();
    let share = if bars.is_empty() {
        0.0
    } else {
        idx.len() as f64 / bars.len() as f64
    };

    result(
        CheckKind::Revisions,
        // Reported, never failed on: the grade function decides what a revision
        // rate means, and it means different things at different grades.
        Outcome::Pass,
        Some(share),
        format!("{} of {} bars revised since first publication", idx.len(), bars.len()),
        idx,
    )
}

// ── 3. bad ticks ─────────────────────────────────────────────────────────────

/// Impossible bars, and spikes that reverse.
///
/// Two families in one check because the spec groups them: a price at or below
/// zero or an OHLC ordering violation is *structurally* impossible, while a
/// large move that undoes itself next bar is statistically a print. The first
/// needs no threshold; the second needs two.
#[must_use]
pub fn bad_ticks(bars: &[QcBar], cfg: &QcConfig) -> CheckResult {
    let mut offending = Vec::new();
    let mut structural = 0usize;

    for (i, b) in bars.iter().enumerate() {
        let nonpositive = b.open <= 0.0 || b.high <= 0.0 || b.low <= 0.0 || b.close <= 0.0;
        // The spec's phrasing: `H < max(O,C)` or `L > min(O,C)`.
        let misordered = b.high < b.open.max(b.close) || b.low > b.open.min(b.close);
        if nonpositive || misordered || !b.close.is_finite() {
            structural += 1;
            offending.push(i);
        }
    }

    // Rolling-sigma spikes that reverse.
    let returns: Vec<f64> = bars
        .windows(2)
        .map(|w| {
            if w[0].close > 0.0 && w[1].close > 0.0 {
                (w[1].close / w[0].close).ln()
            } else {
                0.0
            }
        })
        .collect();

    let win = cfg.bad_ticks.sigma_window.max(2);
    let mut reversing = 0usize;
    if returns.len() > win + 1 {
        for i in win..returns.len() - 1 {
            let window = &returns[i - win..i];
            let mean = window.iter().sum::<f64>() / window.len() as f64;
            let var = window.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / window.len() as f64;
            let sd = var.sqrt();
            if sd <= 0.0 {
                continue;
            }
            let (here, next) = (returns[i], returns[i + 1]);
            let reversed = -next / here;
            if here.abs() > cfg.bad_ticks.sigma_multiple * sd
                && here.signum() != next.signum()
                && reversed >= cfg.bad_ticks.reversal_fraction
            {
                reversing += 1;
                offending.push(i + 1);
            }
        }
    }

    let total = structural + reversing;
    offending.sort_unstable();
    offending.dedup();

    result(
        CheckKind::BadTicks,
        if total == 0 { Outcome::Pass } else { Outcome::Fail },
        Some(total as f64),
        format!(
            "{structural} structurally impossible bar(s), {reversing} spike(s) beyond \
             {:.1}σ reversing ≥ {:.0}%",
            cfg.bad_ticks.sigma_multiple,
            cfg.bad_ticks.reversal_fraction * 100.0
        ),
        offending,
    )
}

// ── 4. stale runs ────────────────────────────────────────────────────────────

/// Runs of identical closes on zero volume.
///
/// Both conditions, not either. A flat close on real volume is a quiet market
/// and a normal thing; a flat close on *no* volume for many bars in a row is a
/// feed republishing its last value.
#[must_use]
pub fn stale_runs(bars: &[QcBar], cfg: &QcConfig, timeframe: &str) -> CheckResult {
    let threshold = cfg.stale_run_for(timeframe);
    let mut offending = Vec::new();
    let mut runs = 0usize;
    let mut longest = 0usize;

    let mut run_start = 0usize;
    let mut run_len = 1usize;
    for i in 1..bars.len() {
        let same = (bars[i].close - bars[i - 1].close).abs() <= f64::EPSILON
            && bars[i].volume <= 0.0
            && bars[i - 1].volume <= 0.0;
        if same {
            run_len += 1;
        } else {
            if run_len >= threshold {
                runs += 1;
                longest = longest.max(run_len);
                offending.push(run_start);
            }
            run_start = i;
            run_len = 1;
        }
    }
    if run_len >= threshold {
        runs += 1;
        longest = longest.max(run_len);
        offending.push(run_start);
    }

    result(
        CheckKind::StaleRuns,
        if runs == 0 { Outcome::Pass } else { Outcome::Fail },
        Some(longest as f64),
        format!(
            "{runs} run(s) of ≥ {threshold} identical closes on zero volume; longest {longest} bars"
        ),
        offending,
    )
}

// ── 5. volume ────────────────────────────────────────────────────────────────

/// Zero-volume share, and spikes against a rolling median.
#[must_use]
pub fn volume(bars: &[QcBar], cfg: &QcConfig) -> CheckResult {
    if bars.is_empty() {
        return result(
            CheckKind::Volume,
            Outcome::InsufficientMetadata,
            None,
            "no bars in the window",
            vec![],
        );
    }

    let zero: Vec<usize> = bars
        .iter()
        .enumerate()
        .filter(|(_, b)| b.volume <= 0.0)
        .map(|(i, _)| i)
        .collect();
    let zero_share = zero.len() as f64 / bars.len() as f64;

    let win = cfg.volume.median_window.max(3);
    let mut spikes = Vec::new();
    if bars.len() > win {
        for i in win..bars.len() {
            let mut window: Vec<f64> = bars[i - win..i].iter().map(|b| b.volume).collect();
            window.sort_by(f64::total_cmp);
            let median = window[window.len() / 2];
            if median > 0.0 && bars[i].volume > median * cfg.volume.spike_multiple {
                spikes.push(i);
            }
        }
    }

    let failed = zero_share > cfg.volume.max_zero_share || !spikes.is_empty();
    let mut offending = zero.clone();
    offending.extend(&spikes);
    offending.sort_unstable();
    offending.dedup();

    result(
        CheckKind::Volume,
        if failed { Outcome::Fail } else { Outcome::Pass },
        Some(zero_share),
        format!(
            "{:.2}% zero-volume bars (limit {:.2}%); {} spike(s) over {:.0}× the rolling median",
            zero_share * 100.0,
            cfg.volume.max_zero_share * 100.0,
            spikes.len(),
            cfg.volume.spike_multiple
        ),
        offending,
    )
}

// ── 6. timestamps ────────────────────────────────────────────────────────────

/// `knowledge_ns ≥ event_ns`, monotonic ordering, no duplicates.
///
/// **A failure here forces grade D regardless of everything else**, and this is
/// the check that earns that. A series whose `knowledge_time` is wrong is a
/// series where every point-in-time filter silently reads the future, and the
/// census found that condition across every row of `market_bars_v2`.
///
/// # The rule this enforces is weaker than the one it should
///
/// `event_ns` is the bar's **open**. The bound below is therefore
/// `knowledge ≥ open`, when the honest bound is `knowledge ≥ close + the source's
/// declared lag` — [`crate::bar::earliest_knowable`] states that one, and only the
/// write path enforces it. A bar stamped as knowable halfway through its own period
/// passes here.
///
/// That slack is measured rather than assumed: across all 170,924 rows of
/// `market_bar` on 2026-09-22, zero have `knowledge_time` earlier than
/// `event_time + period_secs`, and the tightest series (BTC-USD/kraken/1m over
/// `kraken_ws`) sits exactly on the boundary. A measured zero — but closing the gap
/// needs the period inside this signature, which is a change to every caller and is
/// not made here.
///
/// The old census hid this by accident: `market_bars_v2` stored the bar's *close* in
/// `event_time`, so feeding it into a field meaning *open* gave the stronger bound
/// for the wrong reason. Reading the canonical `market_bar` fixes the column meaning
/// and loses the accident with it.
///
/// # What counts as a duplicate
///
/// Two bars sharing an `event_ns` are a **restatement**, not a duplicate, when the
/// later one carries a strictly greater `knowledge_ns`: a point-in-time read picks
/// exactly one of them for any cutoff, which is the whole mechanism by which a
/// vendor correction is representable at all. They are a duplicate only when they
/// share a `knowledge_ns` too — then no cutoff can separate them and a PIT read has
/// to choose arbitrarily, which is the defect this clause is for.
///
/// This distinction had nothing to bite on while the census read `market_bars_v2`.
/// That table keeps `available_time` out of its sorting key, so a restatement was
/// merged into its own original before anyone could read it and every series looked
/// revision-free. Against the canonical `market_bar` (ADR-D-29) restatements survive
/// and are ordinary: measured 2026-09-22, BTC-USD/kraken/1m alone has 18 event times
/// carrying more than one row — 73 restatements in all, one bar of it restated 54
/// times — every group with a distinct `knowledge_time` and a distinct
/// `revision_seq`. Counting those as duplicates forced a D on the platform's
/// best-attested series for doing precisely what the canonical table exists to do.
#[must_use]
pub fn timestamps(bars: &[QcBar]) -> CheckResult {
    let mut offending = Vec::new();
    let mut before_event = 0usize;
    let mut out_of_order = 0usize;
    let mut duplicates = 0usize;

    for (i, b) in bars.iter().enumerate() {
        if b.knowledge_ns < b.event_ns {
            before_event += 1;
            offending.push(i);
        }
        if i > 0 {
            let prev = &bars[i - 1];
            if b.event_ns < prev.event_ns {
                out_of_order += 1;
                offending.push(i);
            } else if b.event_ns == prev.event_ns && b.knowledge_ns <= prev.knowledge_ns {
                duplicates += 1;
                offending.push(i);
            }
        }
    }

    let total = before_event + out_of_order + duplicates;
    offending.sort_unstable();
    offending.dedup();

    result(
        CheckKind::Timestamps,
        if total == 0 { Outcome::Pass } else { Outcome::Fail },
        Some(total as f64),
        format!(
            "{before_event} bar(s) with knowledge_time before event_time, {out_of_order} \
             out of order, {duplicates} duplicate timestamp(s)"
        ),
        offending,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000_000_000;

    fn clean(n: usize) -> Vec<QcBar> {
        (0..n)
            .map(|i| {
                let p = 100.0 + (i as f64 * 0.37).sin() * 0.5;
                QcBar {
                    event_ns: i as i64 * MIN,
                    knowledge_ns: i as i64 * MIN + 1_000,
                    open: p,
                    high: p + 0.2,
                    low: p - 0.2,
                    close: p + 0.05,
                    volume: 1_000.0 + i as f64,
                    revised: false,
                }
            })
            .collect()
    }

    fn cfg() -> QcConfig {
        QcConfig::default()
    }

    // ── the three-state rule ─────────────────────────────────────────────────

    /// A check that cannot run and reports `Pass` converts an absence of
    /// evidence into evidence, inside the function whose job is to say how much
    /// the data can be trusted.
    #[test]
    fn an_outcome_that_cannot_run_is_not_a_pass_and_is_not_evidence() {
        for o in [Outcome::NotApplicable, Outcome::InsufficientMetadata] {
            assert!(!o.is_pass(), "{o:?}");
            assert!(!o.is_evidence(), "{o:?}");
        }
        assert!(Outcome::Pass.is_evidence());
        assert!(Outcome::Fail.is_evidence());
    }

    #[test]
    fn every_check_kind_has_a_distinct_stable_name() {
        let names: std::collections::BTreeSet<_> =
            CheckKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(names.len(), CheckKind::ALL.len());
        assert_eq!(CheckKind::ALL.len(), 8, "DATA-005 §7 specifies eight checks");
    }

    // ── each check fires on its own fixture and is silent on a clean one ─────

    #[test]
    fn coverage_is_silent_on_a_complete_series_and_fires_on_a_gap() {
        let bars = clean(100);
        let ok = coverage(&bars, 100, &cfg(), MIN);
        assert_eq!(ok.outcome, Outcome::Pass, "{}", ok.detail);
        assert!(ok.metric.unwrap().abs() < f64::EPSILON);

        // 90 of 100 present is 10% missing, past the 5% D line.
        let short = coverage(&clean(90), 100, &cfg(), MIN);
        assert_eq!(short.outcome, Outcome::Fail);
        assert!((short.metric.unwrap() - 0.10).abs() < 1e-9);
        assert!(short.detail.contains("10.000% missing"), "{}", short.detail);
    }

    /// The census supplies the expected count; guessing it would be wrong on
    /// every holiday.
    #[test]
    fn coverage_without_an_expected_count_reports_insufficient_metadata() {
        let r = coverage(&clean(10), 0, &cfg(), MIN);
        assert_eq!(r.outcome, Outcome::InsufficientMetadata);
        assert!(r.detail.contains("census"), "{}", r.detail);
    }

    #[test]
    fn coverage_measures_the_longest_gap_in_bars() {
        let mut bars = clean(50);
        // Remove ten bars from the middle, leaving one nine-bar hole.
        bars.drain(20..30);
        let r = coverage(&bars, 50, &cfg(), MIN);
        assert!(r.detail.contains("longest 10 bar(s)"), "{}", r.detail);
        assert_eq!(r.offending, vec![20]);
    }

    #[test]
    fn revisions_counts_without_failing() {
        let mut bars = clean(100);
        for b in bars.iter_mut().take(7) {
            b.revised = true;
        }
        let r = revisions(&bars);
        assert_eq!(r.outcome, Outcome::Pass, "a revision is reported, never failed on");
        assert!((r.metric.unwrap() - 0.07).abs() < 1e-9);
        assert_eq!(r.offending.len(), 7);
    }

    #[test]
    fn bad_ticks_catches_a_structurally_impossible_bar() {
        let mut bars = clean(60);
        bars[30].high = bars[30].close - 1.0; // H < max(O,C)
        let r = bad_ticks(&bars, &cfg());
        assert_eq!(r.outcome, Outcome::Fail);
        assert!(r.offending.contains(&30), "{:?}", r.offending);
        assert!(r.detail.contains("1 structurally impossible"), "{}", r.detail);
    }

    #[test]
    fn bad_ticks_catches_a_non_positive_price() {
        let mut bars = clean(60);
        bars[10].low = 0.0;
        assert_eq!(bad_ticks(&bars, &cfg()).outcome, Outcome::Fail);
    }

    /// A large move that undoes itself is a print; one that persists is a market.
    #[test]
    fn bad_ticks_catches_a_reversing_spike_and_ignores_a_persistent_move() {
        let mut bars = clean(200);
        for (i, b) in bars.iter_mut().enumerate() {
            let p = 100.0 + (i as f64 * 0.01);
            b.open = p;
            b.high = p + 0.05;
            b.low = p - 0.05;
            b.close = p;
        }
        let mut reverting = bars.clone();
        reverting[120].close = 140.0; // spike
        reverting[120].high = 140.1;
        let r = bad_ticks(&reverting, &cfg());
        assert_eq!(r.outcome, Outcome::Fail, "{}", r.detail);
        assert!(r.detail.contains("1 spike"), "{}", r.detail);

        // A move of the same size that stays is not a bad tick.
        let mut persistent = bars.clone();
        for b in persistent.iter_mut().skip(120) {
            b.open += 40.0;
            b.high += 40.0;
            b.low += 40.0;
            b.close += 40.0;
        }
        let r = bad_ticks(&persistent, &cfg());
        assert!(r.detail.contains("0 spike"), "a sustained move is not a print: {}", r.detail);
    }

    #[test]
    fn bad_ticks_is_silent_on_a_clean_series() {
        assert_eq!(bad_ticks(&clean(300), &cfg()).outcome, Outcome::Pass);
    }

    /// Both conditions, not either: a flat close on real volume is a quiet
    /// market.
    #[test]
    fn stale_runs_needs_flat_closes_and_zero_volume_together() {
        let mut flat_with_volume = clean(100);
        for b in flat_with_volume.iter_mut().skip(10).take(60) {
            b.close = 100.0;
        }
        assert_eq!(
            stale_runs(&flat_with_volume, &cfg(), "1m").outcome,
            Outcome::Pass,
            "flat closes on real volume are a quiet market"
        );

        let mut republished = clean(100);
        for b in republished.iter_mut().skip(10).take(40) {
            b.close = 100.0;
            b.volume = 0.0;
        }
        let r = stale_runs(&republished, &cfg(), "1m");
        assert_eq!(r.outcome, Outcome::Fail, "{}", r.detail);
        assert!(r.metric.unwrap() >= 30.0, "the 1m threshold is 30 bars: {}", r.detail);
    }

    #[test]
    fn a_stale_run_at_the_very_end_of_the_series_is_still_caught() {
        let mut bars = clean(100);
        for b in bars.iter_mut().skip(60) {
            b.close = 100.0;
            b.volume = 0.0;
        }
        assert_eq!(stale_runs(&bars, &cfg(), "1m").outcome, Outcome::Fail);
    }

    #[test]
    fn volume_fires_on_a_zero_share_over_the_limit() {
        let mut bars = clean(100);
        for b in bars.iter_mut().take(20) {
            b.volume = 0.0;
        }
        let r = volume(&bars, &cfg());
        assert_eq!(r.outcome, Outcome::Fail);
        assert!((r.metric.unwrap() - 0.20).abs() < 1e-9);
    }

    #[test]
    fn volume_fires_on_a_spike_over_the_median_multiple() {
        let mut bars = clean(200);
        for b in &mut bars {
            b.volume = 1_000.0;
        }
        bars[150].volume = 1_000.0 * 25.0;
        let r = volume(&bars, &cfg());
        assert_eq!(r.outcome, Outcome::Fail, "{}", r.detail);
        assert!(r.offending.contains(&150));
    }

    #[test]
    fn volume_is_silent_on_a_clean_series() {
        assert_eq!(volume(&clean(200), &cfg()).outcome, Outcome::Pass);
    }

    // ── the check that forces D ──────────────────────────────────────────────

    /// The census found this condition across all 158,259 rows the platform
    /// holds, so it is not a hypothetical.
    #[test]
    fn timestamps_fires_when_knowledge_time_precedes_event_time() {
        let mut bars = clean(50);
        bars[20].knowledge_ns = bars[20].event_ns - 1;
        let r = timestamps(&bars);
        assert_eq!(r.outcome, Outcome::Fail);
        assert!(r.detail.contains("1 bar(s) with knowledge_time before"), "{}", r.detail);
        assert_eq!(r.offending, vec![20]);
    }

    #[test]
    fn timestamps_fires_on_out_of_order_and_duplicate_bars() {
        let mut out = clean(50);
        out[30].event_ns = out[29].event_ns - MIN;
        assert!(timestamps(&out).detail.contains("1 out of order"));

        // Same event_ns *and* no later knowledge: indistinguishable to a PIT read.
        let mut dup = clean(50);
        dup[30].event_ns = dup[29].event_ns;
        dup[30].knowledge_ns = dup[29].knowledge_ns;
        assert!(timestamps(&dup).detail.contains("1 duplicate"));
    }

    /// A restatement is two rows at one `event_ns` separated by `knowledge_ns`. A
    /// PIT read resolves it to exactly one bar per cutoff, so it is not a duplicate
    /// and must not force grade D. `market_bars_v2` could never produce this case —
    /// it merged the superseded row away — so it went unnoticed until the census
    /// moved to the canonical table (ADR-D-29).
    #[test]
    fn timestamps_does_not_count_a_restatement_as_a_duplicate() {
        let mut revised = clean(50);
        revised[30].event_ns = revised[29].event_ns;
        revised[30].knowledge_ns = revised[29].knowledge_ns + MIN;
        revised[30].revised = true;

        let r = timestamps(&revised);
        assert!(r.detail.contains("0 duplicate"), "{}", r.detail);
        assert_eq!(r.outcome, Outcome::Pass, "{}", r.detail);
    }

    /// One bar revised many times stays clean, however long the chain — 54 of them
    /// is a real series on this platform, not a contrived bound.
    #[test]
    fn timestamps_accepts_a_long_restatement_chain() {
        let mut bars = clean(10);
        let anchor = bars[5].clone();
        for k in 1..=54 {
            let mut b = anchor.clone();
            b.knowledge_ns = anchor.knowledge_ns + k * MIN;
            b.revised = true;
            bars.insert(5 + k as usize, b);
        }
        let r = timestamps(&bars);
        assert!(r.detail.contains("0 duplicate"), "{}", r.detail);
        assert_eq!(r.outcome, Outcome::Pass, "{}", r.detail);
    }

    /// A restatement stamped as knowable no later than the row it supersedes is
    /// still a duplicate: the `revised` flag does not excuse it, because a PIT read
    /// cannot separate the two whatever they are called.
    #[test]
    fn timestamps_still_catches_a_revision_flag_with_no_knowledge_gap() {
        let mut bars = clean(50);
        bars[30].event_ns = bars[29].event_ns;
        bars[30].knowledge_ns = bars[29].knowledge_ns;
        bars[30].revised = true;
        assert!(timestamps(&bars).detail.contains("1 duplicate"));
    }

    #[test]
    fn timestamps_is_silent_on_a_clean_series() {
        assert_eq!(timestamps(&clean(200)).outcome, Outcome::Pass);
    }

    // ── determinism ──────────────────────────────────────────────────────────

    /// Same input, same config, identical report. A grade that varies between
    /// runs is not a grade.
    #[test]
    fn every_check_is_deterministic() {
        let bars = clean(300);
        let c = cfg();
        for _ in 0..3 {
            assert_eq!(coverage(&bars, 300, &c, MIN), coverage(&bars, 300, &c, MIN));
            assert_eq!(bad_ticks(&bars, &c), bad_ticks(&bars, &c));
            assert_eq!(stale_runs(&bars, &c, "1m"), stale_runs(&bars, &c, "1m"));
            assert_eq!(volume(&bars, &c), volume(&bars, &c));
            assert_eq!(timestamps(&bars), timestamps(&bars));
        }
    }

    /// One broken series must not produce a megabyte of report.
    #[test]
    fn the_offending_list_is_capped() {
        let mut bars = clean(500);
        for b in &mut bars {
            b.knowledge_ns = b.event_ns - 1;
        }
        let r = timestamps(&bars);
        assert_eq!(r.offending.len(), MAX_OFFENDING);
        assert!(r.detail.contains("500 bar(s)"), "the count is still honest: {}", r.detail);
    }

    /// A grade with no reasons is unreviewable and unappealable.
    #[test]
    fn every_check_returns_a_detail_somebody_can_act_on() {
        let bars = clean(100);
        let c = cfg();
        for r in [
            coverage(&bars, 100, &c, MIN),
            revisions(&bars),
            bad_ticks(&bars, &c),
            stale_runs(&bars, &c, "1m"),
            volume(&bars, &c),
            timestamps(&bars),
        ] {
            assert!(!r.detail.trim().is_empty(), "{:?} has no detail", r.kind);
            assert!(r.detail.len() > 15, "{:?}: {}", r.kind, r.detail);
        }
    }

    #[test]
    fn an_empty_series_does_not_panic_any_check() {
        let none: Vec<QcBar> = vec![];
        let c = cfg();
        assert_eq!(volume(&none, &c).outcome, Outcome::InsufficientMetadata);
        assert_eq!(timestamps(&none).outcome, Outcome::Pass);
        assert_eq!(bad_ticks(&none, &c).outcome, Outcome::Pass);
        assert_eq!(stale_runs(&none, &c, "1m").outcome, Outcome::Pass);
        assert_eq!(revisions(&none).outcome, Outcome::Pass);
    }
}
