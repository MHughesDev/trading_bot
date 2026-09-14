//! The sixteen-gate stack, run as one thing (SPEC §12.3, checklist 2.14).
//!
//! Sixteen evaluators exist in [`super::evaluators`]. This is the entry point
//! that runs all of them, in order, against one candidate, and records every
//! verdict — including the ones that could not be evaluated — through
//! [`ledger::gates::GateLog`].
//!
//! ## Why it runs all sixteen rather than stopping at the first failure
//!
//! A stack that short-circuits tells you *a* reason the candidate failed. The
//! agent then fixes that one, resubmits, and discovers the next — one counted
//! trial per gate, learning the gate stack by exhaustion. Running all sixteen
//! costs the same (the evidence is already computed; the gates are arithmetic
//! over it) and returns the whole picture at once. [`StackVerdict::blocked_at`]
//! still names the first failure, for a caller that wants the headline.
//!
//! ## Why an unevaluated gate is recorded
//!
//! `GateOutcome::inconclusive` is never a pass, and it is also never silence. A
//! gate that could not run is written to `mlops.gate_verdict` as a failure with
//! its reason, so the `gate_pass_rate` signal in the self-monitor counts it. The
//! failure mode this prevents is a stack that quietly shrinks: evidence stops
//! being collected for gate 10, gate 10 stops appearing, and the pass rate goes
//! *up*.

use ledger::gates::{GateLog, GateRecord};
use uuid::Uuid;

use super::evaluators::{
    gate_10_factor_attribution, gate_11_regime_coverage, gate_12_perturbation,
    gate_13_stationary_bootstrap, gate_14_romano_wolf, gate_15_paper_shadow, gate_16_capital_ramp,
    gate_2_leakage_suite, gate_3_cost_sensitivity, gate_4_capacity, gate_9_min_length,
    CapacityInputs, CostRung, FactorInputs, GateOutcome, LeakageRun, LengthInputs,
    PerturbationInputs, VolRegime,
};
use super::profile::GateProfile;

/// The number of gates §12.3 defines. A stack that produced a different number
/// of verdicts is not this stack.
pub const GATE_COUNT: usize = 16;

/// How long a leakage run may be before the gate stops accepting it.
///
/// Three days: long enough that a suite run on Friday still gates a Monday
/// candidate, short enough that it cannot predate a dataset rebuild unnoticed.
pub const LEAKAGE_MAX_AGE: chrono::Duration = chrono::Duration::hours(72);

/// Everything the sixteen gates read.
///
/// Every field is an `Option` of already-computed evidence, and an absent field
/// makes its gate **inconclusive** rather than passing. That is the whole
/// contract: the stack cannot be made easier by supplying less.
pub struct StackEvidence<'a> {
    // Gate 1 — pre-registration.
    pub prereg_hash: Option<String>,
    // Gate 2 — the leakage suite's latest run.
    pub leakage_run: Option<&'a LeakageRun>,
    pub leakage_checks: &'a [&'a str],
    // Gate 3 — the cost ladder.
    pub cost_ladder: &'a [CostRung],
    // Gate 4 — capacity.
    pub capacity: Option<&'a CapacityInputs<'a>>,
    // Gate 5 — CPCV.
    pub cpcv_p05_sharpe: Option<f64>,
    // Gate 6 — walk-forward.
    pub walk_forward_sharpe: Option<f64>,
    pub walk_forward_regimes: Option<u32>,
    // Gate 7 — PBO.
    pub pbo: Option<f64>,
    // Gate 8 — deflated Sharpe.
    pub dsr: Option<f64>,
    // Gate 9 — length.
    pub length: Option<LengthInputs>,
    // Gate 10 — factor attribution.
    pub factors: Option<&'a FactorInputs<'a>>,
    // Gate 11 — regime coverage. Market first, strategy second (ADR-P2-29).
    pub market_returns: &'a [(chrono::DateTime<chrono::Utc>, f64)],
    pub strategy_returns: &'a [(chrono::DateTime<chrono::Utc>, f64)],
    // Gate 12 — perturbation.
    pub perturbation: Option<&'a PerturbationInputs<'a>>,
    // Gates 13 / 14 — the stored return series and the candidate family.
    pub return_series: &'a [f64],
    /// **Every** candidate the research programme generated, not the ones the
    /// agent liked. Gate 14 is only honest over the family that was searched.
    pub family: &'a [Vec<f64>],
    /// Where this candidate sits in `family`.
    pub candidate_index: usize,
    /// The platform-held seed (§12.7). Held by the platform precisely so the
    /// thing being evaluated cannot search over it.
    pub bootstrap_seed: u64,
    // Gate 15 — paper/shadow.
    pub paper: Option<super::evaluators::PaperInputs>,
    // Gate 16 — the current rung.
    pub capital_fraction: ledger::AllowedFraction,
    // The significance context INV-3 requires on gates 8 and 14.
    pub n_eff: f64,
    pub trial_count: i64,
    /// Which asset class's declared battery and crisis windows apply.
    pub asset_class: &'a str,
    /// What is being judged.
    pub experiment_id: Option<String>,
    pub trial_id: Option<Uuid>,
    pub campaign_id: Option<Uuid>,
}

