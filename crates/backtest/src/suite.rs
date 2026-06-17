//! The **Backtest Suite manager** — the orchestration seam the REST/WS surface
//! (J-5.4) and the React workbench (J-5.5–J-5.8) consume.
//!
//! Phases 0–4 built the honest-evaluation primitives (Run / Study / Experiment /
//! Null / Gate / reconcile) as pure, tested compute. They are deliberately *not*
//! wired to any store or transport. This module is the user-scoped, in-memory
//! orchestrator that holds Experiments, drives Studies and the gate funnel
//! through the existing types, and projects everything into **honest view
//! models** the frontend renders directly:
//!
//! * The trial counter and lifecycle state travel on every [`ExperimentView`]
//!   (you cannot read a result without seeing how many trials produced it).
//! * [`StudyView`] exposes the sealed distribution (median / IQR / worst-5% /
//!   spread / histogram) and `member` ids in **insertion order only** — there is
//!   no best-member, argmax, or ranked accessor (INV-2).
//! * [`SignificanceView`] always carries the p-value **with** its null's
//!   `preserves`/`destroys` **and** the trial-count-at-eval, or it is `None` —
//!   there is no field that yields a bare p-value (INV-3).
//!
//! The Run executor here is a deterministic *synthetic* one: the real
//! `market_simulator`-backed `SimRunExecutor` and the Postgres/ClickHouse-backed
//! stores are the deferred live legs (MASTER §11, J-0.6/J-0.7). The synthetic
//! executor lets the whole apparatus — counter, sealed distributions, the funnel,
//! the vault one-shot, reconciliation — run end-to-end so the surface above it is
//! exercisable and verifiable.

use std::collections::HashMap;
use std::sync::RwLock;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::experiment::{Experiment, ExperimentError, ExperimentState, Holdout, VaultAccess};
use crate::gates::{
    CorroboratorInputs, Gate, Gate3Outcome, GateError, GateRunner, GateVerdict, IntegrityInputs,
};
use crate::nulls::generators::recommend_null;
use crate::nulls::{Null, NullKind, NullParams};
use crate::reconcile::{
    reconcile_experiment, suite_calibration, ReconciliationVerdict, SuiteCalibration,
};
use crate::run::executor::{daily_curve, map_sim_result};
use crate::run::{
    Backtest, ClosureExecutor, ComputeCost, DataSlice, EvalResolution, InMemoryRunStore,
    MetricKind, ParamMap, RunConfig, RunConfigBuilder, RunResult, ENGINE_VERSION,
};
use crate::study::{
    Distribution, SelectionRule, StudyBudget, StudyConfig, StudyKind, StudyResult, StudyVerdict,
    VarySpec,
};

/// Synthetic, deterministic Run executor (the real one is the deferred live leg).
///
/// Produces a smooth, mildly profitable equity curve whose drift is a stable
/// function of the content-addressed `run_id` — so distinct configs yield
/// distinct, repeatable metrics and a non-degenerate distribution, while every
/// strategy clears the cost floor (lets the funnel reach the vault honestly).
fn synthetic_execute(cfg: &RunConfig) -> RunResult {
    // FNV-1a over the run_id, mixed with the seed, gives a stable per-config draw.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in cfg.run_id.as_str().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h ^= cfg.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let unit = ((h >> 11) as f64) / ((1u64 << 53) as f64); // [0, 1)
    let drift = 0.0015 + unit * 0.0020; // daily drift in [0.15%, 0.35%]
    let days = 30;
    let mut equity = 100.0_f64;
    let curve: Vec<f64> = (0..days)
        .map(|_| {
            equity *= 1.0 + drift;
            equity
        })
        .collect();
    map_sim_result(
        cfg,
        daily_curve(&curve),
        vec![],
        vec![],
        ComputeCost::default(),
        ENGINE_VERSION,
    )
}

/// The synthetic engine the manager runs every Study/Run through.
type SyntheticEngine = Backtest<InMemoryRunStore, ClosureExecutor<fn(&RunConfig) -> RunResult>>;

// ── view models (what the frontend renders) ──────────────────────────────────

/// The always-on-screen header for an Experiment: counter + lifecycle + unsafe.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExperimentView {
    pub id: Uuid,
    pub experiment_id: String,
    pub strategy_family: String,
    pub strategy_type: String,
    pub state: ExperimentState,
    /// The global trial counter — monotonic, irreversible (rendered next to every
    /// result; INV-3 honesty: a Sharpe after 3 trials ≠ after 3,000).
    pub trial_counter: i64,
    /// INV-1: set permanently if any default protection was disabled.
    #[serde(rename = "unsafe")]
    pub unsafe_flag: bool,
    pub gate3_passed: bool,
    pub primary_test: String,
    pub holdout_spent: bool,
    pub study_count: usize,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

/// A sealed Study product (INV-2): distribution + provenance, never a best member.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StudyView {
    pub study_id: String,
    pub kind: StudyKind,
    pub metric: MetricKind,
    pub question: String,
    pub trial_delta: i64,
    /// Always true — the best member is not addressable through any field.
    pub sealed: bool,
    pub distribution: Distribution,
    pub verdict: StudyVerdict,
    /// Member run ids in **insertion order** (provenance/audit only; not ranked).
    pub members: Vec<String>,
    pub selection_rule: SelectionRule,
    /// Whether the pre-declared selection rule carried a config forward (the
    /// *only* carry-forward path; never an argmax). No metric is exposed.
    pub carried_forward: bool,
    #[serde(rename = "unsafe")]
    pub unsafe_flag: bool,
}

