//! Cross-source deviation (DATA-005 §7, P-2.4).
//!
//! Where a second venue covers the same instrument, compare them and flag the
//! windows where they disagree.
//!
//! This is the check that catches **a venue quietly serving bad history during a
//! backfill** — the failure Set P Phase 1 is most likely to introduce and least
//! likely to notice, because a backfilled series that is wrong is still
//! complete, still monotonic, still has plausible volume, and passes every other
//! check in the battery. Only a second opinion finds it.
//!
//! ## Why median absolute deviation
//!
//! A mean would be dragged by the very outliers being looked for: one bad print
//! in ten thousand bars moves the mean enough to widen the tolerance past the
//! next bad print. The median does not move, so the check gets *more* sensitive
//! as the data gets cleaner, which is the right direction.

use serde::{Deserialize, Serialize};

use super::checks::{CheckKind, CheckResult, Outcome, QcBar};
use super::config::QcConfig;

/// A window where two venues disagreed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Divergence {
    /// Inclusive bounds, as event timestamps in Unix nanoseconds. Timestamps
    /// rather than indices, because the two series have different indices and a
    /// reviewer needs to look the window up in both.
    pub from_ns: i64,
    pub to_ns: i64,
    /// Worst absolute deviation in the window, in basis points.
    pub worst_bps: f64,
    pub bars: usize,
}

/// The comparison.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossSource {
    /// Median absolute deviation over every aligned bar, in basis points.
    pub mad_bps: f64,
    /// Bars present in both series at the same timestamp. The denominator.
    pub aligned: usize,
    pub divergences: Vec<Divergence>,
}

/// Compare two venues' histories of the same instrument.
///
/// Returns `None` when there is no second source, which the caller turns into
/// [`Outcome::NotApplicable`] — never a pass. An instrument with one venue has
/// not been cross-checked, and recording that as a pass would mean every
/// single-venue series on the platform carried evidence it never earned.
#[must_use]
pub fn compare(primary: &[QcBar], secondary: &[QcBar], cfg: &QcConfig) -> Option<CrossSource> {
    if secondary.is_empty() || primary.is_empty() {
        return None;
    }

    // Align on event time. A venue that publishes on a different grid is not
    // comparable bar for bar, and interpolating to make it so would invent the
    // agreement being measured.
    let mut by_ts: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
    for b in secondary {
        if b.close > 0.0 {
            by_ts.insert(b.event_ns, b.close);
        }
    }

    let mut deviations: Vec<(i64, f64)> = Vec::new();
    for b in primary {
        if b.close <= 0.0 {
            continue;
        }
        if let Some(other) = by_ts.get(&b.event_ns) {
            let bps = ((b.close - other) / other).abs() * 10_000.0;
            if bps.is_finite() {
                deviations.push((b.event_ns, bps));
            }
        }
    }

    if deviations.is_empty() {
        return None;
    }

    let mut sorted: Vec<f64> = deviations.iter().map(|(_, d)| *d).collect();
    sorted.sort_by(f64::total_cmp);
    let mad_bps = sorted[sorted.len() / 2];

    // Contiguous runs over the threshold become one divergence each, so a
    // week-long outage is reported as a week rather than as ten thousand bars.
    let mut divergences: Vec<Divergence> = Vec::new();
    let mut open: Option<Divergence> = None;
    for (ts, bps) in &deviations {
        if *bps > cfg.cross_source.max_mad_bps {
            match open.as_mut() {
                Some(d) => {
                    d.to_ns = *ts;
                    d.bars += 1;
                    d.worst_bps = d.worst_bps.max(*bps);
                }
                None => {
                    open = Some(Divergence {
                        from_ns: *ts,
                        to_ns: *ts,
                        worst_bps: *bps,
                        bars: 1,
                    });
                }
            }
        } else if let Some(d) = open.take() {
            divergences.push(d);
        }
    }
    if let Some(d) = open.take() {
        divergences.push(d);
    }

    Some(CrossSource {
        mad_bps,
        aligned: deviations.len(),
        divergences,
    })
}

