//! The staged-gate funnel — a funnel, not a menu (spec §2.2).
//!
//! Gates are ordered by cost-per-unit-of-discriminating-power: cheap high-power
//! filters first, expensive tests only on survivors. A gate's *entry* requires
//! the prior gate's **pass verdict**, which only exists if its Studies ran (D-8) —
//! so the funnel cannot be skipped or reordered, and the vault (Gate 4) is
//! reachable only after Gate 3.
//!
//! ```text
//! GATE 0 integrity → GATE 1 single-path → GATE 2 robustness → GATE 3 significance → GATE 4 vault
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::experiment::{Experiment, ExperimentError};
use crate::nulls::{NullId, SignificanceResult};
use crate::run::{Backtest, RunConfig, RunExecutor, RunResult, RunStatus, RunStore};
use crate::stats::{
    deflated_sharpe_ratio, permutation_p_value, probability_of_backtest_overfitting,
    selection_bias_correction,
};
use crate::study::StudyResult;

/// The five gates, in funnel order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    Integrity,
    SinglePath,
    Robustness,
    Significance,
    Vault,
}

impl Gate {
    /// The gate that must have passed before this one may be entered.
    #[must_use]
    pub fn prerequisite(self) -> Option<Gate> {
        match self {
            Gate::Integrity => None,
            Gate::SinglePath => Some(Gate::Integrity),
            Gate::Robustness => Some(Gate::SinglePath),
            Gate::Significance => Some(Gate::Robustness),
            Gate::Vault => Some(Gate::Significance),
        }
    }
}

/// A recorded gate outcome (persisted to `backtest_gate_verdicts`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateVerdict {
    pub gate: Gate,
    pub passed: bool,
    pub summary: String,
    /// Study/run ids that constitute the evidence for this verdict.
    pub evidence: Vec<String>,
    pub at: DateTime<Utc>,
}

impl GateVerdict {
    fn new(gate: Gate, passed: bool, summary: impl Into<String>, evidence: Vec<String>) -> Self {
        Self {
            gate,
            passed,
            summary: summary.into(),
            evidence,
            at: Utc::now(),
        }
    }
}

/// Why a gate could not be entered or run.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GateError {
    #[error("cannot enter {gate:?}: prerequisite {required:?} has not passed")]
    PrerequisiteNotPassed { gate: Gate, required: Gate },
    #[error("gate 0 integrity failed — hard stop")]
    IntegrityHardStop,
    #[error("experiment error: {0}")]
    Experiment(ExperimentError),
}

/// The append-only record of gate verdicts for one Experiment.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GateLedger {
    verdicts: Vec<GateVerdict>,
}

impl GateLedger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `gate` has a passing verdict recorded.
    #[must_use]
    pub fn passed(&self, gate: Gate) -> bool {
        self.verdicts.iter().any(|v| v.gate == gate && v.passed)
    }

    fn record(&mut self, verdict: GateVerdict) {
        self.verdicts.push(verdict);
    }

    #[must_use]
    pub fn verdicts(&self) -> &[GateVerdict] {
        &self.verdicts
    }
}

// ── Gate 0 integrity inputs (spec §2.2 Gate 0) ───────────────────────────────

/// One higher-timeframe signal observation: when the strategy *acted* on it vs
/// when the constituent bar actually *closed*. Acting before the close leaks the
/// whole bar (the single most likely real-world leak in a 1m-constructed stack).
#[derive(Clone, Copy, Debug)]
pub struct SignalStamp {
    pub acted_at_ns: i64,
    pub bar_close_ns: i64,
}

/// Everything Gate 0 inspects, on every config.
pub struct IntegrityInputs<'a> {
    /// Higher-timeframe signal stamps (close-stamped leak scan).
    pub signals: &'a [SignalStamp],
    /// Gross edge (return before costs) and the minimum realistic cost floor.
    pub gross_return: f64,
    pub cost_floor: f64,
    /// Label horizon vs feature-window end (model strategies); `purge`d bars.
    /// `None` for non-model strategies.
    pub label_horizon_bars: Option<i64>,
    pub feature_window_end_bar: Option<i64>,
    pub purge_bars: Option<i64>,
}