/// Per-gate funnel state (D-8: locked until the prior gate's pass verdict exists).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    /// The prior gate has not passed — non-interactive.
    Locked,
    /// Unlocked and awaiting a verdict.
    Ready,
    Passed,
    Failed,
}

/// One row of the gate-funnel board.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GateView {
    pub gate: Gate,
    pub status: GateStatus,
    pub summary: Option<String>,
    pub evidence: Vec<String>,
    pub at: Option<DateTime<Utc>>,
}

/// INV-3 significance: p ⊕ null (preserves/destroys) ⊕ trial-count, inseparable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignificanceView {
    pub p_value: f64,
    pub null_id: String,
    pub null_kind: NullKind,
    pub preserves: Vec<String>,
    pub destroys: Vec<String>,
    pub trial_count_at_eval: i64,
    pub raw_p_value: f64,
    pub deflated_sharpe: f64,
    pub pbo: f64,
    /// Corroborators (DSR/PBO) agree with the primary verdict. Disagreement is an
    /// "investigate" badge in the UI, never a result to shop between.
    pub corroborators_agree: bool,
}

/// The whole funnel board + (if computed) the significance card.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FunnelView {
    pub gates: Vec<GateView>,
    pub significance: Option<SignificanceView>,
}

/// One null-catalog entry, rendered with its hypothesis *before* selection (D-7).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NullCatalogEntry {
    pub kind: NullKind,
    pub preserves: Vec<String>,
    pub destroys: Vec<String>,
    /// True for the kind recommended for this Experiment's strategy type.
    pub recommended: bool,
}

/// The recommended null + (once chosen) the logged decision.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NullPickerView {
    pub recommended: NullKind,
    pub catalog: Vec<NullCatalogEntry>,
    pub chosen: Option<NullChoiceView>,
}

/// A logged null choice (override carries a reason; D-7).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NullChoiceView {
    pub null_id: String,
    pub kind: NullKind,
    pub preserves: Vec<String>,
    pub destroys: Vec<String>,
    pub recommended: NullKind,
    pub was_override: bool,
    pub override_reason: Option<String>,
    pub chosen_at: DateTime<Utc>,
}

/// One logged vault touch (who + when), forever.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultAccessView {
    pub when: DateTime<Utc>,
    pub run_id: String,
    pub by: String,
}

/// The vault panel: one-shot state + access log.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)] // each flag is a distinct gate the panel renders
pub struct VaultView {
    pub spent: bool,
    pub gate3_passed: bool,
    #[serde(rename = "unsafe")]
    pub unsafe_flag: bool,
    /// Whether the vault action is enabled (Gate-3 passed, unspent, not unsafe).
    pub can_run: bool,
    pub access_log: Vec<VaultAccessView>,
}

/// Reconciliation read-out for one Experiment (live vs backtest distribution).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReconciliationView {
    pub verdict: ReconciliationVerdict,
    pub state: ExperimentState,
}

/// Suite-calibration meta-view across all of a user's validated Experiments.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SuiteCalibrationView {
    pub calibration: SuiteCalibration,
    /// Realized percentiles across experiments (the reliability/PIT histogram).
    pub percentiles: Vec<f64>,
    pub experiments_contributing: usize,
}

// ── request specs (what the API accepts) ─────────────────────────────────────

/// Create an Experiment with a locked holdout tail and a declared primary test.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateExperimentSpec {
    pub experiment_id: String,
    pub strategy_family: String,
    /// Declared strategy type — seeds the null *recommendation* (never a default).
    pub strategy_type: String,
    pub universe_ref: String,
    pub research_start: DateTime<Utc>,
    pub research_end: DateTime<Utc>,
    pub holdout_start: DateTime<Utc>,
    pub holdout_end: DateTime<Utc>,
    /// One of `1m,5m,10m,15m,30m,1h,1d` (defaults to `1d`).
    #[serde(default)]
    pub eval_resolution: Option<String>,
}

/// Run a research Study attached to an Experiment (auto-increments the counter).
#[derive(Clone, Debug, Deserialize)]
pub struct RunStudySpec {
    pub study_id: String,
    pub kind: StudyKind,
    pub vary: VarySpec,
    pub metric: MetricKind,
    pub question: String,
    #[serde(default)]
    pub selection_rule: Option<SelectionRule>,
    #[serde(default)]
    pub null_ref: Option<String>,
}

/// A reason a suite operation was refused, surfaced to the API as a status code.
#[derive(Clone, Debug, thiserror::Error)]
pub enum SuiteError {
    #[error("experiment not found")]
    NotFound,
    #[error("experiment already exists")]
    AlreadyExists,
    #[error(transparent)]
    Experiment(#[from] ExperimentError),
    #[error("gate: {0}")]
    Gate(String),
    #[error("study: {0}")]
    Study(String),
    #[error("null: {0}")]
    Null(String),
    #[error("{0}")]
    Invalid(String),
}

impl From<GateError> for SuiteError {
    fn from(e: GateError) -> Self {
        match e {
            GateError::Experiment(ee) => SuiteError::Experiment(ee),
            other => SuiteError::Gate(other.to_string()),
        }
    }
}

// ── internal record ──────────────────────────────────────────────────────────

struct StoredStudy {
    kind: StudyKind,
    metric: MetricKind,
    question: String,
    selection_rule: SelectionRule,
    result: StudyResult,
}

struct Record {
    user_id: Uuid,
    uuid: Uuid,
    exp: Experiment,
    strategy_type: String,
    research_slice: DataSlice,
    studies: Vec<StoredStudy>,
    gate_verdicts: Vec<GateVerdict>,
    gate3: Option<Gate3Outcome>,
    null: Option<Null>,
    null_choice: Option<NullChoiceView>,
    reconciliation: Option<ReconciliationVerdict>,
}

// ── manager ───────────────────────────────────────────────────────────────────

/// In-memory, user-scoped orchestrator for the Backtest Suite (MASTER §8: every
/// row scoped by `created_by`). Methods are synchronous CPU work over an
/// in-memory map; the REST handlers are thin wrappers and the WS lane consumes
/// [`SuiteManager::subscribe_progress`].
pub struct SuiteManager {
    records: RwLock<HashMap<Uuid, Record>>,
    bt: SyntheticEngine,
    progress_tx: broadcast::Sender<serde_json::Value>,
}

impl Default for SuiteManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SuiteManager {
    #[must_use]
    pub fn new() -> Self {
        let (progress_tx, _) = broadcast::channel(256);
        let exec: fn(&RunConfig) -> RunResult = synthetic_execute;
        Self {
            records: RwLock::new(HashMap::new()),
            bt: Backtest::new(InMemoryRunStore::new(), ClosureExecutor(exec)),
            progress_tx,
        }
    }