/// The stack's answer.
pub struct StackVerdict {
    pub outcomes: Vec<GateOutcome>,
    /// The first gate that did not pass, if any.
    pub blocked_at: Option<i32>,
}

impl StackVerdict {
    /// Every gate passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.blocked_at.is_none() && self.outcomes.len() == GATE_COUNT
    }

    /// `(gate_no, passed)` for every gate — the shape
    /// [`ledger::capital::authorise`] reads.
    #[must_use]
    pub fn as_pairs(&self) -> Vec<(i32, bool)> {
        self.outcomes.iter().map(|o| (o.gate_no, o.passed)).collect()
    }

    /// The gates that could not be evaluated. Reported separately from the ones
    /// that failed on their merits, because they mean different things: one is a
    /// result about the candidate, the other is a hole in the evidence.
    #[must_use]
    pub fn inconclusive(&self) -> Vec<&GateOutcome> {
        self.outcomes.iter().filter(|o| o.inconclusive).collect()
    }
}

/// Run all sixteen.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn evaluate_stack(
    profile: &GateProfile,
    evidence: &StackEvidence<'_>,
    now: chrono::DateTime<chrono::Utc>,
) -> StackVerdict {
    let t = profile.thresholds();
    let mut outcomes: Vec<GateOutcome> = Vec::with_capacity(GATE_COUNT);

    // 1 — pre-registration. Structural: either the hypothesis was hash-locked
    // before the first backtest or it was not.
    outcomes.push(match (&evidence.prereg_hash, t.preregistration_required) {
        (Some(h), _) => GateOutcome::structural_pass(
            1,
            "preregistration",
            format!("hypothesis locked at {h}"),
        ),
        (None, true) => GateOutcome::inconclusive_pub(
            1,
            "preregistration",
            "no pre-registration hash: the claim was not written down before the work started",
        ),
        (None, false) => GateOutcome::structural_pass(
            1,
            "preregistration",
            "this profile does not require pre-registration",
        ),
    });

    // 2 — the leakage suite.
    outcomes.push(gate_2_leakage_suite(
        profile,
        evidence.leakage_run,
        evidence.leakage_checks,
        now,
        LEAKAGE_MAX_AGE,
    ));

    // 3 — cost sensitivity.
    outcomes.push(gate_3_cost_sensitivity(profile, evidence.cost_ladder));

    // 4 — capacity.
    outcomes.push(match evidence.capacity {
        Some(c) => gate_4_capacity(profile, c),
        None => GateOutcome::inconclusive_pub(
            4,
            "capacity_adv",
            "no ADV or AUM-ladder evidence was supplied",
        ),
    });

    // 5 — CPCV.
    outcomes.push(threshold_gate(
        5,
        "cpcv_p05",
        evidence.cpcv_p05_sharpe,
        t.cpcv_p05_sharpe_gt,
        Direction::Greater,
        "5th-percentile CPCV path Sharpe",
    ));

    // 6 — walk-forward, with the regime count Gate 11 measures.
    outcomes.push(match (evidence.walk_forward_sharpe, evidence.walk_forward_regimes) {
        (Some(sharpe), Some(regimes)) => {
            let passed = sharpe > t.walk_forward_sharpe_gt && regimes >= t.walk_forward_min_regimes;
            GateOutcome::decided_pub(
                6,
                "walk_forward",
                passed,
                sharpe,
                t.walk_forward_sharpe_gt,
                format!(
                    "strictly-causal walk-forward Sharpe {sharpe:.2} over {regimes} regimes \
                     (needs > {:.2} over ≥ {})",
                    t.walk_forward_sharpe_gt, t.walk_forward_min_regimes
                ),
            )
        }
        _ => GateOutcome::inconclusive_pub(
            6,
            "walk_forward",
            "no walk-forward result, or no regime count for it",
        ),
    });

    // 7 — probability of backtest overfitting.
    outcomes.push(threshold_gate(
        7,
        "pbo",
        evidence.pbo,
        t.pbo_lt,
        Direction::Less,
        "probability of backtest overfitting",
    ));

    // 8 — deflated Sharpe on the platform's own trial count.
    outcomes.push(threshold_gate(
        8,
        "deflated_sharpe",
        evidence.dsr,
        t.dsr_gte,
        Direction::GreaterOrEqual,
        "deflated Sharpe ratio",
    ));

    // 9 — minimum length.
    outcomes.push(match evidence.length {
        Some(l) => gate_9_min_length(profile, l),
        None => GateOutcome::inconclusive_pub(9, "min_length", "no track-record measurements"),
    });

    // 10 — factor attribution.
    outcomes.push(match evidence.factors {
        Some(f) => gate_10_factor_attribution(profile, evidence.asset_class, f),
        None => GateOutcome::inconclusive_pub(
            10,
            "factor_attribution",
            "no factor battery returns were supplied",
        ),
    });

    // 11 — regime coverage. The market series labels the regimes; the strategy's
    // attributes the P&L (ADR-P2-29).
    outcomes.push(gate_11_regime_coverage(
        profile,
        evidence.asset_class,
        evidence.market_returns,
        evidence.strategy_returns,
    ));

    // 12 — perturbation and concentration.
    outcomes.push(match evidence.perturbation {
        Some(p) => gate_12_perturbation(profile, p),
        None => GateOutcome::inconclusive_pub(
            12,
            "perturbation",
            "no neighbourhood study or per-instrument P&L",
        ),
    });

    // 13 / 14 — the ledger statistics over the stored series.
    outcomes.push(gate_13_stationary_bootstrap(
        profile,
        evidence.return_series,
        evidence.bootstrap_seed,
    ));
    outcomes.push(gate_14_romano_wolf(
        profile,
        evidence.family,
        evidence.candidate_index,
        evidence.bootstrap_seed,
    ));

    // 15 — paper/shadow.
    outcomes.push(match evidence.paper {
        Some(p) => gate_15_paper_shadow(profile, p),
        None => GateOutcome::inconclusive_pub(
            15,
            "paper_shadow",
            "no forward-test observations: this candidate has not been run in paper",
        ),
    });

    // 16 — the capital ramp, judged on the fifteen above.
    let so_far: Vec<(i32, bool)> = outcomes.iter().map(|o| (o.gate_no, o.passed)).collect();
    let mut with_16 = so_far.clone();
    with_16.push((16, true)); // 16 gates itself only on the other fifteen
    outcomes.push(gate_16_capital_ramp(profile, evidence.capital_fraction, &with_16));

    let blocked_at = outcomes.iter().find(|o| !o.passed).map(|o| o.gate_no);
    StackVerdict { outcomes, blocked_at }
}

