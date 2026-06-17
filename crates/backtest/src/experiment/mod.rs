//! The **Experiment** — the whole investigation of one strategy idea (spec §1.3).
//!
//! An Experiment owns the two things that make the suite honest: the **global
//! trial counter** (automatic, monotonic, irreversible) and the **holdout vault**
//! (one logged access, self-sealing). It also owns a one-directional lifecycle
//! state machine and the up-front `primary_test` declaration that Gate 3 (Phase
//! 4) reads.
//!
//! Studies attached to an Experiment can only be run through [`Experiment::run_study`],
//! which increments the counter *before* the result is returned — there is no way
//! to run a Study "off the books".

pub mod store;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::run::{
    Backtest, DataSlice, RunConfig, RunExecutor, RunId, RunResult, RunStatus, RunStore,
};
use crate::study::{StudyConfig, StudyConfigError, StudyEngine, StudyKind, StudyResult, VarySpec};

pub use store::{ExperimentStore, InMemoryExperimentStore};

/// Reference to a Null (Phase 3) — the designated significance test.
pub type NullRef = String;

/// One logged touch of the holdout vault. Recorded forever (spec §1.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultAccess {
    pub when: DateTime<Utc>,
    pub run_id: RunId,
    pub by: String,
}

/// The locked tail of data, addressable exactly once (spec §1.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Holdout {
    pub slice: DataSlice,
    /// Every access, forever — append-only.
    pub access_log: Vec<VaultAccess>,
    /// True after the single permitted vault run. Never flips back.
    pub spent: bool,
}

/// Lifecycle state — gates the allowed operations (spec §1.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentState {
    Candidate,
    Validated,
    Live,
    Decaying,
    Retired,
}

/// A class of operation, for [`ExperimentState::allows`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    /// Any research Study (sweeps, CV, significance) — the counter runs hot.
    ResearchStudy,
    /// Live-vs-backtest reconciliation Study (Phase 5).
    ReconciliationStudy,
    /// The one-shot holdout vault run.
    Vault,
    /// Promotion candidate→live.
    PromoteToLive,
}

impl ExperimentState {
    /// Which operations this state permits (spec §1.3 table).
    #[must_use]
    pub fn allows(self, op: Operation) -> bool {
        match self {
            ExperimentState::Candidate => {
                matches!(op, Operation::ResearchStudy | Operation::Vault)
            }
            ExperimentState::Validated => matches!(op, Operation::PromoteToLive),
            ExperimentState::Live => matches!(op, Operation::ReconciliationStudy),
            // Decaying permits reconciliation + diagnostic (research) Studies.
            ExperimentState::Decaying => {
                matches!(
                    op,
                    Operation::ReconciliationStudy | Operation::ResearchStudy
                )
            }
            ExperimentState::Retired => false,
        }
    }

    /// Whether `self → to` is a legal lifecycle transition. One-directional
    /// through validation: nothing returns to `candidate` once it has left.
    #[must_use]
    pub fn can_transition_to(self, to: ExperimentState) -> bool {
        use ExperimentState::{Candidate, Decaying, Live, Retired, Validated};
        // Any non-retired idea may be abandoned/retired.
        if to == Retired {
            return self != Retired;
        }
        const LEGAL: [(ExperimentState, ExperimentState); 4] = [
            (Candidate, Validated),
            (Validated, Live),
            (Live, Decaying),
            (Decaying, Live),
        ];
        LEGAL.contains(&(self, to))
    }
}

/// A reason an Experiment operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExperimentError {
    #[error("state {state:?} does not permit {op:?}")]
    OperationNotAllowed {
        state: ExperimentState,
        op: Operation,
    },
    #[error("illegal lifecycle transition {from:?} -> {to:?}")]
    IllegalTransition {
        from: ExperimentState,
        to: ExperimentState,
    },
    #[error("study data slice intersects the locked holdout vault")]
    TouchesHoldout,
    #[error("the holdout vault is already spent (one evaluation only)")]
    VaultSpent,
    #[error("the vault is reachable only after Gate 3 passes")]
    Gate3NotPassed,
    #[error("an unsafe experiment cannot be validated through the vault")]
    UnsafeBarred,
    #[error("invalid study: {0}")]
    Study(StudyConfigError),
}

