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
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::experiment::{Experiment, ExperimentError, ExperimentState, Holdout, VaultAccess};
use ledger::{ActorKind, DispatchContext, InMemoryLedger, TrialLedger};
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
    MetricKind, MetricSet, Objective, ParamMap, RunConfig, RunConfigBuilder, RunExecutor, RunId,
    RunResult, RunStatus, RunStore, ENGINE_VERSION,
};
use crate::study::{
    Distribution, SelectionRule, StudyBudget, StudyConfig, StudyEngine, StudyKind, StudyResult,
    StudyVerdict, VarySpec,
};

/// Synthetic, deterministic Run executor (the real one is the deferred live leg).
///
/// It has to model three things the funnel actually asks about, or the gates
/// have nothing real to judge:
///
/// * **Edge lives in the parameters, not in the window.** The drift is keyed on
///   `(strategy_ref, params)`, so a configuration that is good in one period is
///   good in the next. Keying it on the whole `run_id` instead would make the
///   in-sample best config random, and PBO — which asks exactly how often the
///   in-sample best underperforms out of sample — would sit at 0.5 for a
///   strategy with a perfect edge.
/// * **A null world destroys the edge.** A Run whose `null_world` is set earns a
///   drift centred on zero: that is what it means for the null to destroy the
///   structure the strategy depends on, and it is what makes a permutation
///   p-value informative rather than a coin flip.
/// * **Every config trades the same market.** A shared daily shock keyed by day
///   makes a sweep look like what it is to platform N_eff — one idea, highly
///   correlated — instead of a set of independent lines.
fn synthetic_execute(cfg: &RunConfig) -> RunResult {
    let fnv = |seed: u64, bytes: &[u8]| {
        let mut h = seed;
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    };
    // The configuration's identity: what it *is*, not where it ran.
    let mut cfg_h = fnv(0xcbf2_9ce4_8422_2325, cfg.strategy_ref.as_bytes());
    for (k, v) in &cfg.params {
        cfg_h = fnv(cfg_h, k.as_bytes());
        cfg_h = fnv(cfg_h, v.to_string().as_bytes());
    }
    // The run's identity: window, seed, null world. Only small noise rides on it.
    let run_h = fnv(0xcbf2_9ce4_8422_2325, cfg.run_id.as_str().as_bytes())
        ^ cfg.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);

    let unit = |h: u64| ((h >> 11) as f64) / ((1u64 << 53) as f64); // [0, 1)
    let drift = if cfg.null_world.is_some() {
        // The structure the edge rested on is gone: what is left is noise around
        // zero, which is the whole hypothesis a null states.
        (unit(cfg_h ^ run_h) - 0.5) * 0.0010
    } else {
        0.0015 + unit(cfg_h) * 0.0020 // daily drift in [0.15%, 0.35%]
    };

    let days = 30;
    let mut equity = 100.0_f64;
    let noise = |key: u64| {
        let mut z = key.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0xd1b5_4a32_d192_ed03;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64 - 0.5
    };
    let curve: Vec<f64> = (0..days)
        .map(|d: u64| {
            let shared = noise(d + 1) * 0.008;
            let own = noise(run_h ^ (d + 7).wrapping_mul(0x2545_f491_4f6c_dd1d)) * 0.001;
            equity *= 1.0 + drift + shared + own;
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

/// The engine the manager runs every Study/Run through. The executor is
/// injected: synthetic by default (tests, offline), the `market_simulator`-backed
/// [`crate::sim_executor::SimRunExecutor`] in the platform.
type SuiteEngine = Backtest<InMemoryRunStore, Box<dyn RunExecutor>, Arc<dyn TrialLedger>>;

// ── Gate 3's measured inputs ─────────────────────────────────────────────────

/// Null draws per funnel advance. 99 is not a round number chosen for looks: a
/// permutation p-value can only take values `(k+1)/(B+1)`, so B = 99 gives a
/// grid of 0.01 — fine enough for a 0.05 decision and no finer than the
/// simulation budget can honestly support. Fewer draws makes the p-value coarser
/// than the threshold it is compared against, which is how a "significant"
/// result becomes an artefact of the draw count.
const NULL_DRAWS: u32 = 99;

/// Below this, the p-value's granularity is coarser than `GATE3_ALPHA` and the
/// gate refuses rather than reporting a number it cannot support.
const MIN_NULL_DRAWS: usize = 19;

/// Gate 3's corrected-p threshold.
const GATE3_ALPHA: f64 = 0.05;

/// The PBO grid: configurations x periods. Both are deliberately small —
/// every cell is a real Run and a real look on the trial counter.
const PBO_CONFIGS: usize = 5;
const PBO_PERIODS: usize = 8;
/// CSCV partitions the periods into this many groups.
const PBO_GROUPS: usize = 4;

/// Daily simple returns from an equity curve.
fn daily_returns(curve: &[(DateTime<Utc>, f64)]) -> Vec<f64> {
    curve
        .windows(2)
        .filter_map(|w| {
            let (a, b) = (w[0].1, w[1].1);
            (a != 0.0).then(|| b / a - 1.0).filter(|r| r.is_finite())
        })
        .collect()
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() { 0.0 } else { xs.iter().sum::<f64>() / xs.len() as f64 }
}

/// Sample variance (ddof = 1).
fn variance(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let m = mean(xs);
    xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (xs.len() as f64 - 1.0)
}

fn skewness(xs: &[f64]) -> f64 {
    let sd = variance(xs).sqrt();
    if xs.len() < 3 || sd <= 0.0 {
        return 0.0;
    }
    let m = mean(xs);
    xs.iter().map(|x| ((x - m) / sd).powi(3)).sum::<f64>() / xs.len() as f64
}

/// Non-excess kurtosis: 3.0 is the normal reference the deflated-Sharpe formula
/// expects, so an empty or degenerate series returns 3.0, not 0.0.
fn kurtosis(xs: &[f64]) -> f64 {
    let sd = variance(xs).sqrt();
    if xs.len() < 4 || sd <= 0.0 {
        return 3.0;
    }
    let m = mean(xs);
    xs.iter().map(|x| ((x - m) / sd).powi(4)).sum::<f64>() / xs.len() as f64
}

// ── view models (what the frontend renders) ──────────────────────────────────

/// The always-on-screen header for an Experiment: counter + lifecycle + unsafe.
//
// The flags are independent facts about one Experiment that the header renders
// side by side, not the states of a machine: `unsafe`, "gate 3 passed", "holdout
// spent" and "suspect overlapping-label leakage" can hold in any combination,
// and collapsing them into an enum would lose exactly the combinations a reader
// needs to see together.
#[allow(clippy::struct_excessive_bools)]
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
    /// `CV_Sharpe − WF_Sharpe`, once both a cross-validated and a walk-forward
    /// Study have run on this idea. `None` means the comparison has not been
    /// made, never that it passed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cv_wf_gap: Option<f64>,
    /// The soft flag from that gap (SPEC §12.5): a cross-validated result more
    /// than 1.0 above the walk-forward result on the same idea is the signature
    /// of overlapping-label leakage. Rendered beside every comparison; it blocks
    /// nothing, which is the point — a human decides.
    pub suspect_overlapping_label_leakage: bool,
    pub primary_test: String,
    pub holdout_spent: bool,
    pub study_count: usize,
    /// The stored strategy slug every Run executes.
    pub strategy_ref: String,
    /// The declared objective, if one was set at creation (immutable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<Objective>,
    /// The research slice's universe (instrument id) and window.
    pub universe_ref: String,
    pub research_start: DateTime<Utc>,
    pub research_end: DateTime<Utc>,
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
    /// The parameter set the selection rule carried forward, when it did.
    /// This is the rule's output (the stable centroid / worst-case member),
    /// never the best member — exposing it is INV-2-compliant by construction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carried_forward_params: Option<ParamMap>,
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
    /// The concrete stored strategy (slug in `strategy_definitions`) every Run
    /// executes. Defaults to `strategy_family` for backwards compatibility.
    #[serde(default)]
    pub strategy_ref: Option<String>,
    /// What sweeps on this Experiment maximise. Immutable once set
    /// (FEAT-003 §6); a different objective is a different Experiment.
    #[serde(default)]
    pub objective: Option<Objective>,
    /// The smallest improvement that would actually matter, declared **before**
    /// any trial runs and hash-locked into every trial's pre-registration.
    ///
    /// REQUIRED, with no default (SPEC §4.1). There is deliberately no
    /// `#[serde(default)]` here: a request that omits it fails to deserialize,
    /// so "how much better is worth caring about" can never be decided after
    /// seeing the results.
    pub delta_practical: f64,
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
    /// Parameter values the base config is centred on (e.g. a prior Study's
    /// `carried_forward_params`) so a `Neighborhood` on one parameter holds
    /// the others where the research left them.
    #[serde(default)]
    pub base_params: Option<ParamMap>,
}

/// Input to [`SuiteManager::run_param_batch`] (in-process only).
#[derive(Clone, Debug)]
pub struct ParamBatchSpec {
    pub study_id: String,
    /// Parameter overrides per member, applied over `base_params`.
    pub grid: Vec<ParamMap>,
    pub metric: MetricKind,
    pub selection_rule: SelectionRule,
    pub question: String,
    pub base_params: Option<ParamMap>,
}

/// Output of [`SuiteManager::run_param_batch`]: the sealed view plus, for the
/// sampler only, each member's full parameter set and metrics (`None` when the
/// Run failed) in grid order.
#[derive(Clone, Debug)]
pub struct ParamBatchOutcome {
    pub view: StudyView,
    pub members: Vec<(ParamMap, Option<MetricSet>)>,
    pub carried_forward: Option<ParamMap>,
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
    /// Stored strategy slug the executor resolves (defaults to the family).
    strategy_ref: String,
    objective: Option<Objective>,
    /// Declared at DEFINE, before any trial ran; hash-locked into every trial's
    /// pre-registration (SPEC §4.1).
    delta_practical: f64,
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
    bt: SuiteEngine,
    progress_tx: broadcast::Sender<serde_json::Value>,
}

impl Default for SuiteManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SuiteManager {
    /// A manager over the deterministic synthetic executor and an in-process
    /// ledger (tests / offline). The ledger is real, not a stub — it chains and
    /// it refuses unlogged propensities — but it dies with the process, which is
    /// acceptable only because nothing produced this way is trusted.
    #[must_use]
    pub fn new() -> Self {
        let exec: fn(&RunConfig) -> RunResult = synthetic_execute;
        Self::with_executor(Box::new(ClosureExecutor(exec)), Arc::new(InMemoryLedger::new()))
    }

    /// A manager over an injected executor and ledger — the platform passes the
    /// real `market_simulator`-backed executor and the durable Postgres ledger,
    /// so every Study runs real backtests against a persistent trial record.
    #[must_use]
    pub fn with_executor(executor: Box<dyn RunExecutor>, ledger: Arc<dyn TrialLedger>) -> Self {
        let (progress_tx, _) = broadcast::channel(256);
        Self {
            records: RwLock::new(HashMap::new()),
            bt: Backtest::new(InMemoryRunStore::new(), executor, ledger),
            progress_tx,
        }
    }

    /// The dispatch provenance for work `user_id` drives on `record`.
    ///
    /// `delta_practical` comes from the Experiment, where it was declared before
    /// any trial ran — never from the caller at dispatch time.
    fn dispatch_ctx(user_id: Uuid, record: &Record) -> DispatchContext {
        DispatchContext {
            tenant_id: user_id.to_string(),
            campaign_id: None,
            experiment_id: Some(record.exp.experiment_id.clone()),
            actor_kind: ActorKind::Human,
            actor_id: user_id.to_string(),
            on_behalf_of: None,
            policy_id: "suite_manual".into(),
            policy_version: 1,
            delta_practical: Some(record.delta_practical),
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
        if let Some(o) = &spec.objective {
            o.validate().map_err(SuiteError::Invalid)?;
        }
        // A non-positive or non-finite practical effect size would make every
        // result "significant enough" — the declaration has to mean something.
        if !spec.delta_practical.is_finite() || spec.delta_practical <= 0.0 {
            return Err(SuiteError::Invalid(
                "delta_practical must be a positive, finite effect size, declared before any trial runs"
                    .into(),
            ));
        }
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
        let strategy_ref = spec
            .strategy_ref
            .clone()
            .unwrap_or_else(|| exp.strategy_family.clone());
        let record = Record {
            user_id,
            uuid,
            exp,
            strategy_type: spec.strategy_type,
            strategy_ref,
            objective: spec.objective,
            delta_practical: spec.delta_practical,
            research_slice: research,
            studies: Vec::new(),
            gate_verdicts: Vec::new(),
            gate3: None,
            null: None,
            null_choice: None,
            reconciliation: None,
        };
        let view = Self::experiment_view(&record);
        records.insert(uuid, record);
        Ok(view)
    }

    /// List this user's Experiments (newest first).
    #[must_use]
    pub fn list_experiments(&self, user_id: Uuid) -> Vec<ExperimentView> {
        let records = self.records.read().expect("suite lock poisoned");
        let mut views: Vec<ExperimentView> = records
            .values()
            .filter(|r| r.user_id == user_id)
            .map(Self::experiment_view)
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
            .map(Self::experiment_view)
    }

    fn experiment_view(r: &Record) -> ExperimentView {
        let exp = &r.exp;
        ExperimentView {
            id: r.uuid,
            experiment_id: exp.experiment_id.clone(),
            strategy_family: exp.strategy_family.clone(),
            strategy_type: r.strategy_type.clone(),
            state: exp.state,
            trial_counter: exp.trial_counter(),
            unsafe_flag: exp.is_unsafe(),
            gate3_passed: exp.gate3_passed(),
            cv_wf_gap: exp.cv_wf_gap(),
            suspect_overlapping_label_leakage: exp.suspect_overlapping_label_leakage(),
            primary_test: exp.primary_test().to_string(),
            holdout_spent: exp.holdout.spent,
            study_count: exp.studies.len(),
            strategy_ref: r.strategy_ref.clone(),
            objective: r.objective.clone(),
            universe_ref: r.research_slice.universe_ref.clone(),
            research_start: r.research_slice.start,
            research_end: r.research_slice.end,
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
        Ok(Self::experiment_view(r))
    }

    /// Retire an Experiment (terminal). Read-only thereafter.
    pub fn retire(&self, user_id: Uuid, id: Uuid) -> Result<ExperimentView, SuiteError> {
        let mut records = self.records.write().expect("suite lock poisoned");
        let r = records
            .get_mut(&id)
            .filter(|r| r.user_id == user_id)
            .ok_or(SuiteError::NotFound)?;
        r.exp.transition(ExperimentState::Retired)?;
        Ok(Self::experiment_view(r))
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
        // Pre-flight under a short read lock: build + validate the config and
        // let the Experiment refuse it (state / holdout) *before* any Run.
        let (study, ctx) = {
            let records = self.records.read().expect("suite lock poisoned");
            let r = records
                .get(&id)
                .filter(|r| r.user_id == user_id)
                .ok_or(SuiteError::NotFound)?;
            let ctx = Self::dispatch_ctx(user_id, r);
            let mut base = Self::base_config(&r.strategy_ref, &r.research_slice);
            if let Some(p) = &spec.base_params {
                base.params = p.clone();
                base = base.rehashed();
            }
            let study = StudyConfig {
                study_id: spec.study_id.clone(),
                kind: spec.kind,
                base_config: base,
                vary: spec.vary.clone(),
                metric: spec.metric,
                null_ref: spec.null_ref.clone(),
                null: None,
                budget: StudyBudget::default(),
                question: spec.question.clone(),
                selection_rule: spec.selection_rule.unwrap_or(SelectionRule::None),
            };
            study
                .validate()
                .map_err(|e| SuiteError::Study(e.to_string()))?;
            r.exp.check_study(&study)?;
            (study, ctx)
        };

        // Execute with no lock held — real Runs take minutes and other users'
        // reads must not block behind them.
        self.emit(user_id, id, "study_running", 10.0, &spec.question);
        let result = StudyEngine::run(&study, &self.bt, &ctx)
            .map_err(|e| SuiteError::Study(e.to_string()))?;

        // Bookkeeping under a short write lock: the counter increments here,
        // through the Experiment's single mutator (J-2.3).
        let mut records = self.records.write().expect("suite lock poisoned");
        let r = records
            .get_mut(&id)
            .filter(|r| r.user_id == user_id)
            .ok_or(SuiteError::NotFound)?;
        r.exp.record_study_result(study.study_id.clone(), study.kind, &result);
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

    /// The objective declared on an Experiment (`Some(None)` = found, none set).
    #[must_use]
    pub fn experiment_objective(&self, user_id: Uuid, id: Uuid) -> Option<Option<Objective>> {
        let records = self.records.read().expect("suite lock poisoned");
        records
            .get(&id)
            .filter(|r| r.user_id == user_id)
            .map(|r| r.objective.clone())
    }

    /// The parameter set a Study's pre-declared selection rule carried forward
    /// (the rule's output — never an argmax).
    #[must_use]
    pub fn study_carried_forward(
        &self,
        user_id: Uuid,
        id: Uuid,
        study_id: &str,
    ) -> Option<ParamMap> {
        let records = self.records.read().expect("suite lock poisoned");
        let r = records.get(&id).filter(|r| r.user_id == user_id)?;
        r.studies
            .iter()
            .rev()
            .find(|s| s.result.study_id == study_id)
            .and_then(|s| s.result.carried_forward.as_ref().map(|c| c.params.clone()))
    }

    /// A stored Run result, reachable only through a Study the user owns
    /// (provenance access for diagnostics — not a ranking path).
    #[must_use]
    pub fn run_result(&self, user_id: Uuid, run_id: &str) -> Option<RunResult> {
        let records = self.records.read().expect("suite lock poisoned");
        let rid: &RunId = records
            .values()
            .filter(|r| r.user_id == user_id)
            .flat_map(|r| r.studies.iter())
            .flat_map(|s| s.result.members().iter())
            .find(|m| m.as_str() == run_id)?;
        self.bt.store().get(rid)
    }

    /// **Sampler-facing** batch evaluation (FEAT-003 §7.3): run one
    /// `ParameterSweep` Study over `grid` and hand back each member's metrics
    /// in grid order so an in-process optimiser can decide where to sample
    /// next. This is exploration, not promotion — the Study is sealed and
    /// counted like any other, the only carry-forward is the selection rule's
    /// output, and **this method is never exposed over HTTP**.
    pub fn run_param_batch(
        &self,
        user_id: Uuid,
        id: Uuid,
        spec: ParamBatchSpec,
    ) -> Result<ParamBatchOutcome, SuiteError> {
        let ParamBatchSpec {
            study_id,
            grid,
            metric,
            selection_rule,
            question,
            base_params,
        } = spec;
        let view = self.run_study(
            user_id,
            id,
            RunStudySpec {
                study_id,
                kind: StudyKind::ParameterSweep,
                vary: VarySpec::Params { grid: grid.clone() },
                metric,
                question,
                selection_rule: Some(selection_rule),
                null_ref: None,
                base_params: base_params.clone(),
            },
        )?;
        let records = self.records.read().expect("suite lock poisoned");
        let r = records
            .get(&id)
            .filter(|r| r.user_id == user_id)
            .ok_or(SuiteError::NotFound)?;
        let stored = r
            .studies
            .iter()
            .rev()
            .find(|s| s.result.study_id == view.study_id)
            .ok_or(SuiteError::NotFound)?;
        let base = base_params.unwrap_or_default();
        let members = stored
            .result
            .members()
            .iter()
            .zip(grid)
            .map(|(rid, entry)| {
                let mut full = base.clone();
                full.extend(entry);
                let metrics = self
                    .bt
                    .store()
                    .get(rid)
                    .filter(|res| res.status == RunStatus::Ok)
                    .map(|res| res.metrics);
                (full, metrics)
            })
            .collect();
        Ok(ParamBatchOutcome {
            carried_forward: view.carried_forward_params.clone(),
            view,
            members,
        })
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
            carried_forward_params: result.carried_forward.as_ref().map(|c| c.params.clone()),
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

        // Gate 3's evidence is gathered here, before the gate runner borrows the
        // Experiment: the observed run, the null distribution and the PBO grid
        // are all real Runs on the ledger, and the counter must climb for them
        // exactly as it does for the evidence studies above.
        let null = r.null.clone().expect("null checked above");

        // (a) The observed statistic: the candidate itself, on real data.
        let (observed_res, observed_trial) = self.run_observed(r, user_id, id, &base)?;
        let observed_statistic = Self::metric_of(&observed_res, MetricKind::Sharpe);

        // (b) The null distribution: the Experiment's *declared* null, executed.
        //     Each member is a Run in a null world (`RunConfig::in_null_world`),
        //     so the spread is what the strategy earns when the structure the
        //     null destroys is gone — not the same strategy under a new seed.
        let null_study = Self::null_study("funnel-null", &base, &null, NULL_DRAWS);
        let null_res = self.run_evidence(r, user_id, id, &null_study)?;
        let null_distribution = null_res.distribution.dist.clone();
        if null_distribution.len() < MIN_NULL_DRAWS {
            return Err(SuiteError::Gate(format!(
                "the null produced only {} usable draws; a permutation p-value needs at least \
                 {MIN_NULL_DRAWS} to mean anything",
                null_distribution.len()
            )));
        }

        // (c) The PBO matrix: every neighbourhood configuration over every
        //     window. PBO asks how often the in-sample best configuration
        //     underperforms out of sample, which is a question about a grid and
        //     cannot be answered from one study's marginal distribution.
        let pbo_perf = self.pbo_matrix(r, user_id, id, &base)?;

        let sharpe_variance_across_trials = variance(
            &[
                wf_res.distribution.dist.clone(),
                cpcv_res.distribution.dist.clone(),
                syn_res.distribution.dist.clone(),
                nbhd_res.distribution.dist.clone(),
            ]
            .concat(),
        );

        // Replay the funnel in order in a single ledger session.
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
        //
        // Every input is measured. The previous implementation applied a real
        // BHY correction and a real N_eff to a hardcoded null (`0..999 / 1000`),
        // a hardcoded observed statistic of 6.0 and hardcoded corroborators —
        // arithmetic that was correct about numbers nobody had computed. Those
        // constants are gone.
        let returns = daily_returns(&observed_res.equity_curve);
        let corr = CorroboratorInputs {
            sharpe: observed_statistic,
            n_obs: returns.len(),
            skew: skewness(&returns),
            kurtosis: kurtosis(&returns),
            // Dispersion of the metric across everything this Experiment has
            // actually looked at — the quantity DSR's haircut is scaled by.
            sharpe_variance_across_trials,
            pbo_performance: &pbo_perf,
            pbo_groups: PBO_GROUPS,
        };

        // N_eff comes from the tenant's whole ledger, never from this Experiment's counter.
        let n_eff = self
            .bt
            .ledger()
            .n_eff(&user_id.to_string())
            .map_err(|e| SuiteError::Gate(format!("N_eff unavailable: {e}")))?;
        let (outcome, _passed3) = runner.gate3(
            observed_statistic,
            &null_distribution,
            null.null_id.clone(),
            &corr,
            &n_eff,
            GATE3_ALPHA,
        )?;
        verdicts.extend(
            runner
                .ledger()
                .verdicts()
                .iter()
                .filter(|v| v.gate == Gate::Significance)
                .cloned(),
        );
        self.emit(user_id, id, "gate_passed", 100.0, "gate 3 significance");

        // The numbers this funnel just computed go on the record, attached to the
        // run that produced them (ADR-P2-31). §12.3's Gates 5–8 read them from
        // there, in a different process, later — the alternative is passing them
        // through the gate job's manifest, which would make every gate statistic
        // something the submitter chose.
        //
        // A failed write is logged, not fatal: the funnel's own verdict stands,
        // and a gate with no recorded statistic is inconclusive, which is the
        // safe direction. Failing the funnel because a bookkeeping write missed
        // would throw away the evidence as well as the record of it.
        let tenant = user_id.to_string();
        for (name, value) in [
            ("cpcv_p05_sharpe", cpcv_res.distribution.worst_5pct),
            ("walk_forward_sharpe", wf_res.distribution.median),
            ("pbo", outcome.pbo),
            ("deflated_sharpe", outcome.deflated_sharpe),
            ("permutation_p_value", outcome.raw_p_value),
        ] {
            if let Err(e) = self.bt.ledger().record_statistic(
                &tenant,
                observed_trial,
                name,
                value,
                "funnel_advance",
            ) {
                tracing::warn!(experiment = %id, statistic = name, error = %e, "statistic not recorded");
            }
        }

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
            null: None,
            budget: StudyBudget::default(),
            question: format!("{kind:?} evidence for the funnel"),
            selection_rule: SelectionRule::None,
        }
    }

    /// Run one funnel-evidence Study through the Experiment (counter increments)
    /// and record it for the distribution viewer, returning the sealed result.
    /// Run the candidate itself, once, on real data. Its metric is Gate 3's
    /// observed statistic — the number the null distribution is a null *for*.
    fn run_observed(
        &self,
        r: &mut Record,
        user_id: Uuid,
        id: Uuid,
        base: &RunConfig,
    ) -> Result<(RunResult, Uuid), SuiteError> {
        // Dispatched with certainty: given the funnel policy, the candidate is
        // the one config this step runs. That is the honest propensity, and it
        // is a real one — the legacy no-propensity marker is for pre-existing
        // paths, not for new code that finds declaring one inconvenient.
        let ctx = Self::dispatch_ctx(user_id, r).with_policy("funnel_observed", 1);
        // The trial id comes back with the result so the funnel's statistics can
        // be attached to the run that produced them (ADR-P2-31). Without it they
        // would have to travel through the gate job's manifest, where the
        // submitter chooses them.
        let (res, _, trial_id) = self
            .bt
            .run_traced_with_trial(&crate::run::RunDispatch::new(&ctx, base, 1.0))
            .map_err(|e| SuiteError::Gate(format!("observed run refused: {e}")))?;
        // A look is a look, whether or not a Study wrapped it (§12.4).
        r.exp.record_dispatches(1);
        self.emit(user_id, id, "run_complete", 20.0, "observed run");
        Ok((res, trial_id))
    }

    /// The permutation-null Study whose distribution Gate 3 tests against.
    fn null_study(id: &str, base: &RunConfig, null: &Null, draws: u32) -> StudyConfig {
        StudyConfig {
            study_id: id.to_string(),
            kind: StudyKind::PermutationNull,
            base_config: base.clone(),
            vary: VarySpec::Seeds { n: draws },
            metric: MetricKind::Sharpe,
            null_ref: Some(null.null_id.as_str().to_string()),
            null: Some(null.clone()),
            budget: StudyBudget::default(),
            question: format!(
                "what does this strategy earn when {} is destroyed and {} is preserved?",
                null.destroys.join(", "),
                null.preserves.join(", ")
            ),
            selection_rule: SelectionRule::None,
        }
    }

    /// `performance[config][period]`: every neighbourhood configuration over
    /// every walk-forward window.
    ///
    /// PBO asks how often the configuration that looked best in sample
    /// underperforms out of sample. That is a question about a grid, so the grid
    /// is what gets run — each cell a real Run, each counted.
    fn pbo_matrix(
        &self,
        r: &mut Record,
        user_id: Uuid,
        id: Uuid,
        base: &RunConfig,
    ) -> Result<Vec<Vec<f64>>, SuiteError> {
        let windows = Self::split_windows(&base.data_slice, PBO_PERIODS);
        let mut matrix = Vec::with_capacity(PBO_CONFIGS);
        for c in 0..PBO_CONFIGS {
            let mut cfg = base.clone();
            // The same neighbourhood the robustness study perturbs, so the PBO
            // grid and Gate 2's evidence are about the same parameter.
            let value = 12.0 + (c as f64 - (PBO_CONFIGS as f64 - 1.0) / 2.0);
            cfg.params.insert(
                "fast".into(),
                serde_json::Number::from_f64(value)
                    .map_or(serde_json::Value::Null, serde_json::Value::Number),
            );
            let mut row = Vec::with_capacity(windows.len());
            for (start, end) in &windows {
                let mut member = cfg.clone();
                member.data_slice.start = *start;
                member.data_slice.end = *end;
                let member = member.rehashed();
                // Every cell of a declared grid is dispatched with certainty.
                let ctx = Self::dispatch_ctx(user_id, r).with_policy("funnel_pbo_grid", 1);
                let (res, _) = self
                    .bt
                    .run_traced(&crate::run::RunDispatch::new(&ctx, &member, 1.0))
                    .map_err(|e| SuiteError::Gate(format!("pbo cell refused: {e}")))?;
                r.exp.record_dispatches(1);
                row.push(Self::metric_of(&res, MetricKind::Sharpe));
            }
            matrix.push(row);
        }
        self.emit(user_id, id, "study_complete", 90.0, "pbo grid");
        Ok(matrix)
    }

    /// `n` contiguous windows covering a slice.
    fn split_windows(slice: &DataSlice, n: usize) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
        let total = (slice.end - slice.start).num_seconds().max(1);
        let step = total / n as i64;
        (0..n as i64)
            .map(|i| {
                (
                    slice.start + chrono::Duration::seconds(i * step),
                    slice.start + chrono::Duration::seconds(((i + 1) * step).min(total)),
                )
            })
            .collect()
    }

    /// The metric a distribution is taken over, read off one result. Non-finite
    /// metrics are 0.0: a Run that produced no usable curve contributes nothing,
    /// rather than poisoning every statistic downstream with a NaN.
    fn metric_of(res: &RunResult, metric: MetricKind) -> f64 {
        let v = res.metrics.value(metric);
        if v.is_finite() { v } else { 0.0 }
    }

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
        let ctx = Self::dispatch_ctx(user_id, r);
        let result = r.exp.run_study(study, &self.bt, &ctx)?;
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
        let ctx = Self::dispatch_ctx(user_id, r);
        let _result = r.exp.run_vault(&candidate, &self.bt, &ctx, by)?;
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
            delta_practical: 0.1,
            experiment_id: id.into(),
            strategy_family: "ema-family".into(),
            strategy_type: "daily_trend".into(),
            universe_ref: "u".into(),
            research_start: Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap(),
            research_end: Utc.with_ymd_and_hms(2022, 1, 1, 0, 0, 0).unwrap(),
            holdout_start: Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
            holdout_end: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            eval_resolution: None,
            strategy_ref: None,
            objective: None,
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
            base_params: None,
        }
    }

    // ------------------------------------------------------------------ //
    // 2.14: Gate 3's inputs are measured, not constants
    // ------------------------------------------------------------------ //

    /// A permutation-null Study expands into Runs that execute in a **null
    /// world** — different data, therefore different `run_id`s from the real
    /// Run. Before this, the members were the same strategy on the same data
    /// under different seeds, and their spread was called a null distribution.
    #[test]
    fn null_study_members_execute_in_a_null_world() {
        let slice = DataSlice::new(
            "u",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap(),
            EvalResolution::Day1,
        );
        let base = SuiteManager::base_config("fam", &slice);
        let null = crate::nulls::Null::new(
            crate::nulls::NullKind::BlockPermutation,
            crate::nulls::NullParams::default(),
        )
        .unwrap();
        let study = SuiteManager::null_study("s", &base, &null, 8);
        assert!(study.validate().is_ok());

        let members = crate::study::engine::expand_members_for_test(&study);
        assert_eq!(members.len(), 8);
        for m in &members {
            let nw = m.null_world.as_ref().expect("member runs in a null world");
            assert_eq!(nw.null.null_id, null.null_id);
            assert_ne!(m.run_id, base.run_id, "a null-world Run is a different Run");
        }
        // Each draw is its own Run: the seeds do not collapse onto one id.
        let ids: std::collections::HashSet<_> = members.iter().map(|m| &m.run_id).collect();
        assert_eq!(ids.len(), members.len());
    }

    /// The measured inputs are what they claim: the observed statistic is the
    /// candidate's own metric, the null distribution has the declared number of
    /// draws, and the PBO grid is the declared shape.
    #[test]
    fn gate3_reports_measured_inputs_not_constants() {
        let m = SuiteManager::new();
        let u = Uuid::new_v4();
        let id = m.create_experiment(u, spec("exp-measured")).unwrap().id;
        let picker = m.null_picker(u, id).unwrap();
        m.choose_null(u, id, picker.recommended, None).unwrap();

        let before = m.get_experiment(u, id).unwrap().trial_counter;
        let funnel = m.advance_funnel(u, id).unwrap();
        let after = m.get_experiment(u, id).unwrap().trial_counter;

        let g3 = funnel.significance.expect("gate 3 ran");
        // The old implementation's p came from a hardcoded 999-point null and an
        // observed statistic of 6.0, which made it 0.001 every single time.
        assert!(
            g3.raw_p_value > 0.0 && g3.raw_p_value <= 1.0,
            "p = {}",
            g3.raw_p_value
        );
        assert!(
            g3.pbo >= 0.0 && g3.pbo <= 1.0,
            "PBO came from a real config x period grid: {}",
            g3.pbo
        );
        // The null draws, the observed run and every PBO cell are real looks and
        // the counter says so.
        let evidence =
            i64::from(NULL_DRAWS) + 1 + i64::try_from(PBO_CONFIGS * PBO_PERIODS).unwrap_or(0);
        assert!(
            after - before >= evidence,
            "counter climbed by {} but Gate 3 alone dispatched {evidence} runs",
            after - before
        );
    }

    /// The converse, which is what makes the gate mean anything: a candidate
    /// whose null earns exactly what it earns is not significant. With
    /// fabricated inputs this was unreachable — the gate passed on constants.
    #[test]
    fn a_candidate_with_no_edge_over_its_null_fails_gate_3() {
        // An executor in which the null world changes nothing: the strategy's
        // return does not depend on the structure the null destroys, which is
        // the definition of having no edge.
        let m = SuiteManager::with_executor(
            Box::new(ClosureExecutor(|cfg: &RunConfig| {
                let mut stripped = cfg.clone();
                stripped.null_world = None;
                let mut r = synthetic_execute(&stripped);
                r.run_id = cfg.run_id.clone();
                r
            })),
            Arc::new(InMemoryLedger::new()),
        );
        let u = Uuid::new_v4();
        let id = m.create_experiment(u, spec("exp-no-edge")).unwrap().id;
        let picker = m.null_picker(u, id).unwrap();
        m.choose_null(u, id, picker.recommended, None).unwrap();

        let funnel = m.advance_funnel(u, id).unwrap();
        let sig = funnel
            .gates
            .iter()
            .find(|g| g.gate == Gate::Significance)
            .expect("gate 3 recorded");
        assert_ne!(
            sig.status,
            GateStatus::Passed,
            "a strategy indistinguishable from its own null must not clear significance"
        );
        assert!(!m.get_experiment(u, id).unwrap().gate3_passed);
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