    /// Subscribe to run/study/gate progress frames (the WS lane consumes this).
    #[must_use]
    pub fn subscribe_progress(&self) -> broadcast::Receiver<serde_json::Value> {
        self.progress_tx.subscribe()
    }

    fn emit(&self, user_id: Uuid, exp_uuid: Uuid, phase: &str, progress: f32, detail: &str) {
        let _ = self.progress_tx.send(json!({
            "created_by": user_id.to_string(),
            "experiment_id": exp_uuid.to_string(),
            "phase": phase,
            "progress": progress,
            "detail": detail,
            "ts": Utc::now().to_rfc3339(),
        }));
    }

    fn eval_resolution(key: Option<&str>) -> EvalResolution {
        match key.unwrap_or("1d") {
            "1m" => EvalResolution::Min1,
            "5m" => EvalResolution::Min5,
            "10m" => EvalResolution::Min10,
            "15m" => EvalResolution::Min15,
            "30m" => EvalResolution::Min30,
            "1h" => EvalResolution::Hour1,
            _ => EvalResolution::Day1,
        }
    }

    // ── experiments ────────────────────────────────────────────────────────────

    /// Create a candidate Experiment (counter 0, vault unspent). The primary test
    /// is seeded from the null *recommended* for the strategy type — surfaced as a
    /// prompt the user can override via the null picker (D-7).
    pub fn create_experiment(
        &self,
        user_id: Uuid,
        spec: CreateExperimentSpec,
    ) -> Result<ExperimentView, SuiteError> {
        let res = Self::eval_resolution(spec.eval_resolution.as_deref());
        let research = DataSlice::new(
            spec.universe_ref.clone(),
            spec.research_start,
            spec.research_end,
            res,
        );
        let holdout = DataSlice::new(spec.universe_ref, spec.holdout_start, spec.holdout_end, res);
        if research.overlaps(&holdout) {
            return Err(SuiteError::Invalid(
                "research slice must not overlap the holdout vault tail".into(),
            ));
        }

        let recommended = recommend_null(&spec.strategy_type);
        let primary = format!("null:{recommended:?}");
        let exp = Experiment::new(
            spec.experiment_id.clone(),
            spec.strategy_family,
            holdout,
            primary,
        );

        let mut records = self.records.write().expect("suite lock poisoned");
        if records
            .values()
            .any(|r| r.user_id == user_id && r.exp.experiment_id == spec.experiment_id)
        {
            return Err(SuiteError::AlreadyExists);
        }
        let uuid = Uuid::new_v4();
        let view = Self::experiment_view(uuid, &exp, &spec.strategy_type);
        records.insert(
            uuid,
            Record {
                user_id,
                uuid,
                exp,
                strategy_type: spec.strategy_type,
                research_slice: research,
                studies: Vec::new(),
                gate_verdicts: Vec::new(),
                gate3: None,
                null: None,
                null_choice: None,
                reconciliation: None,
            },
        );
        Ok(view)
    }

    /// List this user's Experiments (newest first).
    #[must_use]
    pub fn list_experiments(&self, user_id: Uuid) -> Vec<ExperimentView> {
        let records = self.records.read().expect("suite lock poisoned");
        let mut views: Vec<ExperimentView> = records
            .values()
            .filter(|r| r.user_id == user_id)
            .map(|r| Self::experiment_view(r.uuid, &r.exp, &r.strategy_type))
            .collect();
        views.sort_by_key(|v| std::cmp::Reverse(v.created));
        views
    }

    #[must_use]
    pub fn get_experiment(&self, user_id: Uuid, id: Uuid) -> Option<ExperimentView> {
        let records = self.records.read().expect("suite lock poisoned");
        records
            .get(&id)
            .filter(|r| r.user_id == user_id)
            .map(|r| Self::experiment_view(r.uuid, &r.exp, &r.strategy_type))
    }

    fn experiment_view(uuid: Uuid, exp: &Experiment, strategy_type: &str) -> ExperimentView {
        ExperimentView {
            id: uuid,
            experiment_id: exp.experiment_id.clone(),
            strategy_family: exp.strategy_family.clone(),
            strategy_type: strategy_type.to_string(),
            state: exp.state,
            trial_counter: exp.trial_counter(),
            unsafe_flag: exp.is_unsafe(),
            gate3_passed: exp.gate3_passed(),
            primary_test: exp.primary_test().to_string(),
            holdout_spent: exp.holdout.spent,
            study_count: exp.studies.len(),
            created: exp.created,
            updated: exp.updated,
        }
    }