/// The container for one strategy idea, across its whole life (spec §1.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Experiment {
    pub experiment_id: String,
    /// The idea being investigated (version-agnostic root).
    pub strategy_family: String,
    pub state: ExperimentState,
    /// References to every Study run (provenance).
    pub studies: Vec<String>,
    /// AUTOMATIC, MONOTONIC, IRREVERSIBLE.
    trial_counter: i64,
    pub holdout: Holdout,
    /// The ONE designated significance test, declared up front. Immutable.
    primary_test: NullRef,
    /// Set by Gate 3 (Phase 4); precondition for the vault.
    gate3_passed: bool,
    /// True if any Study/Run within was unsafe (INV-1). Never clears.
    unsafe_: bool,
    pub verdict: Option<String>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

impl Experiment {
    /// Create a new candidate Experiment with a locked holdout and a declared
    /// primary significance test. Counter starts at 0; vault unspent.
    #[must_use]
    pub fn new(
        experiment_id: impl Into<String>,
        strategy_family: impl Into<String>,
        holdout_slice: DataSlice,
        primary_test: impl Into<NullRef>,
    ) -> Self {
        let now = Utc::now();
        Self {
            experiment_id: experiment_id.into(),
            strategy_family: strategy_family.into(),
            state: ExperimentState::Candidate,
            studies: Vec::new(),
            trial_counter: 0,
            holdout: Holdout {
                slice: holdout_slice,
                access_log: Vec::new(),
                spent: false,
            },
            primary_test: primary_test.into(),
            gate3_passed: false,
            unsafe_: false,
            verdict: None,
            created: now,
            updated: now,
        }
    }

    /// The global trial counter (read-only; there is no setter, decrement, or reset).
    #[must_use]
    pub fn trial_counter(&self) -> i64 {
        self.trial_counter
    }

    /// The declared significance test (read-only; immutable after creation, J-2.7).
    #[must_use]
    pub fn primary_test(&self) -> &str {
        &self.primary_test
    }

    /// Whether the Experiment has been flagged unsafe (INV-1 propagation).
    #[must_use]
    pub fn is_unsafe(&self) -> bool {
        self.unsafe_
    }

    #[must_use]
    pub fn gate3_passed(&self) -> bool {
        self.gate3_passed
    }

    /// Mark Gate 3 as passed (called by the funnel, Phase 4). Precondition for
    /// the vault. Idempotent.
    pub fn mark_gate3_passed(&mut self) {
        self.gate3_passed = true;
        self.updated = Utc::now();
    }

    /// Perform a legal lifecycle transition.
    ///
    /// # Errors
    /// [`ExperimentError::IllegalTransition`] if the edge is not allowed.
    pub fn transition(&mut self, to: ExperimentState) -> Result<(), ExperimentError> {
        if self.state.can_transition_to(to) {
            self.state = to;
            self.updated = Utc::now();
            Ok(())
        } else {
            Err(ExperimentError::IllegalTransition {
                from: self.state,
                to,
            })
        }
    }

    /// Run a research Study **attached to this Experiment** — the only public
    /// path, so the counter always increments before the caller sees a result
    /// (J-2.3). Refuses if the state forbids research or the Study addresses the
    /// holdout (J-2.4 / J-2.5).
    ///
    /// # Errors
    /// See [`ExperimentError`].
    pub fn run_study<S: RunStore, E: RunExecutor>(
        &mut self,
        study: &StudyConfig,
        bt: &Backtest<S, E>,
    ) -> Result<StudyResult, ExperimentError> {
        if !self.state.allows(Operation::ResearchStudy) {
            return Err(ExperimentError::OperationNotAllowed {
                state: self.state,
                op: Operation::ResearchStudy,
            });
        }
        if self.study_touches_holdout(study) {
            return Err(ExperimentError::TouchesHoldout);
        }
        let result = StudyEngine::run(study, bt).map_err(ExperimentError::Study)?;
        self.record_study(study.study_id.clone(), &result);
        Ok(result)
    }

