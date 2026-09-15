//! The data-quality battery (DATA-005 §7; Set P Phase 2).
//!
//! One deterministic job per `(instrument, timeframe, window)` producing a
//! [`QcReport`]: eight checks, a grade, and the reasons behind it.
//!
//! ## Why grading is a separate function from checking
//!
//! [`checks`] measures; [`grade`] judges. Keeping them apart means a threshold
//! change can re-grade the whole history **without re-running the battery** —
//! the checks' metrics are stored, and the grade is a pure function of those
//! metrics plus a config version. That matters because re-running the battery
//! over every series the platform holds is expensive, and the thresholds are
//! exactly the thing most likely to be revised.
//!
//! ## Determinism, and what it is worth
//!
//! `run` over the same bars with the same config produces a byte-identical
//! report. That is not a nicety: a grade that varies between runs cannot be
//! appealed, cannot be cached, and cannot be compared across time to show
//! whether a series is getting better. [`QcReport::digest`] is what a caller
//! stores to prove two reports are the same.
//!
//! ## Publish before enforcing
//!
//! P-2.6 sequences a full run and a published blast radius **before** P-2.3's
//! refusal is switched on. Nothing in this module enforces anything; it produces
//! evidence, and the route layer decides what to do with it. Switching on
//! refusal without first knowing how many series would be refused is how a
//! correctness improvement becomes an outage.

pub mod checks;
pub mod config;
pub mod cross_source;
pub mod grade;
pub mod survivorship;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use checks::{CheckKind, CheckResult, Outcome, QcBar};
pub use config::QcConfig;
pub use grade::{Grade, Graded};
pub use survivorship::{ListingStatus, Membership};

/// What the battery is being run against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QcSubject {
    pub instrument_id: String,
    pub venue_id: String,
    pub timeframe: String,
    pub from_ns: i64,
    pub to_ns: i64,
    /// Bars the census says should be present over this window.
    ///
    /// Supplied rather than derived: the count depends on the venue's session
    /// calendar, and a checker that guesses it is wrong on every holiday.
    pub expected_bars: u64,
    /// One period in nanoseconds, for gap measurement.
    pub period_ns: i64,
}

/// Everything the battery needs beyond the bars themselves.
#[derive(Clone, Debug, Default)]
pub struct QcContext<'a> {
    /// A second venue's bars for the same instrument and window, if one exists.
    pub secondary: Option<&'a [QcBar]>,
    /// The universe's membership, when grading a universe.
    pub membership: Option<Membership>,
    /// This instrument's listing status, when grading one series.
    pub listing: Option<ListingStatus>,
    /// Whether bad ticks found in this series were repaired in storage. Moves a
    /// bad-tick failure between C and D.
    pub bad_ticks_repaired: bool,
}

/// One complete run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QcReport {
    pub subject: QcSubject,
    pub graded: Graded,
    pub bars_examined: usize,
}

impl QcReport {
    /// A content digest of the report's *findings*.
    ///
    /// Covers the subject, the grade, every check's outcome and metric, and the
    /// config version. Deliberately **excludes** prose details and the offending
    /// index lists — rewording a message must not invalidate a stored digest,
    /// while changing an outcome or a metric always must.
    #[must_use]
    pub fn digest(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.subject.instrument_id.as_bytes());
        h.update(self.subject.venue_id.as_bytes());
        h.update(self.subject.timeframe.as_bytes());
        h.update(self.subject.from_ns.to_le_bytes());
        h.update(self.subject.to_ns.to_le_bytes());
        h.update(self.subject.expected_bars.to_le_bytes());
        h.update(self.graded.config_version.as_bytes());
        h.update(self.graded.grade.as_str().as_bytes());
        for c in &self.graded.checks {
            h.update(c.kind.as_str().as_bytes());
            h.update(format!("{:?}", c.outcome).as_bytes());
            // `None` hashes as a distinct fixed pattern rather than as any
            // float, so "could not run" and "measured exactly zero" are
            // different findings and digest differently.
            match c.metric {
                Some(m) => h.update(m.to_bits().to_le_bytes()),
                None => h.update(b"no-metric"),
            }
        }
        hex::encode(&h.finalize()[..16])
    }

    /// Whether an Experiment may use this series without a human waiver.
    #[must_use]
    pub const fn admissible_without_waiver(&self) -> bool {
        self.graded.grade.admissible_without_waiver()
    }
}