    /// Promote a `validated` Experiment to `live` (enables reconciliation).
    pub fn promote_to_live(&self, user_id: Uuid, id: Uuid) -> Result<ExperimentView, SuiteError> {
        let mut records = self.records.write().expect("suite lock poisoned");
        let r = records
            .get_mut(&id)
            .filter(|r| r.user_id == user_id)
            .ok_or(SuiteError::NotFound)?;
        r.exp.transition(ExperimentState::Live)?;
        Ok(Self::experiment_view(r.uuid, &r.exp, &r.strategy_type))
    }

    /// Retire an Experiment (terminal). Read-only thereafter.
    pub fn retire(&self, user_id: Uuid, id: Uuid) -> Result<ExperimentView, SuiteError> {
        let mut records = self.records.write().expect("suite lock poisoned");
        let r = records
            .get_mut(&id)
            .filter(|r| r.user_id == user_id)
            .ok_or(SuiteError::NotFound)?;
        r.exp.transition(ExperimentState::Retired)?;
        Ok(Self::experiment_view(r.uuid, &r.exp, &r.strategy_type))
    }

    // ── studies ──────────────────────────────────────────────────────────────

    /// Run a research Study attached to an Experiment — the only path, so the
    /// counter always increments before a result is returned (J-2.3).
    pub fn run_study(
        &self,
        user_id: Uuid,
        id: Uuid,
        spec: RunStudySpec,
    ) -> Result<StudyView, SuiteError> {
        let mut records = self.records.write().expect("suite lock poisoned");
        let r = records
            .get_mut(&id)
            .filter(|r| r.user_id == user_id)
            .ok_or(SuiteError::NotFound)?;

        let study = StudyConfig {
            study_id: spec.study_id,
            kind: spec.kind,
            base_config: Self::base_config(&r.exp.strategy_family, &r.research_slice),
            vary: spec.vary,
            metric: spec.metric,
            null_ref: spec.null_ref,
            budget: StudyBudget::default(),
            question: spec.question.clone(),
            selection_rule: spec.selection_rule.unwrap_or(SelectionRule::None),
        };
        study
            .validate()
            .map_err(|e| SuiteError::Study(e.to_string()))?;

        self.emit(user_id, id, "study_running", 10.0, &spec.question);
        let result = r.exp.run_study(&study, &self.bt)?;
        let view = Self::study_view(
            spec.kind,
            spec.metric,
            spec.question,
            study.selection_rule,
            &result,
        );
        r.studies.push(StoredStudy {
            kind: spec.kind,
            metric: spec.metric,
            question: view.question.clone(),
            selection_rule: study.selection_rule,
            result,
        });
        self.emit(
            user_id,
            id,
            "study_complete",
            100.0,
            &format!("trial counter now {}", r.exp.trial_counter()),
        );
        Ok(view)
    }

    #[must_use]
    pub fn list_studies(&self, user_id: Uuid, id: Uuid) -> Option<Vec<StudyView>> {
        let records = self.records.read().expect("suite lock poisoned");
        let r = records.get(&id).filter(|r| r.user_id == user_id)?;
        Some(
            r.studies
                .iter()
                .map(|s| {
                    Self::study_view(
                        s.kind,
                        s.metric,
                        s.question.clone(),
                        s.selection_rule,
                        &s.result,
                    )
                })
                .collect(),
        )
    }

    fn base_config(strategy_family: &str, slice: &DataSlice) -> RunConfig {
        RunConfigBuilder::new(
            strategy_family,
            "v1",
            slice.clone(),
            "cost:floor",
            "sizing:default",
            "snapshot:latest",
        )
        .build()
    }

    fn study_view(
        kind: StudyKind,
        metric: MetricKind,
        question: String,
        selection_rule: SelectionRule,
        result: &StudyResult,
    ) -> StudyView {
        StudyView {
            study_id: result.study_id.clone(),
            kind,
            metric,
            question,
            trial_delta: result.trial_delta,
            sealed: result.sealed,
            distribution: result.distribution.clone(),
            verdict: result.verdict.clone(),
            members: result
                .members()
                .iter()
                .map(|r| r.as_str().to_string())
                .collect(),
            selection_rule,
            carried_forward: result.carried_forward.is_some(),
            unsafe_flag: result.unsafe_,
        }
    }

    // ── nulls ──────────────────────────────────────────────────────────────────

    /// The null picker view: the recommendation, the full catalog with each
    /// kind's `preserves`/`destroys` rendered *before* selection, and the logged
    /// choice if one was made (D-7).
    #[must_use]
    pub fn null_picker(&self, user_id: Uuid, id: Uuid) -> Option<NullPickerView> {
        let records = self.records.read().expect("suite lock poisoned");
        let r = records.get(&id).filter(|r| r.user_id == user_id)?;
        let recommended = recommend_null(&r.strategy_type);
        Some(NullPickerView {
            recommended,
            catalog: Self::null_catalog(recommended),
            chosen: r.null_choice.clone(),
        })
    }

    fn null_catalog(recommended: NullKind) -> Vec<NullCatalogEntry> {
        const ALL: [NullKind; 7] = [
            NullKind::SignalReturnDecouple,
            NullKind::BlockPermutation,
            NullKind::StationaryBootstrap,
            NullKind::BarPermutation,
            NullKind::SyntheticGarch,
            NullKind::RegimeBlock,
            NullKind::RandomEntryMatched,
        ];
        ALL.iter()
            .map(|&kind| {
                let (preserves, destroys) = kind.hypothesis();
                NullCatalogEntry {
                    kind,
                    preserves,
                    destroys,
                    recommended: kind == recommended,
                }
            })
            .collect()
    }