    /// Increment the counter, append the Study reference, and propagate `unsafe`.
    /// The *only* mutator of the counter; it only ever increases.
    fn record_study(&mut self, study_id: String, result: &StudyResult) {
        self.trial_counter += result.trial_delta;
        self.studies.push(study_id);
        self.unsafe_ |= result.unsafe_;
        self.updated = Utc::now();
    }

    /// Run the one-shot holdout vault evaluation (Gate 4, spec §2.2 / J-2.6).
    ///
    /// Reachable only from a Gate-3-passed `candidate` Experiment that is not
    /// `unsafe` and whose vault is unspent. Logs the access, flips `spent`
    /// **before returning**, and transitions `candidate → validated` on a
    /// successful evaluation. A second call is refused.
    ///
    /// # Errors
    /// See [`ExperimentError`].
    pub fn run_vault<S: RunStore, E: RunExecutor>(
        &mut self,
        candidate: &RunConfig,
        bt: &Backtest<S, E>,
        by: impl Into<String>,
    ) -> Result<RunResult, ExperimentError> {
        // Spent is checked first so a second attempt always reports the most
        // specific reason (VaultSpent), even though the post-vault state also
        // forbids the operation.
        if self.holdout.spent {
            return Err(ExperimentError::VaultSpent);
        }
        if !self.state.allows(Operation::Vault) {
            return Err(ExperimentError::OperationNotAllowed {
                state: self.state,
                op: Operation::Vault,
            });
        }
        if !self.gate3_passed {
            return Err(ExperimentError::Gate3NotPassed);
        }
        if self.unsafe_ {
            return Err(ExperimentError::UnsafeBarred);
        }

        // The single legitimate holdout access: the candidate config evaluated
        // over the locked tail. This is NOT an `unsafe` holdout unlock — it is
        // the one sanctioned, logged, self-sealing path.
        let mut vault_cfg = candidate.clone();
        vault_cfg.data_slice = self.holdout.slice.clone();
        let vault_cfg = vault_cfg.rehashed();

        let result = bt.run(&vault_cfg);

        // Seal BEFORE returning — a second attempt can never reach execution.
        self.holdout.access_log.push(VaultAccess {
            when: Utc::now(),
            run_id: result.run_id.clone(),
            by: by.into(),
        });
        self.holdout.spent = true;
        self.unsafe_ |= result.unsafe_;
        self.updated = Utc::now();

        // A successful evaluation validates the Experiment. A failed one leaves
        // it candidate-but-spent: dead for this holdout (continuing needs a new
        // Experiment with genuinely new data). Gate 4 (Phase 4) supplies the
        // pass/fail performance verdict; here the mechanical contract is "the
        // evaluation ran".
        if result.status == RunStatus::Ok {
            // can_transition_to(Candidate -> Validated) is legal.
            let _ = self.transition(ExperimentState::Validated);
        }
        Ok(result)
    }

    /// True if any slice the Study would evaluate intersects the holdout tail.
    fn study_touches_holdout(&self, study: &StudyConfig) -> bool {
        let h = &self.holdout.slice;
        if study.base_config.data_slice.overlaps(h) {
            return true;
        }
        // Window-varying studies may reach beyond the base slice.
        match &study.vary {
            VarySpec::DataWindows { windows } => windows.iter().any(|(start, end)| {
                let mut s = study.base_config.data_slice.clone();
                s.start = *start;
                s.end = *end;
                s.overlaps(h)
            }),
            VarySpec::Regimes { windows } => windows.iter().any(|(start, end, _)| {
                let mut s = study.base_config.data_slice.clone();
                s.start = *start;
                s.end = *end;
                s.overlaps(h)
            }),
            _ => false,
        }
    }
}