/// Run the whole battery.
///
/// Every check runs, including the ones that will report `NotApplicable` — a
/// battery that skips a check it cannot run produces a report where the reader
/// cannot tell the difference between "passed" and "absent", which is the
/// distinction the whole three-state outcome exists to preserve.
#[must_use]
pub fn run(bars: &[QcBar], subject: &QcSubject, cfg: &QcConfig, ctx: &QcContext<'_>) -> QcReport {
    let mut results = vec![
        checks::coverage(bars, subject.expected_bars, cfg, subject.period_ns),
        checks::revisions(bars),
        checks::bad_ticks(bars, cfg),
        checks::stale_runs(bars, cfg, &subject.timeframe),
        checks::volume(bars, cfg),
        checks::timestamps(bars),
    ];

    results.push(match ctx.secondary {
        Some(other) => cross_source::check(bars, other, cfg),
        None => cross_source::check(bars, &[], cfg),
    });

    results.push(match (&ctx.membership, ctx.listing) {
        (Some(m), _) => survivorship::check_universe(m),
        (None, Some(status)) => survivorship::check_instrument(status),
        (None, None) => survivorship::check_universe(&Membership::Unknown),
    });

    // Ordered so the report is stable regardless of the order above.
    results.sort_by_key(|c| c.kind);

    QcReport {
        graded: grade::grade(results, cfg, ctx.bad_ticks_repaired),
        subject: subject.clone(),
        bars_examined: bars.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000_000_000;

    fn bars(n: usize) -> Vec<QcBar> {
        (0..n)
            .map(|i| {
                let p = 100.0 + (i as f64 * 0.11).sin() * 0.4;
                QcBar {
                    event_ns: i as i64 * MIN,
                    knowledge_ns: i as i64 * MIN + 1_000,
                    open: p,
                    high: p + 0.2,
                    low: p - 0.2,
                    close: p + 0.03,
                    volume: 900.0 + i as f64,
                    revised: false,
                }
            })
            .collect()
    }

    fn subject(n: usize) -> QcSubject {
        QcSubject {
            instrument_id: "BTC-USD".into(),
            venue_id: "coinbase".into(),
            timeframe: "1m".into(),
            from_ns: 0,
            to_ns: n as i64 * MIN,
            expected_bars: n as u64,
            period_ns: MIN,
        }
    }

    /// The universe case, so survivorship has evidence rather than reporting
    /// `InsufficientMetadata` and dragging the grade down in every test.
    fn ctx() -> QcContext<'static> {
        QcContext {
            membership: Some(Membership::Historical {
                members: 1,
                departed: 1,
                window_days: 30,
            }),
            ..QcContext::default()
        }
    }

    // ── the whole battery ────────────────────────────────────────────────────

    #[test]
    fn a_clean_series_with_a_second_source_grades_a() {
        let b = bars(400);
        let secondary = bars(400);
        let c = QcContext { secondary: Some(&secondary), ..ctx() };
        let r = run(&b, &subject(400), &QcConfig::default(), &c);
        assert_eq!(r.graded.grade, Grade::A, "{:?}", r.graded.reasons);
        assert_eq!(r.bars_examined, 400);
    }

    /// A battery that skips a check produces a report where the reader cannot
    /// tell "passed" from "absent".
    #[test]
    fn all_eight_checks_are_always_present_in_the_report() {
        let r = run(&bars(100), &subject(100), &QcConfig::default(), &QcContext::default());
        assert_eq!(r.graded.checks.len(), 8);
        let kinds: Vec<CheckKind> = r.graded.checks.iter().map(|c| c.kind).collect();
        for k in CheckKind::ALL {
            assert!(kinds.contains(k), "{k:?} missing from the report");
        }
    }

    /// With no second venue and no membership, two checks cannot run — and the
    /// grade is capped rather than the report claiming an A.
    #[test]
    fn a_series_with_no_second_source_and_no_membership_cannot_reach_an_a() {
        let r = run(&bars(400), &subject(400), &QcConfig::default(), &QcContext::default());
        let unevidenced = r.graded.unevidenced();
        assert!(unevidenced.contains(&CheckKind::CrossSource), "{unevidenced:?}");
        assert!(unevidenced.contains(&CheckKind::Survivorship), "{unevidenced:?}");
        assert!(r.graded.grade < Grade::A, "got {:?}", r.graded.grade);
    }

    // ── determinism ──────────────────────────────────────────────────────────

    /// A grade that varies between runs cannot be appealed, cached, or compared
    /// across time.
    #[test]
    fn the_same_input_and_config_produce_a_byte_identical_report() {
        let b = bars(500);
        let s = subject(500);
        let cfg = QcConfig::default();
        let first = run(&b, &s, &cfg, &ctx());
        for _ in 0..5 {
            let again = run(&b, &s, &cfg, &ctx());
            assert_eq!(first, again);
            assert_eq!(first.digest(), again.digest());
        }
    }

    #[test]
    fn the_check_order_in_a_report_is_stable() {
        let r = run(&bars(50), &subject(50), &QcConfig::default(), &ctx());
        let kinds: Vec<CheckKind> = r.graded.checks.iter().map(|c| c.kind).collect();
        let mut sorted = kinds.clone();
        sorted.sort();
        assert_eq!(kinds, sorted);
    }

    /// Rewording a message must not invalidate a stored digest; changing an
    /// outcome always must.
    #[test]
    fn the_digest_covers_findings_and_not_prose() {
        let mut a = run(&bars(200), &subject(200), &QcConfig::default(), &ctx());
        let original = a.digest();

        a.graded.checks[0].detail = "reworded entirely".into();
        a.graded.checks[0].offending = vec![1, 2, 3];
        assert_eq!(a.digest(), original, "prose and indices are not findings");

        a.graded.checks[0].outcome = Outcome::Fail;
        assert_ne!(a.digest(), original, "an outcome change must move the digest");
    }

    /// Several checks report no metric at all when they could not run. An
    /// earlier version used `NaN` as that sentinel, which made a report unequal
    /// to a re-run of itself and unable to survive JSON. `Option` is the fix and
    /// this pins it.
    #[test]
    fn a_report_containing_absent_metrics_still_digests_consistently() {
        let r = run(&bars(100), &subject(100), &QcConfig::default(), &QcContext::default());
        assert!(
            r.graded.checks.iter().any(|c| c.metric.is_none()),
            "this fixture should contain at least one check that could not run"
        );
        assert_eq!(r.digest(), r.digest());
    }

    #[test]
    fn a_different_config_produces_a_different_digest() {
        let b = bars(300);
        let s = subject(300);
        let mut strict = QcConfig::default();
        strict.coverage.b_max_missing = 0.0001;
        assert_ne!(
            run(&b, &s, &QcConfig::default(), &ctx()).digest(),
            run(&b, &s, &strict, &ctx()).digest()
        );
    }

    // ── planted failures reach the grade ─────────────────────────────────────

    #[test]
    fn a_timestamp_violation_reaches_the_report_as_a_d() {
        let mut b = bars(300);
        b[100].knowledge_ns = b[100].event_ns - 1;
        let r = run(&b, &subject(300), &QcConfig::default(), &ctx());
        assert_eq!(r.graded.grade, Grade::D);
        assert!(!r.admissible_without_waiver());
        assert!(!r.graded.grade.waivable(), "a D cannot be waived");
    }

    #[test]
    fn missing_bars_reach_the_report_as_a_d() {
        let b = bars(200);
        let mut s = subject(200);
        s.expected_bars = 400; // half the window is absent
        let r = run(&b, &s, &QcConfig::default(), &ctx());
        assert_eq!(r.graded.grade, Grade::D);
        assert!(r.graded.reasons.iter().any(|x| x.contains("missing")), "{:?}", r.graded.reasons);
    }

    #[test]
    fn a_cross_source_divergence_reaches_the_report_as_a_c() {
        let b = bars(400);
        let mut other = bars(400);
        for x in other.iter_mut().skip(100).take(50) {
            x.close *= 1.05;
        }
        let c = QcContext { secondary: Some(&other), ..ctx() };
        let r = run(&b, &subject(400), &QcConfig::default(), &c);
        assert_eq!(r.graded.grade, Grade::C, "{:?}", r.graded.reasons);
        assert!(r.graded.grade.waivable(), "a C is the case a waiver exists for");
    }

    #[test]
    fn an_empty_window_does_not_panic() {
        let r = run(&[], &subject(0), &QcConfig::default(), &QcContext::default());
        assert_eq!(r.bars_examined, 0);
        assert_eq!(r.graded.checks.len(), 8);
    }

    #[test]
    fn a_report_round_trips_through_json() {
        let r = run(&bars(120), &subject(120), &QcConfig::default(), &ctx());
        let s = serde_json::to_string(&r).unwrap();
        let back: QcReport = serde_json::from_str(&s).unwrap();
        assert_eq!(back.digest(), r.digest());
    }
}