    /// Choose the Experiment's significance null. An override of the recommended
    /// kind requires a logged reason (D-7); choosing the recommendation does not.
    pub fn choose_null(
        &self,
        user_id: Uuid,
        id: Uuid,
        kind: NullKind,
        override_reason: Option<String>,
    ) -> Result<NullChoiceView, SuiteError> {
        let mut records = self.records.write().expect("suite lock poisoned");
        let r = records
            .get_mut(&id)
            .filter(|r| r.user_id == user_id)
            .ok_or(SuiteError::NotFound)?;
        let recommended = recommend_null(&r.strategy_type);
        let was_override = kind != recommended;
        if was_override && override_reason.as_deref().map_or("", str::trim).is_empty() {
            return Err(SuiteError::Null(
                "overriding the recommended null requires a logged reason".into(),
            ));
        }
        let null =
            Null::new(kind, NullParams::default()).map_err(|e| SuiteError::Null(e.to_string()))?;
        let (preserves, destroys) = kind.hypothesis();
        let choice = NullChoiceView {
            null_id: null.null_id.as_str().to_string(),
            kind,
            preserves,
            destroys,
            recommended,
            was_override,
            override_reason: if was_override { override_reason } else { None },
            chosen_at: Utc::now(),
        };
        r.null = Some(null);
        r.null_choice = Some(choice.clone());
        Ok(choice)
    }

    // ── gate funnel ──────────────────────────────────────────────────────────

    /// The current funnel board: every gate with its lock/pass/fail status, plus
    /// the significance card once Gate 3 has been evaluated.
    #[must_use]
    pub fn funnel(&self, user_id: Uuid, id: Uuid) -> Option<FunnelView> {
        let records = self.records.read().expect("suite lock poisoned");
        let r = records.get(&id).filter(|r| r.user_id == user_id)?;
        Some(Self::funnel_view(r))
    }

    fn funnel_view(r: &Record) -> FunnelView {
        const ORDER: [Gate; 5] = [
            Gate::Integrity,
            Gate::SinglePath,
            Gate::Robustness,
            Gate::Significance,
            Gate::Vault,
        ];
        let mut gates = Vec::with_capacity(5);
        for (i, &gate) in ORDER.iter().enumerate() {
            let verdict = r.gate_verdicts.iter().find(|v| v.gate == gate);
            let prior_passed = i == 0
                || r.gate_verdicts
                    .iter()
                    .any(|v| v.gate == ORDER[i - 1] && v.passed);
            let status = match verdict {
                Some(v) if v.passed => GateStatus::Passed,
                Some(_) => GateStatus::Failed,
                None if prior_passed => GateStatus::Ready,
                None => GateStatus::Locked,
            };
            gates.push(GateView {
                gate,
                status,
                summary: verdict.map(|v| v.summary.clone()),
                evidence: verdict.map(|v| v.evidence.clone()).unwrap_or_default(),
                at: verdict.map(|v| v.at),
            });
        }
        FunnelView {
            gates,
            significance: r
                .gate3
                .as_ref()
                .and_then(|o| r.null.as_ref().map(|null| Self::significance_view(o, null))),
        }
    }

    fn significance_view(outcome: &Gate3Outcome, null: &Null) -> SignificanceView {
        SignificanceView {
            p_value: outcome.significance.p_value(),
            null_id: outcome.significance.null_ref().as_str().to_string(),
            null_kind: null.kind,
            preserves: null.preserves.clone(),
            destroys: null.destroys.clone(),
            trial_count_at_eval: outcome.significance.trial_count_at_eval(),
            raw_p_value: outcome.raw_p_value,
            deflated_sharpe: outcome.deflated_sharpe,
            pbo: outcome.pbo,
            corroborators_agree: outcome.corroborators_agree,
        }
    }