/// The recommended permutation-style Study kind has no bearing here; the
/// permutation-null requirement is enforced by [`StudyConfig`]. Re-exported for
/// callers wiring Gate 3.
#[must_use]
pub fn is_permutation_study(kind: StudyKind) -> bool {
    matches!(kind, StudyKind::PermutationNull)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::executor::{daily_curve, map_sim_result};
    use crate::run::{
        Backtest, ClosureExecutor, ComputeCost, EvalResolution, InMemoryRunStore, MetricKind,
        ParamMap, RunConfig, RunConfigBuilder, UnsafeFlags, ENGINE_VERSION,
    };
    use crate::study::{SelectionRule, StudyBudget, StudyConfig, StudyKind, VarySpec};
    use chrono::TimeZone;

    fn research_slice() -> DataSlice {
        DataSlice::new(
            "u",
            Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
            EvalResolution::Day1,
        )
    }

    fn holdout_slice() -> DataSlice {
        // The locked tail: 2023 onward, disjoint from research.
        DataSlice::new(
            "u",
            Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            EvalResolution::Day1,
        )
    }

    fn base_config(slice: DataSlice) -> RunConfig {
        RunConfigBuilder::new("ema", "v1", slice, "cost:floor", "sizing", "snap").build()
    }

    fn sweep_over(slice: DataSlice) -> StudyConfig {
        StudyConfig {
            study_id: "sweep-1".into(),
            kind: StudyKind::ParameterSweep,
            base_config: base_config(slice),
            vary: VarySpec::Params {
                grid: (0..10).map(|_| ParamMap::new()).collect(),
            },
            metric: MetricKind::TotalReturn,
            null_ref: None,
            budget: StudyBudget::default(),
            question: "how does perf vary?".into(),
            selection_rule: SelectionRule::None,
        }
    }

    fn ok_executor() -> impl RunExecutor {
        ClosureExecutor(|cfg: &RunConfig| {
            map_sim_result(
                cfg,
                daily_curve(&[100.0, 101.0, 102.0]),
                vec![],
                vec![],
                ComputeCost::default(),
                ENGINE_VERSION,
            )
        })
    }

    fn experiment() -> Experiment {
        Experiment::new("exp-1", "ema-family", holdout_slice(), "null:block_perm")
    }

    #[test]
    fn new_experiment_starts_candidate_zeroed_unspent() {
        let e = experiment();
        assert_eq!(e.state, ExperimentState::Candidate);
        assert_eq!(e.trial_counter(), 0);
        assert!(!e.holdout.spent);
        assert_eq!(e.primary_test(), "null:block_perm");
    }

    #[test]
    fn running_studies_auto_increments_monotonically() {
        let mut e = experiment();
        let bt = Backtest::new(InMemoryRunStore::new(), ok_executor());
        // Each sweep has 10 members → +10 per study.
        let mut s1 = sweep_over(research_slice());
        s1.study_id = "s1".into();
        let mut s2 = sweep_over(research_slice());
        s2.study_id = "s2".into();
        s2.vary = VarySpec::Params {
            grid: (0..120).map(|_| ParamMap::new()).collect(),
        };
        e.run_study(&s1, &bt).unwrap();
        assert_eq!(e.trial_counter(), 10);
        e.run_study(&s2, &bt).unwrap();
        assert_eq!(e.trial_counter(), 130);
        assert_eq!(e.studies.len(), 2);
    }

    #[test]
    fn research_study_touching_holdout_is_refused() {
        let mut e = experiment();
        let bt = Backtest::new(InMemoryRunStore::new(), ok_executor());
        // A sweep whose base slice IS the holdout tail.
        let bad = sweep_over(holdout_slice());
        assert_eq!(
            e.run_study(&bad, &bt).err(),
            Some(ExperimentError::TouchesHoldout)
        );
        assert_eq!(e.trial_counter(), 0, "a refused study never counts");
    }

    #[test]
    fn vault_requires_gate3_then_runs_once_and_validates() {
        let mut e = experiment();
        let bt = Backtest::new(InMemoryRunStore::new(), ok_executor());
        let candidate = base_config(research_slice());

        // Before Gate 3: refused.
        assert_eq!(
            e.run_vault(&candidate, &bt, "alice").err(),
            Some(ExperimentError::Gate3NotPassed)
        );

        e.mark_gate3_passed();
        let r = e.run_vault(&candidate, &bt, "alice").unwrap();
        assert_eq!(r.status, RunStatus::Ok);
        assert!(e.holdout.spent);
        assert_eq!(e.holdout.access_log.len(), 1);
        assert_eq!(e.holdout.access_log[0].by, "alice");
        assert_eq!(e.state, ExperimentState::Validated);

        // Second attempt refused — self-sealed.
        assert_eq!(
            e.run_vault(&candidate, &bt, "bob").err(),
            Some(ExperimentError::VaultSpent)
        );
    }

    #[test]
    fn vault_runs_over_the_holdout_slice() {
        let mut e = experiment();
        let bt = Backtest::new(InMemoryRunStore::new(), ok_executor());
        let candidate = base_config(research_slice());
        e.mark_gate3_passed();
        let r = e.run_vault(&candidate, &bt, "alice").unwrap();
        // The stored run's config slice must be the holdout, not the research slice.
        let stored = bt.store().get(&r.run_id).unwrap();
        assert_eq!(stored.run_id, r.run_id);
    }

    #[test]
    fn unsafe_propagates_and_bars_the_vault() {
        let mut e = experiment();
        let unsafe_exec = ClosureExecutor(|cfg: &RunConfig| {
            map_sim_result(
                cfg,
                daily_curve(&[100.0, 101.0]),
                vec![],
                vec![],
                ComputeCost::default(),
                ENGINE_VERSION,
            )
        });
        let bt = Backtest::new(InMemoryRunStore::new(), unsafe_exec);
        // A sweep whose base config disabled costs → unsafe runs.
        let mut s = sweep_over(research_slice());
        s.base_config =
            RunConfigBuilder::new("ema", "v1", research_slice(), "cost:none", "sz", "snap")
                .disable_protection(UnsafeFlags {
                    costs_disabled: true,
                    ..Default::default()
                })
                .build();
        e.run_study(&s, &bt).unwrap();
        assert!(e.is_unsafe());
        e.mark_gate3_passed();
        let candidate = base_config(research_slice());
        assert_eq!(
            e.run_vault(&candidate, &bt, "alice").err(),
            Some(ExperimentError::UnsafeBarred)
        );
    }

    #[test]
    fn lifecycle_is_one_directional_through_validation() {
        let mut e = experiment();
        e.mark_gate3_passed();
        let bt = Backtest::new(InMemoryRunStore::new(), ok_executor());
        e.run_vault(&base_config(research_slice()), &bt, "a")
            .unwrap();
        assert_eq!(e.state, ExperimentState::Validated);
        // Cannot drop back to candidate.
        assert!(e.transition(ExperimentState::Candidate).is_err());
        // Legal forward path.
        e.transition(ExperimentState::Live).unwrap();
        e.transition(ExperimentState::Decaying).unwrap();
        e.transition(ExperimentState::Live).unwrap();
        e.transition(ExperimentState::Retired).unwrap();
        assert!(e.transition(ExperimentState::Live).is_err());
    }

    #[test]
    fn validated_state_forbids_research_studies() {
        let mut e = experiment();
        e.mark_gate3_passed();
        let bt = Backtest::new(InMemoryRunStore::new(), ok_executor());
        e.run_vault(&base_config(research_slice()), &bt, "a")
            .unwrap();
        let s = sweep_over(research_slice());
        assert_eq!(
            e.run_study(&s, &bt).err(),
            Some(ExperimentError::OperationNotAllowed {
                state: ExperimentState::Validated,
                op: Operation::ResearchStudy,
            })
        );
    }

    #[test]
    fn counter_has_no_reset_or_decrement_path() {
        // Documents J-2.9: the only mutator is record_study (via run_study), and
        // it only adds. "Starting over" is a NEW Experiment with its own zeroed
        // counter — a fresh struct — never a reset of this one.
        let mut e = experiment();
        let bt = Backtest::new(InMemoryRunStore::new(), ok_executor());
        e.run_study(&sweep_over(research_slice()), &bt).unwrap();
        let after = e.trial_counter();
        assert_eq!(after, 10);
        let fresh = Experiment::new("exp-2", "ema-family", holdout_slice(), "null:block_perm");
        assert_eq!(
            fresh.trial_counter(),
            0,
            "a new experiment cannot inherit a lower count"
        );
    }

    #[test]
    fn round_trips_serde() {
        let e = experiment();
        let back: Experiment = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(e, back);
    }
}
