//! Grades A–D, with their reasons (DATA-005 §7, P-2.2).
//!
//! The rule, exactly as specified:
//!
//! | Grade | Condition |
//! |---|---|
//! | A | every check passes |
//! | B | gaps < 0.5% and no unrepaired bad ticks |
//! | C | material issues — gaps < 5%, or repaired bad ticks |
//! | D | gaps ≥ 5%, unrepaired bad ticks, or **any** timestamp violation |
//!
//! ## Two things the letter alone cannot carry
//!
//! **Reasons.** A grade with no reasons is unreviewable and unappealable — the
//! researcher refused at Experiment creation cannot tell whether the answer is
//! "collect more history" or "this venue is lying". So [`Grade`] is never
//! returned on its own; [`grade`] returns a [`Graded`] carrying every reason
//! that moved the letter, and the persisted row stores them.
//!
//! **Checks that could not run.** `NotApplicable` and `InsufficientMetadata` are
//! not passes, so a series where six of eight checks could not run does not earn
//! an A. [`Graded::evidence_completeness`] reports what fraction of the battery
//! actually produced evidence, and a grade computed from too little of it is
//! capped — see [`grade`].

use serde::{Deserialize, Serialize};

use super::checks::{CheckKind, CheckResult, Outcome};
use super::config::QcConfig;

/// The letter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Grade {
    /// Ordered worst-first so `Ord` means "at least this good" without a
    /// custom impl, and so `min` is the conservative combinator.
    D,
    C,
    B,
    A,
}

impl Grade {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
        }
    }

    /// Whether an Experiment may use this series without a waiver.
    #[must_use]
    pub const fn admissible_without_waiver(self) -> bool {
        matches!(self, Self::A | Self::B)
    }

    /// Whether a waiver could ever admit it. Grade D cannot be waived: the data
    /// is wrong, and a human signature does not make it right.
    #[must_use]
    pub const fn waivable(self) -> bool {
        matches!(self, Self::C)
    }
}

/// A grade and everything that produced it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Graded {
    pub grade: Grade,
    /// Every reason that moved the letter, worst first. Never empty — an A
    /// carries "every check passed" rather than silence.
    pub reasons: Vec<String>,
    pub checks: Vec<CheckResult>,
    /// The config that produced this. Two grades with different versions were
    /// not computed by the same rules, whatever their letters say.
    pub config_version: String,
}

impl Graded {
    /// Fraction of the battery that actually produced evidence.
    ///
    /// `NotApplicable` and `InsufficientMetadata` do not count. A series where
    /// most of the battery could not run has not been graded so much as
    /// guessed at.
    #[must_use]
    pub fn evidence_completeness(&self) -> f64 {
        if self.checks.is_empty() {
            return 0.0;
        }
        let evidenced = self
            .checks
            .iter()
            .filter(|c| c.outcome.is_evidence())
            .count();
        evidenced as f64 / self.checks.len() as f64
    }

    /// The checks that could not run, by name.
    #[must_use]
    pub fn unevidenced(&self) -> Vec<CheckKind> {
        self.checks
            .iter()
            .filter(|c| !c.outcome.is_evidence())
            .map(|c| c.kind)
            .collect()
    }
}

/// Below this fraction of evidenced checks, a grade is capped at C.
///
/// Five of eight. The number is a judgement and is written down rather than
/// inlined: a series graded on three checks is not an A-grade series, it is an
/// ungraded one, and calling it A would let the thinnest data on the platform
/// through the strictest gate.
const MIN_EVIDENCE_FOR_TOP_GRADES: f64 = 5.0 / 8.0;