/// The check-battery wrapper.
#[must_use]
pub fn check(primary: &[QcBar], secondary: &[QcBar], cfg: &QcConfig) -> CheckResult {
    let Some(cs) = compare(primary, secondary, cfg) else {
        return CheckResult {
            kind: CheckKind::CrossSource,
            // Never `Pass`: a single-venue series has not been cross-checked,
            // and recording that as a pass gives it evidence it never earned.
            outcome: Outcome::NotApplicable,
            detail: "no second venue covers this instrument over this window, so the series has \
                     not been cross-checked. This is not a pass."
                .into(),
            offending: vec![],
            metric: None,
        };
    };

    let failed = !cs.divergences.is_empty();
    CheckResult {
        kind: CheckKind::CrossSource,
        outcome: if failed { Outcome::Fail } else { Outcome::Pass },
        detail: format!(
            "median absolute deviation {:.1} bp over {} aligned bars (limit {:.0} bp); \
             {} divergent window(s)",
            cs.mad_bps,
            cs.aligned,
            cfg.cross_source.max_mad_bps,
            cs.divergences.len()
        ),
        offending: vec![],
        metric: Some(cs.mad_bps),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000_000_000;

    fn series(n: usize, base: f64) -> Vec<QcBar> {
        (0..n)
            .map(|i| {
                let p = base + (i as f64 * 0.05);
                QcBar {
                    event_ns: i as i64 * MIN,
                    knowledge_ns: i as i64 * MIN + 1_000,
                    open: p,
                    high: p + 0.1,
                    low: p - 0.1,
                    close: p,
                    volume: 1_000.0,
                    revised: false,
                }
            })
            .collect()
    }

    fn cfg() -> QcConfig {
        QcConfig::default()
    }

    // ── the rule that matters ────────────────────────────────────────────────

    /// Every single-venue series on the platform would otherwise carry evidence
    /// it never earned.
    #[test]
    fn one_source_reports_not_applicable_and_never_a_pass() {
        let r = check(&series(100, 100.0), &[], &cfg());
        assert_eq!(r.outcome, Outcome::NotApplicable);
        assert!(!r.outcome.is_pass());
        assert!(!r.outcome.is_evidence());
        assert!(r.detail.contains("This is not a pass"), "{}", r.detail);
    }

    #[test]
    fn two_agreeing_venues_pass_with_a_small_deviation() {
        let a = series(500, 100.0);
        let b = series(500, 100.0);
        let r = check(&a, &b, &cfg());
        assert_eq!(r.outcome, Outcome::Pass, "{}", r.detail);
        assert!(
            r.metric.unwrap() < 1.0,
            "identical series deviate by ~0 bp: {:?}",
            r.metric
        );
    }

    /// The failure Phase 1 is most likely to introduce: a backfilled window that
    /// is complete, monotonic, plausibly-volumed and wrong.
    #[test]
    fn a_planted_divergence_is_flagged_with_its_bounds() {
        let a = series(500, 100.0);
        let mut b = series(500, 100.0);
        for bar in b.iter_mut().skip(200).take(50) {
            bar.close *= 1.05; // 500 bp out
        }

        let r = check(&a, &b, &cfg());
        assert_eq!(r.outcome, Outcome::Fail, "{}", r.detail);

        let cs = compare(&a, &b, &cfg()).unwrap();
        assert_eq!(cs.divergences.len(), 1, "one contiguous window, not fifty");
        let d = cs.divergences[0];
        assert_eq!(d.bars, 50);
        assert_eq!(d.from_ns, 200 * MIN);
        assert_eq!(d.to_ns, 249 * MIN);
        assert!(d.worst_bps > 400.0, "{}", d.worst_bps);
    }

    /// A week-long outage is one divergence, not ten thousand.
    #[test]
    fn contiguous_disagreement_collapses_into_one_window() {
        let a = series(300, 100.0);
        let mut b = series(300, 100.0);
        for bar in b.iter_mut().take(200) {
            bar.close *= 1.10;
        }
        let cs = compare(&a, &b, &cfg()).unwrap();
        assert_eq!(cs.divergences.len(), 1);
        assert_eq!(cs.divergences[0].bars, 200);
    }

    #[test]
    fn two_separate_divergent_windows_are_reported_separately() {
        let a = series(500, 100.0);
        let mut b = series(500, 100.0);
        for bar in b.iter_mut().skip(100).take(20) {
            bar.close *= 1.05;
        }
        for bar in b.iter_mut().skip(300).take(20) {
            bar.close *= 0.95;
        }
        assert_eq!(compare(&a, &b, &cfg()).unwrap().divergences.len(), 2);
    }

    // ── the median ───────────────────────────────────────────────────────────

    /// A mean would be dragged by the very outliers being looked for.
    #[test]
    fn one_extreme_outlier_does_not_move_the_median() {
        let a = series(1_000, 100.0);
        let mut b = series(1_000, 100.0);
        b[500].close *= 100.0; // an absurd print

        let cs = compare(&a, &b, &cfg()).unwrap();
        assert!(
            cs.mad_bps < 1.0,
            "the median must stay near zero; a mean would be ~99 bp. Got {}",
            cs.mad_bps
        );
        assert_eq!(cs.divergences.len(), 1, "and the outlier is still flagged");
    }

    // ── alignment ────────────────────────────────────────────────────────────

    /// Interpolating onto a common grid would invent the agreement being
    /// measured.
    #[test]
    fn only_bars_sharing_a_timestamp_are_compared() {
        let a = series(100, 100.0);
        let mut b = series(100, 100.0);
        for bar in &mut b {
            bar.event_ns += MIN / 2; // a half-bar offset grid
        }
        assert!(
            compare(&a, &b, &cfg()).is_none(),
            "no timestamps align, so there is nothing to compare"
        );
    }

    #[test]
    fn a_partially_overlapping_second_source_compares_only_the_overlap() {
        let a = series(100, 100.0);
        let b: Vec<QcBar> = series(100, 100.0).into_iter().skip(60).collect();
        let cs = compare(&a, &b, &cfg()).unwrap();
        assert_eq!(cs.aligned, 40);
    }

    #[test]
    fn non_positive_closes_are_skipped_rather_than_dividing_by_zero() {
        let a = series(50, 100.0);
        let mut b = series(50, 100.0);
        b[10].close = 0.0;
        let cs = compare(&a, &b, &cfg()).unwrap();
        assert_eq!(cs.aligned, 49);
        assert!(cs.mad_bps.is_finite());
    }

    #[test]
    fn the_comparison_is_deterministic() {
        let a = series(400, 100.0);
        let mut b = series(400, 100.0);
        for bar in b.iter_mut().skip(50).take(30) {
            bar.close *= 1.03;
        }
        let first = compare(&a, &b, &cfg());
        for _ in 0..3 {
            assert_eq!(compare(&a, &b, &cfg()), first);
        }
    }
}