/// Result of the Gate 0 scan: the flags it raised (empty == clean).
#[must_use]
pub fn integrity_scan(inputs: &IntegrityInputs<'_>) -> Vec<crate::run::Flag> {
    use crate::run::Flag;
    let mut flags = Vec::new();

    // (a) Close-stamped leakage: every higher-TF signal must be acted on at or
    // after its constituent bar's close (ADR-0008 available_time ordering).
    for (i, s) in inputs.signals.iter().enumerate() {
        if s.acted_at_ns < s.bar_close_ns {
            flags.push(Flag {
                code: "lookahead.higher_tf_open".into(),
                detail: format!(
                    "signal #{i} acted at {} before its bar closed at {} (leaked the whole bar)",
                    s.acted_at_ns, s.bar_close_ns
                ),
            });
        }
    }

    // (b) Cost sanity: the gross edge must clear the floor cost model.
    if inputs.gross_return <= inputs.cost_floor {
        flags.push(Flag {
            code: "cost.below_floor".into(),
            detail: format!(
                "gross edge {:.4} does not exceed the cost floor {:.4}",
                inputs.gross_return, inputs.cost_floor
            ),
        });
    }

    // (c) Label look-ahead: a label horizon overlapping the feature window
    // without sufficient purge leaks the future into the features.
    if let (Some(h), Some(fend), Some(purge)) = (
        inputs.label_horizon_bars,
        inputs.feature_window_end_bar,
        inputs.purge_bars,
    ) {
        // The label spans [decision, decision + h]; the feature window ends at
        // fend. If the gap between them is smaller than the horizon and the
        // purge does not cover it, they overlap.
        if h > purge && fend >= -purge {
            flags.push(Flag {
                code: "lookahead.label_overlap".into(),
                detail: format!(
                    "label horizon {h} exceeds purge {purge} with feature window end {fend} — overlap"
                ),
            });
        }
    }

    flags
}

/// Drives an Experiment through the gate funnel, enforcing ordering (D-8) and
/// recording every verdict.
pub struct GateRunner<'e> {
    experiment: &'e mut Experiment,
    ledger: GateLedger,
}

/// The full Gate 3 outcome: one primary p-value + two corroborators.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gate3Outcome {
    /// The primary, selection-bias-corrected significance (INV-3).
    pub significance: SignificanceResult,
    /// The raw (uncorrected) permutation p-value, for transparency.
    pub raw_p_value: f64,
    /// Deflated Sharpe Ratio corroborator (probability in [0,1]).
    pub deflated_sharpe: f64,
    /// Probability of Backtest Overfitting corroborator.
    pub pbo: f64,
    /// Whether the corroborators agree with the primary verdict. Disagreement is
    /// a flag to investigate, not a vote to break.
    pub corroborators_agree: bool,
}

/// Inputs for the Gate 3 corroborators.
pub struct CorroboratorInputs<'a> {
    pub sharpe: f64,
    pub n_obs: usize,
    pub skew: f64,
    pub kurtosis: f64,
    pub sharpe_variance_across_trials: f64,
    /// `performance[config][period]` for the PBO (CSCV) computation.
    pub pbo_performance: &'a [Vec<f64>],
    pub pbo_groups: usize,
}

impl<'e> GateRunner<'e> {
    #[must_use]
    pub fn new(experiment: &'e mut Experiment) -> Self {
        Self {
            experiment,
            ledger: GateLedger::new(),
        }
    }

    #[must_use]
    pub fn ledger(&self) -> &GateLedger {
        &self.ledger
    }

    /// Check that a gate's prerequisite has passed before it may be entered.
    fn enter(&self, gate: Gate) -> Result<(), GateError> {
        if let Some(required) = gate.prerequisite() {
            if !self.ledger.passed(required) {
                return Err(GateError::PrerequisiteNotPassed { gate, required });
            }
        }
        Ok(())
    }

    /// GATE 0 — integrity. Runs on every config; failure is a **hard stop**
    /// (no Gate 1). The integrity findings are returned so the caller can set
    /// `RunResult.status = rejected_integrity` and store the (counted) run.
    pub fn gate0(&mut self, inputs: &IntegrityInputs<'_>) -> Result<&GateVerdict, GateError> {
        self.enter(Gate::Integrity)?;
        let flags = integrity_scan(inputs);
        let passed = flags.is_empty();
        let summary = if passed {
            "integrity clean: close-stamped, gross edge clears the cost floor, no label overlap".into()
        } else {
            format!("{} integrity violation(s): {}", flags.len(), flags[0].code)
        };
        self.ledger
            .record(GateVerdict::new(Gate::Integrity, passed, summary, vec![]));
        if !passed {
            return Err(GateError::IntegrityHardStop);
        }
        Ok(self.ledger.verdicts.last().unwrap())
    }