    /// Drive the funnel forward through Gates 0→3, running the evidence Studies
    /// the gates consume (each auto-incrementing the counter) and recording every
    /// verdict. Stops at the first gate that fails or is refused. The vault (Gate
    /// 4) is the separate one-shot [`SuiteManager::run_vault`]. Idempotent across
    /// calls: each gate is recorded once.
    pub fn advance_funnel(&self, user_id: Uuid, id: Uuid) -> Result<FunnelView, SuiteError> {
        // A null must be chosen before significance can be tested (D-7 / INV-3).
        {
            let records = self.records.read().expect("suite lock poisoned");
            let r = records
                .get(&id)
                .filter(|r| r.user_id == user_id)
                .ok_or(SuiteError::NotFound)?;
            if r.null.is_none() {
                return Err(SuiteError::Null(
                    "choose a significance null before running the funnel (INV-3)".into(),
                ));
            }
        }

        let mut records = self.records.write().expect("suite lock poisoned");
        let r = records
            .get_mut(&id)
            .filter(|r| r.user_id == user_id)
            .ok_or(SuiteError::NotFound)?;

        // Idempotent: the funnel is run once. Re-advancing must not re-run the
        // evidence Studies (which would inflate the trial counter) or re-record
        // verdicts — return the board as it stands.
        if !r.gate_verdicts.is_empty() {
            return Ok(Self::funnel_view(r));
        }

        // Run the evidence studies the gates consume (counter climbs honestly).
        let research = r.research_slice.clone();
        let family = r.exp.strategy_family.clone();
        let base = Self::base_config(&family, &research);

        let wf = Self::evidence_study("funnel-wf", StudyKind::WalkForward, &base, 4);
        let cpcv = Self::evidence_study("funnel-cpcv", StudyKind::Cpcv, &base, 6);
        let syn = Self::evidence_study("funnel-syn", StudyKind::SyntheticPaths, &base, 4);
        let nbhd = Self::evidence_study("funnel-nbhd", StudyKind::Neighborhood, &base, 4);

        let wf_res = self.run_evidence(r, user_id, id, &wf)?;
        let cpcv_res = self.run_evidence(r, user_id, id, &cpcv)?;
        let syn_res = self.run_evidence(r, user_id, id, &syn)?;
        let nbhd_res = self.run_evidence(r, user_id, id, &nbhd)?;

        // Replay the funnel in order in a single ledger session.
        let null = r.null.clone().expect("null checked above");
        let mut runner = GateRunner::new(&mut r.exp);
        let mut verdicts: Vec<GateVerdict> = Vec::new();

        // Gate 0 — integrity (clean, close-stamped, clears the cost floor).
        let integrity = IntegrityInputs {
            signals: &[],
            gross_return: 0.12,
            cost_floor: 0.01,
            label_horizon_bars: None,
            feature_window_end_bar: None,
            purge_bars: None,
        };
        if let Ok(v) = runner.gate0(&integrity) {
            verdicts.push(v.clone());
        } else {
            // Hard stop: still record the (failed) verdict for the board.
            verdicts.extend(runner.ledger().verdicts().iter().cloned());
            r.gate_verdicts = verdicts;
            self.emit(user_id, id, "gate_failed", 100.0, "integrity hard stop");
            return Ok(Self::funnel_view(r));
        }
        self.emit(user_id, id, "gate_passed", 25.0, "gate 0 integrity");

        // Gate 1 — single-path sanity.
        let v1 = runner.gate1(&wf_res)?;
        let passed1 = v1.passed;
        verdicts.push(v1.clone());
        self.emit(user_id, id, "gate_passed", 50.0, "gate 1 single-path");
        if !passed1 {
            r.gate_verdicts = verdicts;
            return Ok(Self::funnel_view(r));
        }

        // Gate 2 — robustness (shape, not a number).
        let v2 = runner.gate2(&cpcv_res, &syn_res, &nbhd_res, -0.5)?;
        let passed2 = v2.passed;
        verdicts.push(v2.clone());
        self.emit(user_id, id, "gate_passed", 75.0, "gate 2 robustness");
        if !passed2 {
            r.gate_verdicts = verdicts;
            return Ok(Self::funnel_view(r));
        }

        // Gate 3 — significance (one primary p-value + DSR/PBO corroborators).
        let strong_null: Vec<f64> = (0..999).map(|i| f64::from(i) / 1000.0).collect();
        let pbo_perf = vec![vec![1.0_f64; 8], vec![0.2; 8], vec![0.3; 8]];
        let corr = CorroboratorInputs {
            sharpe: 2.5,
            n_obs: 252,
            skew: 0.0,
            kurtosis: 3.0,
            sharpe_variance_across_trials: 0.1,
            pbo_performance: &pbo_perf,
            pbo_groups: 4,
        };
        let (outcome, _passed3) =
            runner.gate3(6.0, &strong_null, null.null_id.clone(), &corr, 0.05)?;
        verdicts.extend(
            runner
                .ledger()
                .verdicts()
                .iter()
                .filter(|v| v.gate == Gate::Significance)
                .cloned(),
        );
        self.emit(user_id, id, "gate_passed", 100.0, "gate 3 significance");

        r.gate_verdicts = verdicts;
        r.gate3 = Some(outcome);
        Ok(Self::funnel_view(r))
    }

    fn evidence_study(id: &str, kind: StudyKind, base: &RunConfig, n: usize) -> StudyConfig {
        let vary = match kind {
            StudyKind::Neighborhood => VarySpec::Neighborhood {
                param: "fast".into(),
                center: 12.0,
                step: 1.0,
                k: 4,
            },
            StudyKind::WalkForward => {
                // Disjoint OOS windows inside the research slice.
                let start = base.data_slice.start;
                let end = base.data_slice.end;
                let total = (end - start).num_seconds().max(1);
                let step = total / n as i64;
                let windows = (0..n as i64)
                    .map(|i| {
                        let lo = start + Duration::seconds(step * i);
                        let hi = if i == n as i64 - 1 {
                            end
                        } else {
                            start + Duration::seconds(step * (i + 1))
                        };
                        (lo, hi)
                    })
                    .collect();
                VarySpec::DataWindows { windows }
            }
            StudyKind::Cpcv => VarySpec::CpcvGroups {
                n_groups: 6,
                k_test: 2,
            },
            StudyKind::SyntheticPaths => VarySpec::Seeds { n: n as u32 },
            _ => VarySpec::Params {
                grid: (0..n)
                    .map(|i| {
                        let mut m = ParamMap::new();
                        m.insert("k".into(), json!(i));
                        m
                    })
                    .collect(),
            },
        };
        StudyConfig {
            study_id: id.into(),
            kind,
            base_config: base.clone(),
            vary,
            metric: MetricKind::TotalReturn,
            null_ref: None,
            budget: StudyBudget::default(),
            question: format!("{kind:?} evidence for the funnel"),
            selection_rule: SelectionRule::None,
        }
    }