/// Record every verdict, including the inconclusive ones.
///
/// # Errors
/// The first backend failure. Earlier verdicts are already written — the table
/// is append-only, so a partial stack is a partial stack and not a corrupted
/// one, and the caller retries the whole evaluation rather than patching it.
pub fn record_stack<L: GateLog + ?Sized>(
    log: &L,
    tenant_id: &str,
    profile: &GateProfile,
    evidence: &StackEvidence<'_>,
    verdict: &StackVerdict,
) -> Result<Vec<Uuid>, ledger::LedgerError> {
    let mut ids = Vec::with_capacity(verdict.outcomes.len());
    for outcome in &verdict.outcomes {
        let mut record: GateRecord = outcome.to_record(profile.profile_id());
        if let Some(exp) = &evidence.experiment_id {
            record = record.for_experiment(exp.clone());
        }
        if let Some(trial) = evidence.trial_id {
            record = record.for_trial(trial);
        }
        if let Some(campaign) = evidence.campaign_id {
            record = record.in_campaign(campaign);
        }
        // Gates 8 and 14 are significance gates: the database refuses a pass
        // from either without `N_eff` and the trial count beside it
        // (`chk_gate_significance_never_naked`). Attaching it to every record is
        // cheaper than remembering which two need it, and it makes every verdict
        // readable against how much searching preceded it.
        record = record.with_significance(evidence.n_eff, evidence.trial_count);
        ids.push(log.record_gate(tenant_id, &record)?);
    }
    Ok(ids)
}