    /// GATE 1 — single-path sanity. One honest walk-forward under the pessimistic
    /// cost model; pass iff the OOS distribution's median is positive.
    pub fn gate1(&mut self, walk_forward: &StudyResult) -> Result<&GateVerdict, GateError> {
        self.enter(Gate::SinglePath)?;
        let passed = walk_forward.verdict.positive_median;
        let summary = format!(
            "single honest walk-forward (pessimistic costs): median {:.4} ({})",
            walk_forward.distribution.median,
            if passed { "profitable" } else { "not profitable — stop" }
        );
        self.ledger.record(GateVerdict::new(
            Gate::SinglePath,
            passed,
            summary,
            vec![walk_forward.study_id.clone()],
        ));
        Ok(self.ledger.verdicts.last().unwrap())
    }

    /// GATE 2 — robustness. Pass iff the CPCV OOS distribution is positive at the
    /// median AND survivable at the worst-5%, AND the neighborhood is a plateau
    /// (not an isolated spike). A *shape*, not a number (spec §2.2 Gate 2).
    pub fn gate2(
        &mut self,
        cpcv: &StudyResult,
        synthetic: &StudyResult,
        neighborhood: &StudyResult,
        worst5_threshold: f64,
    ) -> Result<&GateVerdict, GateError> {
        self.enter(Gate::Robustness)?;
        let positive_median = cpcv.distribution.median > 0.0;
        let survivable = cpcv.distribution.worst_5pct >= worst5_threshold
            && synthetic.distribution.worst_5pct >= worst5_threshold;
        let plateau = neighborhood.verdict.plateau.unwrap_or(false);
        let passed = positive_median && survivable && plateau;
        let summary = format!(
            "robustness: cpcv median {:.4}, worst-5% {:.4}; synthetic worst-5% {:.4}; neighborhood {} → {}",
            cpcv.distribution.median,
            cpcv.distribution.worst_5pct,
            synthetic.distribution.worst_5pct,
            if plateau { "plateau" } else { "spike" },
            if passed { "robust" } else { "fragile — stop" }
        );
        self.ledger.record(GateVerdict::new(
            Gate::Robustness,
            passed,
            summary,
            vec![
                cpcv.study_id.clone(),
                synthetic.study_id.clone(),
                neighborhood.study_id.clone(),
            ],
        ));
        Ok(self.ledger.verdicts.last().unwrap())
    }

    /// GATE 3 — significance. The single primary permutation test against the
    /// Experiment's declared null, selection-bias-corrected by the live trial
    /// counter, plus DSR + PBO corroborators. On pass, marks the Experiment
    /// Gate-3-passed (the vault precondition). `alpha` is the corrected-p
    /// threshold (e.g. 0.05).
    pub fn gate3(
        &mut self,
        observed_statistic: f64,
        null_distribution: &[f64],
        null_id: NullId,
        corroborators: &CorroboratorInputs<'_>,
        alpha: f64,
    ) -> Result<(Gate3Outcome, bool), GateError> {
        self.enter(Gate::Significance)?;

        let trials = self.experiment.trial_counter();
        let raw_p = permutation_p_value(observed_statistic, null_distribution);
        let corrected_p = selection_bias_correction(raw_p, trials);
        let significance = SignificanceResult::new(corrected_p, null_id, trials);

        let dsr = deflated_sharpe_ratio(
            corroborators.sharpe,
            corroborators.n_obs,
            corroborators.skew,
            corroborators.kurtosis,
            trials,
            corroborators.sharpe_variance_across_trials,
        );
        let pbo = probability_of_backtest_overfitting(
            corroborators.pbo_performance,
            corroborators.pbo_groups,
        );

        let primary_significant = corrected_p <= alpha;
        // Corroborators "agree" with a significant primary when DSR is high and
        // PBO is low (and vice-versa for a non-significant primary).
        let corroborators_significant = dsr >= 0.95 && pbo <= 0.5;
        let corroborators_agree = primary_significant == corroborators_significant;

        // The gate passes only when the primary is significant AND the
        // corroborators agree. Disagreement is a flag to investigate, not a vote.
        let passed = primary_significant && corroborators_agree;
        let summary = format!(
            "significance: {} | DSR={:.3} PBO={:.3} | corroborators {}",
            significance.render(),
            dsr,
            pbo,
            if corroborators_agree { "agree" } else { "DISAGREE — investigate" }
        );
        self.ledger.record(GateVerdict::new(
            Gate::Significance,
            passed,
            summary,
            vec![significance.null_ref().to_string()],
        ));
        if passed {
            self.experiment.mark_gate3_passed();
        }

        let outcome = Gate3Outcome {
            significance,
            raw_p_value: raw_p,
            deflated_sharpe: dsr,
            pbo,
            corroborators_agree,
        };
        Ok((outcome, passed))
    }