/// Grade a completed battery.
///
/// `repaired_bad_ticks` says whether the bad ticks found were corrected in the
/// stored series. The spec distinguishes repaired from unrepaired and they land
/// in different grades — a venue that published a bad print and then corrected
/// it is a C, one that never corrected it is a D.
#[must_use]
pub fn grade(checks: Vec<CheckResult>, cfg: &QcConfig, repaired_bad_ticks: bool) -> Graded {
    let find = |k: CheckKind| checks.iter().find(|c| c.kind == k);
    let mut reasons: Vec<String> = Vec::new();
    let mut letter = Grade::A;

    // ── D, and it outranks everything ──
    //
    // A timestamp violation forces D whatever else passed. A series whose
    // `knowledge_time` is wrong is one where every point-in-time filter reads
    // the future, and no amount of clean OHLC compensates for that.
    if let Some(ts) = find(CheckKind::Timestamps) {
        if ts.outcome == Outcome::Fail {
            letter = Grade::D;
            reasons.push(format!("D: timestamp violation — {}", ts.detail));
        }
    }

    let missing = find(CheckKind::Coverage)
        .and_then(|c| c.metric)
        .unwrap_or(0.0);
    if find(CheckKind::Coverage).is_some_and(|c| c.outcome == Outcome::Fail)
        || missing >= cfg.coverage.d_min_missing
    {
        letter = letter.min(Grade::D);
        reasons.push(format!(
            "D: {:.2}% of expected bars missing, at or above the {:.0}% limit",
            missing * 100.0,
            cfg.coverage.d_min_missing * 100.0
        ));
    }

    let bad = find(CheckKind::BadTicks);
    if bad.is_some_and(|c| c.outcome == Outcome::Fail) {
        if repaired_bad_ticks {
            letter = letter.min(Grade::C);
            reasons.push(format!(
                "C: bad ticks found and repaired in the stored series — {}",
                bad.map_or("", |c| c.detail.as_str())
            ));
        } else {
            letter = letter.min(Grade::D);
            reasons.push(format!(
                "D: unrepaired bad ticks — {}",
                bad.map_or("", |c| c.detail.as_str())
            ));
        }
    }

    // ── B: gaps under 0.5% ──
    if missing >= cfg.coverage.b_max_missing && missing < cfg.coverage.d_min_missing {
        letter = letter.min(Grade::C);
        reasons.push(format!(
            "C: {:.2}% of expected bars missing, over the {:.1}% B limit",
            missing * 100.0,
            cfg.coverage.b_max_missing * 100.0
        ));
    }

    // ── the remaining failures are material but not fatal ──
    for kind in [
        CheckKind::StaleRuns,
        CheckKind::Volume,
        CheckKind::CrossSource,
    ] {
        if let Some(c) = find(kind) {
            if c.outcome == Outcome::Fail {
                letter = letter.min(Grade::C);
                reasons.push(format!("C: {} — {}", kind.as_str(), c.detail));
            }
        }
    }

    // Survivorship is a property of the *study*, not of the bars, so it cannot
    // lower a series grade. It is reported so the refusal path can surface it.
    if let Some(c) = find(CheckKind::Survivorship) {
        if c.outcome == Outcome::Fail {
            reasons.push(format!("flag: survivorship — {}", c.detail));
        }
    }

    let graded_so_far = Graded {
        grade: letter,
        reasons: reasons.clone(),
        checks: checks.clone(),
        config_version: cfg.version(),
    };
    // `A: all pass` means all eight. A check that could not run did not pass,
    // so any unevidenced check caps the letter at B — otherwise the thinnest
    // data on the platform earns the grade that clears the strictest gate.
    let completeness = graded_so_far.evidence_completeness();
    if completeness < 1.0 && letter > Grade::B {
        letter = Grade::B;
        let names: Vec<&str> = graded_so_far
            .unevidenced()
            .iter()
            .map(|k| k.as_str())
            .collect();
        reasons.push(format!(
            "B: {} could not run, and an A means every one of the eight checks passed",
            names.join(", ")
        ));
    }
    if completeness < MIN_EVIDENCE_FOR_TOP_GRADES && letter > Grade::C {
        letter = Grade::C;
        let names: Vec<&str> = graded_so_far
            .unevidenced()
            .iter()
            .map(|k| k.as_str())
            .collect();
        reasons.push(format!(
            "C: only {:.0}% of the battery produced evidence ({} could not run); a grade above C \
             needs at least {:.0}%",
            completeness * 100.0,
            names.join(", "),
            MIN_EVIDENCE_FOR_TOP_GRADES * 100.0
        ));
    }

    if reasons.is_empty() {
        reasons.push("A: every check passed".into());
    }

    Graded {
        grade: letter,
        reasons,
        checks,
        config_version: cfg.version(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(kind: CheckKind, outcome: Outcome, metric: f64) -> CheckResult {
        CheckResult {
            kind,
            outcome,
            detail: format!("{} detail for the test", kind.as_str()),
            offending: vec![],
            metric: Some(metric),
        }
    }

    /// A full battery where everything passes.
    fn all_pass() -> Vec<CheckResult> {
        CheckKind::ALL
            .iter()
            .map(|k| check(*k, Outcome::Pass, 0.0))
            .collect()
    }

    fn with(kind: CheckKind, outcome: Outcome, metric: f64) -> Vec<CheckResult> {
        let mut v = all_pass();
        if let Some(c) = v.iter_mut().find(|c| c.kind == kind) {
            *c = check(kind, outcome, metric);
        }
        v
    }

    fn cfg() -> QcConfig {
        QcConfig::default()
    }

    // ── the four grades ──────────────────────────────────────────────────────

    #[test]
    fn everything_passing_is_an_a() {
        let g = grade(all_pass(), &cfg(), false);
        assert_eq!(g.grade, Grade::A);
        assert_eq!(g.reasons, vec!["A: every check passed"]);
    }

    /// Both sides of the 0.5% B boundary.
    #[test]
    fn the_b_boundary_is_half_a_percent_of_gaps() {
        let just_under = grade(
            with(CheckKind::Coverage, Outcome::Pass, 0.0049),
            &cfg(),
            false,
        );
        assert_eq!(just_under.grade, Grade::A, "{:?}", just_under.reasons);

        let at_limit = grade(
            with(CheckKind::Coverage, Outcome::Pass, 0.005),
            &cfg(),
            false,
        );
        assert_eq!(
            at_limit.grade,
            Grade::C,
            "0.5% is the limit, not the allowance"
        );
    }

    /// Both sides of the 5% D boundary.
    #[test]
    fn the_d_boundary_is_five_percent_of_gaps() {
        let just_under = grade(
            with(CheckKind::Coverage, Outcome::Pass, 0.049),
            &cfg(),
            false,
        );
        assert_eq!(just_under.grade, Grade::C);

        let at_limit = grade(
            with(CheckKind::Coverage, Outcome::Fail, 0.05),
            &cfg(),
            false,
        );
        assert_eq!(at_limit.grade, Grade::D);
        assert!(at_limit
            .reasons
            .iter()
            .any(|r| r.contains("5.00% of expected bars missing")));
    }

    /// A venue that published a bad print and corrected it is a C; one that
    /// never corrected it is a D.
    #[test]
    fn repaired_and_unrepaired_bad_ticks_land_in_different_grades() {
        let checks = with(CheckKind::BadTicks, Outcome::Fail, 3.0);
        assert_eq!(grade(checks.clone(), &cfg(), true).grade, Grade::C);
        assert_eq!(grade(checks, &cfg(), false).grade, Grade::D);
    }

    /// The rule that earns its own line in the spec.
    #[test]
    fn a_timestamp_violation_forces_d_with_every_other_check_passing() {
        let checks = with(CheckKind::Timestamps, Outcome::Fail, 1.0);
        let g = grade(checks, &cfg(), true);
        assert_eq!(g.grade, Grade::D);
        assert!(
            g.reasons
                .iter()
                .any(|r| r.starts_with("D: timestamp violation")),
            "{:?}",
            g.reasons
        );
    }

    #[test]
    fn stale_runs_and_volume_are_material_but_not_fatal() {
        for kind in [
            CheckKind::StaleRuns,
            CheckKind::Volume,
            CheckKind::CrossSource,
        ] {
            let g = grade(with(kind, Outcome::Fail, 1.0), &cfg(), false);
            assert_eq!(g.grade, Grade::C, "{kind:?} should be C, got {:?}", g.grade);
        }
    }

    /// Survivorship is a property of the study, not of the bars.
    #[test]
    fn survivorship_flags_without_lowering_the_grade() {
        let g = grade(
            with(CheckKind::Survivorship, Outcome::Fail, 1.0),
            &cfg(),
            false,
        );
        assert_eq!(g.grade, Grade::A);
        assert!(
            g.reasons
                .iter()
                .any(|r| r.starts_with("flag: survivorship")),
            "{:?}",
            g.reasons
        );
    }

    // ── reasons ──────────────────────────────────────────────────────────────

    /// The researcher refused at Experiment creation cannot tell whether the
    /// answer is "collect more history" or "this venue is lying" without these.
    #[test]
    fn no_grade_is_ever_returned_without_a_reason() {
        let cases = [
            all_pass(),
            with(CheckKind::Coverage, Outcome::Fail, 0.2),
            with(CheckKind::Timestamps, Outcome::Fail, 1.0),
            with(CheckKind::Volume, Outcome::Fail, 1.0),
        ];
        for checks in cases {
            let g = grade(checks, &cfg(), false);
            assert!(!g.reasons.is_empty());
            assert!(g.reasons.iter().all(|r| r.len() > 10), "{:?}", g.reasons);
        }
    }

    #[test]
    fn every_failing_check_contributes_its_own_reason() {
        let mut checks = all_pass();
        for kind in [CheckKind::Volume, CheckKind::StaleRuns] {
            if let Some(c) = checks.iter_mut().find(|c| c.kind == kind) {
                *c = check(kind, Outcome::Fail, 1.0);
            }
        }
        let g = grade(checks, &cfg(), false);
        assert!(g.reasons.iter().any(|r| r.contains("volume")));
        assert!(g.reasons.iter().any(|r| r.contains("stale_runs")));
    }

    // ── checks that could not run ────────────────────────────────────────────

    /// A series graded on three checks is not an A-grade series, it is an
    /// ungraded one.
    #[test]
    fn a_grade_computed_from_too_little_evidence_is_capped_at_c() {
        let mut checks = all_pass();
        for c in checks.iter_mut().take(5) {
            c.outcome = Outcome::InsufficientMetadata;
        }
        let g = grade(checks, &cfg(), false);
        assert_eq!(g.grade, Grade::C);
        assert!(
            g.reasons.iter().any(|r| r.contains("could not run")),
            "the cap must say why: {:?}",
            g.reasons
        );
    }

    #[test]
    fn evidence_completeness_counts_only_pass_and_fail() {
        let mut checks = all_pass();
        checks[0].outcome = Outcome::NotApplicable;
        checks[1].outcome = Outcome::InsufficientMetadata;
        let g = grade(checks, &cfg(), false);
        assert!((g.evidence_completeness() - 6.0 / 8.0).abs() < 1e-9);
        assert_eq!(g.unevidenced().len(), 2);
    }

    /// The cap must not promote a D into a C.
    #[test]
    fn the_evidence_cap_never_improves_a_grade() {
        let mut checks = with(CheckKind::Timestamps, Outcome::Fail, 1.0);
        for c in checks.iter_mut().skip(3) {
            if c.kind != CheckKind::Timestamps {
                c.outcome = Outcome::NotApplicable;
            }
        }
        assert_eq!(grade(checks, &cfg(), false).grade, Grade::D);
    }

    // ── admission ────────────────────────────────────────────────────────────

    /// Grade D cannot be waived: the data is wrong, and a human signature does
    /// not make it right.
    #[test]
    fn d_is_never_waivable_and_a_and_b_need_no_waiver() {
        assert!(Grade::A.admissible_without_waiver());
        assert!(Grade::B.admissible_without_waiver());
        assert!(!Grade::C.admissible_without_waiver());
        assert!(!Grade::D.admissible_without_waiver());

        assert!(Grade::C.waivable());
        assert!(!Grade::D.waivable());
        assert!(
            !Grade::A.waivable(),
            "an A needs no waiver, so asking for one is a mistake"
        );
    }

    /// Ordering is worst-first so `min` is the conservative combinator; a bug
    /// here would silently upgrade every grade.
    #[test]
    fn grades_order_worst_first_so_min_is_conservative() {
        assert!(Grade::D < Grade::C);
        assert!(Grade::C < Grade::B);
        assert!(Grade::B < Grade::A);
        assert_eq!(Grade::A.min(Grade::D), Grade::D);
        assert_eq!(Grade::B.min(Grade::C), Grade::C);
    }

    #[test]
    fn the_config_version_is_carried_onto_the_grade() {
        let g = grade(all_pass(), &cfg(), false);
        assert_eq!(g.config_version, cfg().version());
        assert_eq!(g.config_version.len(), 16);
    }

    /// Changing a threshold must be able to re-grade history without re-running
    /// the battery — which only works if the grade is a pure function of the
    /// checks plus the config.
    #[test]
    fn the_same_checks_under_a_stricter_config_grade_worse() {
        let checks = with(CheckKind::Coverage, Outcome::Pass, 0.004);
        assert_eq!(grade(checks.clone(), &cfg(), false).grade, Grade::A);

        let mut strict = cfg();
        strict.coverage.b_max_missing = 0.001;
        let g = grade(checks, &strict, false);
        assert_eq!(g.grade, Grade::C);
        assert_ne!(g.config_version, cfg().version());
    }
}