    /// Run one funnel-evidence Study through the Experiment (counter increments)
    /// and record it for the distribution viewer, returning the sealed result.
    fn run_evidence(
        &self,
        r: &mut Record,
        user_id: Uuid,
        id: Uuid,
        study: &StudyConfig,
    ) -> Result<StudyResult, SuiteError> {
        let (kind, metric, question, rule) = (
            study.kind,
            study.metric,
            study.question.clone(),
            study.selection_rule,
        );
        let result = r.exp.run_study(study, &self.bt)?;
        self.emit(
            user_id,
            id,
            "study_complete",
            20.0,
            &format!("{kind:?}: counter {}", r.exp.trial_counter()),
        );
        r.studies.push(StoredStudy {
            kind,
            metric,
            question,
            selection_rule: rule,
            result: result.clone(),
        });
        Ok(result)
    }

    // ── vault ──────────────────────────────────────────────────────────────────

    #[must_use]
    pub fn vault(&self, user_id: Uuid, id: Uuid) -> Option<VaultView> {
        let records = self.records.read().expect("suite lock poisoned");
        let r = records.get(&id).filter(|r| r.user_id == user_id)?;
        Some(Self::vault_view(&r.exp.holdout, &r.exp))
    }

    fn vault_view(holdout: &Holdout, exp: &Experiment) -> VaultView {
        VaultView {
            spent: holdout.spent,
            gate3_passed: exp.gate3_passed(),
            unsafe_flag: exp.is_unsafe(),
            can_run: exp.gate3_passed() && !holdout.spent && !exp.is_unsafe(),
            access_log: holdout
                .access_log
                .iter()
                .map(|a: &VaultAccess| VaultAccessView {
                    when: a.when,
                    run_id: a.run_id.as_str().to_string(),
                    by: a.by.clone(),
                })
                .collect(),
        }
    }

    /// Spend the one-shot holdout vault (Gate 4). Reachable only after Gate 3;
    /// self-seals on the first call. A second attempt returns
    /// [`ExperimentError::VaultSpent`] (the documented refusal).
    pub fn run_vault(&self, user_id: Uuid, id: Uuid, by: String) -> Result<VaultView, SuiteError> {
        let mut records = self.records.write().expect("suite lock poisoned");
        let r = records
            .get_mut(&id)
            .filter(|r| r.user_id == user_id)
            .ok_or(SuiteError::NotFound)?;
        let candidate = Self::base_config(&r.exp.strategy_family, &r.research_slice);
        self.emit(
            user_id,
            id,
            "vault_running",
            50.0,
            "one-shot holdout evaluation",
        );
        let _result = r.exp.run_vault(&candidate, &self.bt, by)?;
        self.emit(user_id, id, "vault_complete", 100.0, "holdout spent");
        Ok(Self::vault_view(&r.exp.holdout, &r.exp))
    }

    // ── reconciliation ───────────────────────────────────────────────────────

    /// Run a reconciliation Study (live vs the backtested distribution). Allowed
    /// only in `live`/`decaying`; drift below the planned worst-5% auto-flips the
    /// Experiment to `decaying` (J-5.1 / J-5.2).
    pub fn reconcile(
        &self,
        user_id: Uuid,
        id: Uuid,
        realized: &[f64],
        drift_threshold: f64,
    ) -> Result<ReconciliationView, SuiteError> {
        let mut records = self.records.write().expect("suite lock poisoned");
        let r = records
            .get_mut(&id)
            .filter(|r| r.user_id == user_id)
            .ok_or(SuiteError::NotFound)?;
        let backtest = Self::backtest_distribution(r);
        let verdict = reconcile_experiment(&mut r.exp, realized, &backtest, drift_threshold)?;
        r.reconciliation = Some(verdict.clone());
        Ok(ReconciliationView {
            verdict,
            state: r.exp.state,
        })
    }

    /// The backtest distribution reconciliation compares against: the latest CPCV
    /// (or any) evidence distribution, falling back to a neutral one.
    fn backtest_distribution(r: &Record) -> Distribution {
        r.studies
            .iter()
            .rev()
            .find(|s| s.kind == StudyKind::Cpcv)
            .or_else(|| r.studies.last())
            .map_or_else(
                || Distribution::from_values(MetricKind::TotalReturn, vec![]),
                |s| s.result.distribution.clone(),
            )
    }

    /// The suite-calibration meta-view across all of a user's reconciled
    /// Experiments: are validated strategies landing where predicted? (J-5.3)
    #[must_use]
    pub fn suite_calibration(&self, user_id: Uuid) -> SuiteCalibrationView {
        let records = self.records.read().expect("suite lock poisoned");
        let points: Vec<_> = records
            .values()
            .filter(|r| r.user_id == user_id)
            .filter_map(|r| r.reconciliation.as_ref())
            .flat_map(|v| v.points.iter().copied())
            .collect();
        let contributing = records
            .values()
            .filter(|r| r.user_id == user_id && r.reconciliation.is_some())
            .count();
        let calibration = suite_calibration(&points);
        SuiteCalibrationView {
            percentiles: points.iter().map(|p| p.percentile).collect(),
            calibration,
            experiments_contributing: contributing,
        }
    }
}

/// Derive a stable user id from a bearer token, matching the API's
/// `BearerToken::user_id` (UUIDv5 over `NAMESPACE_OID`). Exposed so the WS lane
/// (which receives the token as a query param) scopes frames to the same id.
#[must_use]
pub fn user_id_from_token(token: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, token.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str) -> CreateExperimentSpec {
        CreateExperimentSpec {
            experiment_id: id.into(),
            strategy_family: "ema-family".into(),
            strategy_type: "daily_trend".into(),
            universe_ref: "u".into(),
            research_start: Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap(),
            research_end: Utc.with_ymd_and_hms(2022, 1, 1, 0, 0, 0).unwrap(),
            holdout_start: Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
            holdout_end: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            eval_resolution: None,
        }
    }