/// Which direction a threshold is crossed in.
///
/// Named `Direction` rather than `Comparison` because `stats::compare::Comparison`
/// is the §11.5 verdict, and two types with one name in one crate is how the
/// wrong one gets used.
#[derive(Clone, Copy)]
enum Direction {
    Greater,
    GreaterOrEqual,
    Less,
}

/// A gate that is a single number against a single threshold. Absent evidence is
/// inconclusive, never a pass.
fn threshold_gate(
    gate_no: i32,
    name: &'static str,
    value: Option<f64>,
    threshold: f64,
    direction: Direction,
    label: &str,
) -> GateOutcome {
    let Some(v) = value.filter(|x| x.is_finite()) else {
        return GateOutcome::inconclusive_pub(gate_no, name, format!("no {label} was computed"));
    };
    let (passed, rel) = match direction {
        Direction::Greater => (v > threshold, ">"),
        Direction::GreaterOrEqual => (v >= threshold, "≥"),
        Direction::Less => (v < threshold, "<"),
    };
    GateOutcome::decided_pub(
        gate_no,
        name,
        passed,
        v,
        threshold,
        format!("{label} {v:.4} (needs {rel} {threshold:.4})"),
    )
}

/// Which regimes a set of labels covers. Re-exported for callers assembling
/// Gate 6's regime count from the same labelling Gate 11 uses, so the two
/// numbers cannot disagree.
#[must_use]
pub fn distinct_regimes(labels: &[VolRegime]) -> u32 {
    let mut seen = [false; 3];
    for l in labels {
        seen[match l {
            VolRegime::Low => 0,
            VolRegime::Mid => 1,
            VolRegime::High => 2,
        }] = true;
    }
    u32::try_from(seen.iter().filter(|s| **s).count()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use ledger::gates::PassRate;
    use ledger::LedgerError;
    use std::sync::Mutex;

    /// A `GateLog` that keeps what it was told.
    struct Recorder(Mutex<Vec<GateRecord>>);

    impl GateLog for Recorder {
        fn record_gate(&self, _tenant: &str, record: &GateRecord) -> Result<Uuid, LedgerError> {
            record.validate()?;
            self.0.lock().expect("recorder").push(record.clone());
            Ok(Uuid::new_v4())
        }
        fn pass_rate(&self, _t: &str, _p: &str, _w: i64) -> Result<PassRate, LedgerError> {
            unimplemented!("not exercised here")
        }
    }

    fn empty_evidence<'a>(checks: &'a [&'a str]) -> StackEvidence<'a> {
        StackEvidence {
            prereg_hash: None,
            leakage_run: None,
            leakage_checks: checks,
            cost_ladder: &[],
            capacity: None,
            cpcv_p05_sharpe: None,
            walk_forward_sharpe: None,
            walk_forward_regimes: None,
            pbo: None,
            dsr: None,
            length: None,
            factors: None,
            market_returns: &[],
            strategy_returns: &[],
            perturbation: None,
            return_series: &[],
            family: &[],
            candidate_index: 0,
            bootstrap_seed: 7,
            paper: None,
            capital_fraction: ledger::AllowedFraction::default(),
            n_eff: 12.0,
            trial_count: 40,
            asset_class: "crypto",
            experiment_id: Some("exp-1".into()),
            trial_id: None,
            campaign_id: None,
        }
    }

    /// The stack always produces sixteen verdicts, whatever it was given. A
    /// stack that quietly shrinks when evidence stops arriving would make the
    /// pass rate go *up* as the platform got worse.
    #[test]
    fn the_stack_is_always_sixteen_gates_long() {
        let checks = ["causal_access"];
        let verdict = evaluate_stack(&GateProfile::strict_v1(), &empty_evidence(&checks), Utc::now());
        assert_eq!(verdict.outcomes.len(), GATE_COUNT);
        let numbers: Vec<i32> = verdict.outcomes.iter().map(|o| o.gate_no).collect();
        assert_eq!(numbers, (1..=16).collect::<Vec<_>>(), "in §12.3's order");
    }

    /// Supplying nothing passes nothing. This is the property that makes the
    /// stack un-gameable by omission.
    #[test]
    fn no_evidence_passes_no_gate() {
        let checks = ["causal_access"];
        let verdict = evaluate_stack(&GateProfile::strict_v1(), &empty_evidence(&checks), Utc::now());
        for o in &verdict.outcomes {
            assert!(!o.passed, "gate {} passed on no evidence: {}", o.gate_no, o.detail);
        }
        assert_eq!(verdict.blocked_at, Some(1));
        assert!(!verdict.passed());
        // And most of them say *why* they could not run, rather than reading as
        // a judgement about the candidate.
        assert!(verdict.inconclusive().len() >= 10, "{:?}", verdict.inconclusive().len());
    }

    /// Every verdict reaches the ledger, including the inconclusive ones, and
    /// each carries the significance context the database requires.
    #[test]
    fn every_verdict_is_recorded_with_its_trial_count() {
        let checks = ["causal_access"];
        let evidence = empty_evidence(&checks);
        let profile = GateProfile::strict_v1();
        let verdict = evaluate_stack(&profile, &evidence, Utc::now());

        let log = Recorder(Mutex::new(Vec::new()));
        let ids = record_stack(&log, "t", &profile, &evidence, &verdict).expect("recorded");
        assert_eq!(ids.len(), GATE_COUNT);

        let written = log.0.lock().expect("recorder");
        assert_eq!(written.len(), GATE_COUNT);
        for r in written.iter() {
            assert_eq!(r.experiment_id.as_deref(), Some("exp-1"));
            assert_eq!(r.profile_id, "strict_v1");
            assert!(
                r.n_eff.is_some() && r.trial_count_at_eval == Some(40),
                "gate {} was recorded without the searching that preceded it",
                r.gate_no
            );
        }
    }

    /// The capital ramp does not move on a stack that did not pass, and the
    /// gate that says so is the sixteenth of sixteen.
    #[test]
    fn gate_16_does_not_move_capital_on_a_failed_stack() {
        let checks = ["causal_access"];
        let verdict = evaluate_stack(&GateProfile::strict_v1(), &empty_evidence(&checks), Utc::now());
        let sixteen = verdict.outcomes.last().expect("sixteen");
        assert_eq!(sixteen.gate_no, 16);
        assert!(!sixteen.passed);
        assert!((sixteen.statistic.unwrap() - 0.0).abs() < f64::EPSILON, "{}", sixteen.detail);
    }

    #[test]
    fn a_threshold_gate_reads_its_number_and_its_absence_differently() {
        let checks = ["causal_access"];
        let mut evidence = empty_evidence(&checks);
        evidence.pbo = Some(0.05);
        let verdict = evaluate_stack(&GateProfile::strict_v1(), &evidence, Utc::now());
        let seven = verdict.outcomes.iter().find(|o| o.gate_no == 7).expect("gate 7");
        assert!(seven.passed, "{}", seven.detail);
        assert!(!seven.inconclusive);

        evidence.pbo = Some(0.9);
        let failed = evaluate_stack(&GateProfile::strict_v1(), &evidence, Utc::now());
        let seven = failed.outcomes.iter().find(|o| o.gate_no == 7).expect("gate 7");
        assert!(!seven.passed && !seven.inconclusive, "{}", seven.detail);
    }

    #[test]
    fn distinct_regimes_counts_what_was_actually_seen() {
        use super::super::evaluators::VolRegime::{High, Low, Mid};
        assert_eq!(distinct_regimes(&[Low, Low, Low]), 1);
        assert_eq!(distinct_regimes(&[Low, Mid, Low, High]), 3);
        assert_eq!(distinct_regimes(&[]), 0);
    }

    #[test]
    fn a_stale_leakage_run_does_not_gate_a_fresh_candidate() {
        let checks = ["causal_access", "random_label"];
        let old = LeakageRun {
            subject: "ds:abc".into(),
            checks_run: checks.iter().map(ToString::to_string).collect(),
            blocking_count: 0,
            flag_count: 0,
            finished_at: Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap(),
        };
        let mut evidence = empty_evidence(&checks);
        evidence.leakage_run = Some(&old);
        let verdict = evaluate_stack(&GateProfile::strict_v1(), &evidence, Utc::now());
        let two = verdict.outcomes.iter().find(|o| o.gate_no == 2).expect("gate 2");
        assert!(two.inconclusive && !two.passed, "{}", two.detail);
    }
}