    /// GATE 4 — the vault. Reachable only after a passing Gate 3. Runs the
    /// candidate config once against the locked holdout; on a successful
    /// evaluation the Experiment becomes `validated`.
    pub fn gate4<S: RunStore, E: RunExecutor>(
        &mut self,
        candidate: &RunConfig,
        bt: &Backtest<S, E>,
        by: impl Into<String>,
    ) -> Result<(RunResult, &GateVerdict), GateError> {
        self.enter(Gate::Vault)?;
        let result = self
            .experiment
            .run_vault(candidate, bt, by)
            .map_err(GateError::Experiment)?;
        let passed = result.status == RunStatus::Ok;
        let summary = format!(
            "vault: single holdout evaluation → {:?} ({})",
            result.status,
            if passed { "validated" } else { "dead for this holdout" }
        );
        self.ledger.record(GateVerdict::new(
            Gate::Vault,
            passed,
            summary,
            vec![result.run_id.to_string()],
        ));
        Ok((result, self.ledger.verdicts.last().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::executor::{daily_curve, map_sim_result};
    use crate::run::{
        Backtest, ClosureExecutor, ComputeCost, DataSlice, EvalResolution, InMemoryRunStore,
        MetricKind, ParamMap, RunConfig, RunConfigBuilder, ENGINE_VERSION,
    };
    use crate::study::{Distribution, StudyResult, StudyVerdict};
    use chrono::TimeZone;

    fn verdict(positive_median: bool, plateau: Option<bool>) -> StudyVerdict {
        StudyVerdict {
            summary: "v".into(),
            positive_median,
            survivable_worst5: true,
            plateau,
        }
    }

    fn study(id: &str, values: Vec<f64>, v: StudyVerdict) -> StudyResult {
        StudyResult::new(
            id.into(),
            vec![],
            Distribution::from_values(MetricKind::DetrendedSharpe, values),
            v,
            0,
            None,
            false,
        )
    }

    fn experiment() -> Experiment {
        let holdout = DataSlice::new(
            "u",
            Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            EvalResolution::Day1,
        );
        Experiment::new("exp", "fam", holdout, "null:block")
    }

    fn clean_integrity() -> IntegrityInputs<'static> {
        IntegrityInputs {
            signals: &[],
            gross_return: 0.10,
            cost_floor: 0.01,
            label_horizon_bars: None,
            feature_window_end_bar: None,
            purge_bars: None,
        }
    }

    #[test]
    fn close_stamp_leak_is_caught() {
        let leaky = [SignalStamp { acted_at_ns: 100, bar_close_ns: 200 }];
        let inputs = IntegrityInputs {
            signals: &leaky,
            gross_return: 0.10,
            cost_floor: 0.01,
            label_horizon_bars: None,
            feature_window_end_bar: None,
            purge_bars: None,
        };
        let flags = integrity_scan(&inputs);
        assert!(flags.iter().any(|f| f.code == "lookahead.higher_tf_open"));
    }

    #[test]
    fn cost_floor_death_is_caught() {
        let inputs = IntegrityInputs {
            signals: &[],
            gross_return: 0.005,
            cost_floor: 0.01,
            ..clean_integrity()
        };
        let flags = integrity_scan(&inputs);
        assert!(flags.iter().any(|f| f.code == "cost.below_floor"));
    }

    #[test]
    fn funnel_cannot_be_skipped() {
        let mut e = experiment();
        let mut runner = GateRunner::new(&mut e);
        // Entering Gate 1 before Gate 0 passes is refused.
        let wf = study("wf", vec![0.1, 0.2], verdict(true, None));
        assert_eq!(
            runner.gate1(&wf).err(),
            Some(GateError::PrerequisiteNotPassed {
                gate: Gate::SinglePath,
                required: Gate::Integrity,
            })
        );
    }

    #[test]
    fn gate0_hard_stops_on_leak() {
        let mut e = experiment();
        let mut runner = GateRunner::new(&mut e);
        let leaky = [SignalStamp { acted_at_ns: 1, bar_close_ns: 2 }];
        let inputs = IntegrityInputs { signals: &leaky, ..clean_integrity() };
        assert_eq!(runner.gate0(&inputs).err(), Some(GateError::IntegrityHardStop));
        assert!(!runner.ledger().passed(Gate::Integrity));
    }

    #[test]
    fn full_funnel_reaches_the_vault_only_in_order() {
        let mut e = experiment();
        let bt = Backtest::new(
            InMemoryRunStore::new(),
            ClosureExecutor(|cfg: &RunConfig| {
                map_sim_result(cfg, daily_curve(&[100.0, 101.0, 103.0]), vec![], vec![], ComputeCost::default(), ENGINE_VERSION)
            }),
        );
        let candidate = RunConfigBuilder::new(
            "s",
            "v",
            DataSlice::new(
                "u",
                Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap(),
                Utc.with_ymd_and_hms(2022, 1, 1, 0, 0, 0).unwrap(),
                EvalResolution::Day1,
            ),
            "c",
            "z",
            "snap",
        )
        .build();

        let mut runner = GateRunner::new(&mut e);
        // Vault before Gate 3 is refused.
        assert!(matches!(
            runner.gate4(&candidate, &bt, "a"),
            Err(GateError::PrerequisiteNotPassed { gate: Gate::Vault, required: Gate::Significance })
        ));

        runner.gate0(&clean_integrity()).unwrap();
        runner.gate1(&study("wf", vec![0.1, 0.2, 0.15], verdict(true, None))).unwrap();
        runner
            .gate2(
                &study("cpcv", vec![0.1, 0.2, 0.15, 0.12], verdict(true, None)),
                &study("syn", vec![0.08, 0.12, 0.10], verdict(true, None)),
                &study("nbhd", vec![0.10, 0.11, 0.10, 0.11], verdict(true, Some(true))),
                -0.5,
            )
            .unwrap();

        // Strong signal, few trials → significant; corroborators agree.
        let null: Vec<f64> = (0..999).map(|i| f64::from(i) / 1000.0).collect();
        let perf = vec![vec![1.0; 8], vec![0.1; 8], vec![0.2; 8]];
        let (outcome, passed) = runner
            .gate3(
                5.0,
                &null,
                crate::nulls::Null::new(crate::nulls::NullKind::BlockPermutation, crate::nulls::NullParams::default())
                    .unwrap()
                    .null_id,
                &CorroboratorInputs {
                    sharpe: 2.5,
                    n_obs: 252,
                    skew: 0.0,
                    kurtosis: 3.0,
                    sharpe_variance_across_trials: 0.1,
                    pbo_performance: &perf,
                    pbo_groups: 4,
                },
                0.05,
            )
            .unwrap();
        assert!(passed, "strong + few trials should pass: {}", outcome.significance.render());
        assert!(runner.ledger().passed(Gate::Significance));

        let (vault_result, v) = runner.gate4(&candidate, &bt, "alice").unwrap();
        assert_eq!(vault_result.status, RunStatus::Ok);
        assert!(v.passed);
        assert_eq!(e.state, crate::experiment::ExperimentState::Validated);
    }

    #[test]
    fn gate3_corroborator_disagreement_blocks_pass() {
        let mut e = experiment();
        // Inflate the trial counter so a single permutation p won't survive
        // correction, while the (small-sample) PBO matrix says "fine" — a
        // constructed disagreement.
        let mut runner = GateRunner::new(&mut e);
        runner.gate0(&clean_integrity()).unwrap();
        runner.gate1(&study("wf", vec![0.1], verdict(true, None))).unwrap();
        runner
            .gate2(
                &study("cpcv", vec![0.1, 0.2], verdict(true, None)),
                &study("syn", vec![0.1], verdict(true, None)),
                &study("nbhd", vec![0.1, 0.1], verdict(true, Some(true))),
                -1.0,
            )
            .unwrap();
        // A weak observed statistic against the null → large raw p → not significant.
        let null: Vec<f64> = (0..99).map(f64::from).collect();
        let perf = vec![vec![1.0; 6], vec![0.9; 6]];
        let (_outcome, passed) = runner
            .gate3(
                -10.0, // far below the null → p≈1, not significant
                &null,
                crate::nulls::Null::new(crate::nulls::NullKind::SignalReturnDecouple, crate::nulls::NullParams::default())
                    .unwrap()
                    .null_id,
                &CorroboratorInputs {
                    sharpe: 3.0, // but DSR says strong → disagreement with primary
                    n_obs: 252,
                    skew: 0.0,
                    kurtosis: 3.0,
                    sharpe_variance_across_trials: 0.01,
                    pbo_performance: &perf,
                    pbo_groups: 2,
                },
                0.05,
            )
            .unwrap();
        assert!(!passed, "a non-significant primary must not pass even if a corroborator is strong");
    }
}