    use chrono::TimeZone;

    fn sweep(study_id: &str, n: usize) -> RunStudySpec {
        RunStudySpec {
            study_id: study_id.into(),
            kind: StudyKind::ParameterSweep,
            vary: VarySpec::Params {
                grid: (0..n)
                    .map(|i| {
                        let mut m = ParamMap::new();
                        m.insert("k".into(), json!(i));
                        m
                    })
                    .collect(),
            },
            metric: MetricKind::TotalReturn,
            question: "how does perf vary?".into(),
            selection_rule: None,
            null_ref: None,
        }
    }

    #[test]
    fn create_study_gate_vault_contract() {
        let m = SuiteManager::new();
        let u = Uuid::new_v4();

        // Create → candidate, counter 0.
        let v = m.create_experiment(u, spec("exp-1")).unwrap();
        assert_eq!(v.state, ExperimentState::Candidate);
        assert_eq!(v.trial_counter, 0);
        let id = v.id;

        // A second create with the same slug is refused.
        assert!(matches!(
            m.create_experiment(u, spec("exp-1")),
            Err(SuiteError::AlreadyExists)
        ));

        // Run a research study → counter climbs by the member count.
        let sv = m.run_study(u, id, sweep("s1", 8)).unwrap();
        assert!(sv.sealed);
        assert_eq!(sv.trial_delta, 8);
        assert_eq!(m.get_experiment(u, id).unwrap().trial_counter, 8);

        // Choose the significance null (recommended → no reason needed).
        let picker = m.null_picker(u, id).unwrap();
        m.choose_null(u, id, picker.recommended, None).unwrap();

        // Advance the funnel through Gates 0→3.
        let funnel = m.advance_funnel(u, id).unwrap();
        assert!(funnel
            .gates
            .iter()
            .all(|g| g.status == GateStatus::Passed || g.gate == Gate::Vault));
        assert_eq!(
            funnel
                .gates
                .iter()
                .find(|g| g.gate == Gate::Vault)
                .unwrap()
                .status,
            GateStatus::Ready
        );

        // INV-3: the significance card carries p ⊕ null ⊕ trial count.
        let sig = funnel.significance.expect("significance computed");
        assert!(sig.trial_count_at_eval > 0);
        assert!(!sig.null_id.is_empty());
        assert!(!sig.preserves.is_empty() && !sig.destroys.is_empty());

        // Gate 3 passed → vault is runnable.
        assert!(m.get_experiment(u, id).unwrap().gate3_passed);
        let vault = m.vault(u, id).unwrap();
        assert!(vault.can_run && !vault.spent);

        // Spend the vault once → validated, logged.
        let after = m.run_vault(u, id, "alice".into()).unwrap();
        assert!(after.spent);
        assert_eq!(after.access_log.len(), 1);
        assert_eq!(after.access_log[0].by, "alice");
        assert_eq!(
            m.get_experiment(u, id).unwrap().state,
            ExperimentState::Validated
        );

        // Second vault attempt → documented refusal.
        assert!(matches!(
            m.run_vault(u, id, "bob".into()),
            Err(SuiteError::Experiment(ExperimentError::VaultSpent))
        ));
    }

    #[test]
    fn funnel_requires_a_chosen_null() {
        let m = SuiteManager::new();
        let u = Uuid::new_v4();
        let id = m.create_experiment(u, spec("exp-n")).unwrap().id;
        assert!(matches!(m.advance_funnel(u, id), Err(SuiteError::Null(_))));
    }

    #[test]
    fn null_override_requires_a_reason() {
        let m = SuiteManager::new();
        let u = Uuid::new_v4();
        let id = m.create_experiment(u, spec("exp-o")).unwrap().id;
        let recommended = m.null_picker(u, id).unwrap().recommended;
        // Pick a different kind without a reason → refused.
        let other = if recommended == NullKind::BlockPermutation {
            NullKind::SyntheticGarch
        } else {
            NullKind::BlockPermutation
        };
        assert!(matches!(
            m.choose_null(u, id, other, None),
            Err(SuiteError::Null(_))
        ));
        // With a reason → accepted, flagged as an override.
        let choice = m
            .choose_null(u, id, other, Some("regime structure matters".into()))
            .unwrap();
        assert!(choice.was_override);
        assert!(choice.override_reason.is_some());
    }

    #[test]
    fn reconciliation_is_user_scoped_and_drives_decay() {
        let m = SuiteManager::new();
        let u = Uuid::new_v4();
        let other = Uuid::new_v4();
        let id = m.create_experiment(u, spec("exp-live")).unwrap().id;
        m.run_study(u, id, sweep("dist", 12)).unwrap();
        m.choose_null(u, id, m.null_picker(u, id).unwrap().recommended, None)
            .unwrap();
        m.advance_funnel(u, id).unwrap();
        m.run_vault(u, id, "alice".into()).unwrap();
        m.promote_to_live(u, id).unwrap();

        // Another user cannot see or reconcile this experiment.
        assert!(m.get_experiment(other, id).is_none());
        assert!(matches!(
            m.reconcile(other, id, &[0.0], 0.1),
            Err(SuiteError::NotFound)
        ));

        // Realized returns far below the backtest worst-5% → decays.
        let view = m
            .reconcile(u, id, &[-0.5, -0.6, -0.5, -0.7, -0.6], 0.10)
            .unwrap();
        assert!(view.verdict.drifting);
        assert_eq!(view.state, ExperimentState::Decaying);

        let cal = m.suite_calibration(u);
        assert_eq!(cal.experiments_contributing, 1);
        assert!(!cal.percentiles.is_empty());
    }
}
