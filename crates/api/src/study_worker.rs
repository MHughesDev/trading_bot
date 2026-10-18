//! The EXPERIMENT phase's worker, and BASELINE's (SPEC §10, checklist 2.1;
//! OPEN-TASKS C1.1).
//!
//! # What was broken
//!
//! `campaign_driver::child_kind` maps EXPERIMENT to `JobKind::Study` and BASELINE
//! to `JobKind::Backtest`. Neither kind had a worker registered, so a campaign
//! that got past DIAGNOSE and HYPOTHESIZE submitted a child no pool would ever run
//! successfully: the claim failed `no_worker` and the driver stopped with the log
//! where it was. Everything the campaign had spent thinking was then unreachable.
//!
//! # The contract a campaign has to meet (ADR-C-01)
//!
//! A campaign's DEFINE is immutable and hash-locked, and `mlops.campaign` has no
//! column that says *what strategy is being searched, on which data*. That is
//! declared where the DEFINE already carries free-form structure:
//! `search_space.evaluation`, an [`EvaluationSpec`]. A campaign without one fails
//! its first EXPERIMENT with `no_evaluation_spec` naming the fix, which is a
//! better failure than a campaign that runs against a strategy nobody chose.
//! *Rejected alternative:* a new column on `mlops.campaign` — it would have meant
//! a migration against a table whose immutability trigger is the point, for a
//! value `search_space` already holds under the same hash.
//!
//! # What EXPERIMENT does, and does not, apply (ADR-C-04)
//!
//! It runs a sweep over the parameter space the HYPOTHESIZE worker proposed
//! ranges for — the proposal's `ranges`, narrowing (never widening) what the
//! strategy declares — with the standing incumbent's parameters held as the base.
//! It does **not** apply the proposal's structural half (`add_condition_filter`,
//! `swap_feature`, ...). Those moves edit a strategy graph, and §8.3 says a
//! graph edit is machine-checked by a graph validator, a feature-lane lookup and
//! a regime-label check that this build does not have (`Pending`). Applying an
//! edit that cannot be validated would mean the campaign evaluated something the
//! proposer did not describe. The result therefore says `structural_move_applied:
//! false` and names the move, so a reader can see that a campaign's generations
//! searched parameters and did not change structure.
//!
//! # Where the heavy lifting lives
//!
//! Experiments, Studies and sweeps already exist as `backtest::suite::SuiteManager`
//! and `research::run_sweep`, reached by HTTP and by the agent's tools. The worker
//! drives them through the [`CampaignEvaluator`] trait, so the campaign logic is
//! tested against a fake and only [`SuiteEvaluator`] touches the real engine.

#![allow(clippy::result_large_err)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use backtest::run::{MetricKind, Objective, ParamMap, RunStatus};
use backtest::study::{StudyKind, VarySpec};
use backtest::suite::{CreateExperimentSpec, RunStudySpec, SuiteManager};
use chrono::{DateTime, Utc};
use jobs::store::JobStore;
use jobs::{Cost, JobContext, JobError, JobKind, JobOutput, Progress, Worker};
use ledger::phase::CampaignPhase;
use research::{Narrow, SamplerKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use super::compare_worker::standing_incumbent;
use crate::knowledge::RegimeLabel;
use crate::knowledge_routes::{ArmStore, PriorStore};
use crate::research::{ResearchManager, StartError, StartSweepBody, SweepStatus};

/// Where in `mlops.campaign.search_space` the evaluation block lives.
pub const EVALUATION_KEY: &str = "evaluation";

/// How often a running sweep is polled.
const SWEEP_POLL: Duration = Duration::from_secs(2);
/// How long a sweep waits for a free research permit before giving up.
const BUSY_RETRIES: u32 = 360;

fn default_runs() -> u32 {
    24
}
fn default_replicates() -> usize {
    backtest::stats::compare::TARGET_REPLICATES
}

/// A closed interval of time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// What a campaign searches and on which data, declared at DEFINE.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvaluationSpec {
    /// The stored strategy (slug in `strategy_definitions`) every run executes.
    pub strategy_ref: String,
    /// Declared type; seeds the null recommendation, never a default.
    pub strategy_type: String,
    pub universe_ref: String,
    /// Where the search and the comparison happen.
    pub research: Window,
    /// The sealed tail. Never touched here; it exists so the Experiment is
    /// constructible and the study engine can refuse anything that reaches it.
    pub holdout: Window,
    #[serde(default)]
    pub eval_resolution: Option<String>,
    /// What the campaign maximises, and its constraints.
    pub objective: Objective,
    /// The incumbent before any candidate has been promoted. Empty means the
    /// strategy's own defaults.
    #[serde(default)]
    pub incumbent_params: ParamMap,
    /// Sampled runs per sweep.
    #[serde(default = "default_runs")]
    pub max_runs: u32,
    /// Replicates (windows) per comparison, clamped to the protocol's range.
    #[serde(default = "default_replicates")]
    pub replicates: usize,
    #[serde(default)]
    pub sampler: SamplerKind,
    /// The ROPE half-width, declared. Absent means a multiple of
    /// `delta_practical`; see `compare_worker::DEFAULT_SCALE_MULTIPLE`.
    #[serde(default)]
    pub rope_scale: Option<f64>,
}

impl EvaluationSpec {
    /// Read the block out of a campaign's `search_space`.
    ///
    /// # Errors
    /// The block is absent, does not parse, or is inconsistent.
    pub fn from_search_space(search_space: &Value) -> Result<Self, String> {
        let block = search_space.get(EVALUATION_KEY).ok_or_else(|| {
            format!(
                "this campaign's search_space has no `{EVALUATION_KEY}` block. It must declare \
                 strategy_ref, strategy_type, universe_ref, research and holdout windows and an \
                 objective at DEFINE; the DEFINE is immutable, so a campaign defined without one \
                 has to be redefined"
            )
        })?;
        let spec: Self = serde_json::from_value(block.clone())
            .map_err(|e| format!("`{EVALUATION_KEY}` does not parse: {e}"))?;
        spec.validate()?;
        Ok(spec)
    }

    /// Structural checks that hold before any engine is involved.
    ///
    /// # Errors
    /// A reason naming the field.
    pub fn validate(&self) -> Result<(), String> {
        if self.strategy_ref.trim().is_empty() || self.universe_ref.trim().is_empty() {
            return Err("strategy_ref and universe_ref must be named".into());
        }
        if self.research.end <= self.research.start {
            return Err("the research window must end after it starts".into());
        }
        if self.holdout.end <= self.holdout.start {
            return Err("the holdout window must end after it starts".into());
        }
        if self.research.start < self.holdout.end && self.holdout.start < self.research.end {
            return Err(
                "the research window overlaps the holdout; the holdout is sealed and the \
                 search must not be able to see it"
                    .into(),
            );
        }
        if self.max_runs == 0 {
            return Err("max_runs must be at least 1".into());
        }
        if let Some(s) = self.rope_scale {
            if !s.is_finite() || s <= 0.0 {
                return Err("rope_scale must be positive and finite".into());
            }
        }
        self.objective.validate()
    }
}

/// [`EvaluationSpec::from_search_space`] as a job error.
///
/// # Errors
/// A terminal `no_evaluation_spec`.
pub fn evaluation_spec(search_space: &Value) -> Result<EvaluationSpec, JobError> {
    EvaluationSpec::from_search_space(search_space).map_err(|why| {
        JobError::logic(
            ledger::TerminalReason::IntegrityRejected,
            "no_evaluation_spec",
            why,
        )
    })
}

/// The campaign's declared `delta_practical`, gate profile and `search_space`.
///
/// Read from the row rather than carried in a manifest: the DEFINE is immutable
/// and the row is where it lives, so a restarted worker reads what the first one
/// did.
///
/// # Errors
/// An unknown campaign, or a backend failure.
pub async fn campaign_delta_and_profile(
    pg: &PgPool,
    tenant: &str,
    campaign_id: Uuid,
) -> Result<(f64, String, Value), JobError> {
    let mut tx = ledger::pg::tenant_tx(pg, tenant)
        .await
        .map_err(|e| JobError::infrastructure(format!("tenant transaction: {e}")))?;
    let row: Option<(f64, String, Value)> = sqlx::query_as(
        "SELECT delta_practical, gates_profile, search_space
           FROM mlops.campaign WHERE campaign_id = $1",
    )
    .bind(campaign_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| JobError::infrastructure(format!("reading the campaign: {e}")))?;
    let _ = tx.commit().await;
    row.ok_or_else(|| {
        JobError::logic(
            ledger::TerminalReason::IntegrityRejected,
            "unknown_campaign",
            "the manifest names a campaign this tenant has not defined",
        )
    })
}

/// The platform's seed for a campaign position.
///
/// Derived from the campaign and the fold position and nothing else. There is no
/// argument through which a caller could choose it: a caller that could would,
/// eventually, choose the seed where its candidate wins (ADR-P2-02).
#[must_use]
pub fn replicate_seed(campaign_id: Uuid, seq: i64) -> u64 {
    let mut h = Sha256::new();
    h.update(campaign_id.as_bytes());
    h.update(b"\x1fseed\x1f");
    h.update(seq.to_be_bytes());
    let d = h.finalize();
    u64::from_be_bytes(d[..8].try_into().expect("sha256 is 32 bytes"))
}

/// The experiment id a campaign's Backtest Suite experiment goes by.
///
/// `campaign_trials_counted` matches on this exact form, which is how trials the
/// suite registers under an experiment (it cannot tag them with a campaign) are
/// found by the budget rule.
#[must_use]
pub fn experiment_name(campaign_id: Uuid) -> String {
    format!("campaign-{campaign_id}")
}

/// The proposal's ranges as the narrowing a sweep accepts.
///
/// A range can only tighten what the strategy declares; widening is refused one
/// level down by `SearchSpace::from_definition`, which is the right place for the
/// refusal because only it knows the declaration.
///
/// # Errors
/// A `ranges` entry that is not a dimension.
pub fn narrowing_from_proposal(proposal: &Value) -> Result<BTreeMap<String, Narrow>, String> {
    let Some(ranges) = proposal.get("ranges").and_then(Value::as_array) else {
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    for r in ranges {
        let dim: research::space::Dim = serde_json::from_value(r.clone())
            .map_err(|e| format!("a proposed range does not parse: {e}"))?;
        #[allow(clippy::cast_precision_loss)]
        let narrow = match &dim.kind {
            research::space::DimKind::Int { min, max, .. } => Narrow {
                min: Some(*min as f64),
                max: Some(*max as f64),
                choices: None,
            },
            research::space::DimKind::Float { min, max, .. } => Narrow {
                min: Some(*min),
                max: Some(*max),
                choices: None,
            },
            research::space::DimKind::Enum { choices } => Narrow {
                min: None,
                max: None,
                choices: Some(choices.clone()),
            },
        };
        out.insert(dim.name, narrow);
    }
    Ok(out)
}

// ───────────────────────────────────────────────────────────────────────────────
// the engine seam
// ───────────────────────────────────────────────────────────────────────────────

/// One sweep, as the evaluator is asked for it.
#[derive(Clone, Debug)]
pub struct SweepCall {
    pub experiment: Uuid,
    pub narrowing: BTreeMap<String, Narrow>,
    pub base_params: ParamMap,
    pub seed: u64,
    pub question: String,
    pub sampler: SamplerKind,
    pub max_runs: u32,
}

/// What a sweep produced, without the sealed distribution's members.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SweepOutcome {
    /// The neighbourhood study's carried-forward member: the only candidate a
    /// sweep may hand on, and never an argmax.
    pub candidate: Option<ParamMap>,
    pub trials_consumed: u32,
    pub n_sampled: u32,
    pub n_feasible: u32,
    pub plateau: Option<bool>,
    pub study_ids: Vec<String>,
    /// The sealed-metric score of every sampled Run, in draw order. **Not serialised**: the
    /// EXPERIMENT job's result is readable by the agent and INV-2 forbids a score per sample
    /// in anything it reads (see `research::SweepReport::sample_scores`). The platform
    /// records it under `knowledge.write` as the campaign's arm (ADR-RB-11).
    #[serde(skip)]
    pub sample_scores: Vec<f64>,
}

/// One configuration measured over several windows.
#[derive(Clone, Debug)]
pub struct ReplicateCall {
    pub experiment: Uuid,
    pub study_id: String,
    pub params: ParamMap,
    pub windows: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    pub metric: MetricKind,
    pub question: String,
    /// Trading costs scaled by this many hundredths of a percent for every Run
    /// (`+2000` = +20 %); zero is the modelled cost. The cost axis of the paired
    /// replicate grid (SPEC §11.5).
    pub cost_scale_bp: i32,
}

/// What a campaign needs from the evaluation engine.
#[async_trait::async_trait]
pub trait CampaignEvaluator: Send + Sync {
    /// The campaign's experiment, created on first use.
    async fn ensure_experiment(
        &self,
        user: Uuid,
        campaign_id: Uuid,
        spec: &EvaluationSpec,
        delta_practical: f64,
    ) -> Result<Uuid, String>;

    /// Search the proposed space around the incumbent.
    async fn sweep(
        &self,
        user: Uuid,
        call: SweepCall,
        cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<SweepOutcome, String>;

    /// Measure one configuration on each window. One entry per window, in
    /// window order; `None` where the run failed. A result whose length is not
    /// the window count is an error, because a misaligned pair is the one thing
    /// a paired comparison cannot survive.
    async fn replicate_metrics(
        &self,
        user: Uuid,
        call: ReplicateCall,
        cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<Vec<Option<f64>>, String>;
}

/// The production evaluator: the Backtest Suite and the research sweep manager.
pub struct SuiteEvaluator {
    suite: Arc<SuiteManager>,
    research: Arc<ResearchManager>,
}

impl SuiteEvaluator {
    #[must_use]
    pub fn new(suite: Arc<SuiteManager>, research: Arc<ResearchManager>) -> Self {
        Self { suite, research }
    }
}

#[async_trait::async_trait]
impl CampaignEvaluator for SuiteEvaluator {
    async fn ensure_experiment(
        &self,
        user: Uuid,
        campaign_id: Uuid,
        spec: &EvaluationSpec,
        delta_practical: f64,
    ) -> Result<Uuid, String> {
        let name = experiment_name(campaign_id);
        let find = |suite: &SuiteManager| {
            suite
                .list_experiments(user)
                .into_iter()
                .find(|e| e.experiment_id == name)
                .map(|e| e.id)
        };
        if let Some(id) = find(&self.suite) {
            return Ok(id);
        }
        let created = self.suite.create_experiment(
            user,
            CreateExperimentSpec {
                experiment_id: name.clone(),
                strategy_family: spec.strategy_ref.clone(),
                strategy_type: spec.strategy_type.clone(),
                universe_ref: spec.universe_ref.clone(),
                research_start: spec.research.start,
                research_end: spec.research.end,
                holdout_start: spec.holdout.start,
                holdout_end: spec.holdout.end,
                eval_resolution: spec.eval_resolution.clone(),
                strategy_ref: Some(spec.strategy_ref.clone()),
                objective: Some(spec.objective.clone()),
                delta_practical,
                target: None,
            },
        );
        match created {
            Ok(view) => Ok(view.id),
            // Lost a race with a re-queued worker: the other one created it.
            Err(backtest::suite::SuiteError::AlreadyExists) => {
                find(&self.suite).ok_or_else(|| "the experiment vanished".to_string())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    async fn sweep(
        &self,
        user: Uuid,
        call: SweepCall,
        cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<SweepOutcome, String> {
        let body = StartSweepBody {
            experiment_id: call.experiment,
            strategy_ref: None,
            objective: None,
            narrowing: call.narrowing,
            sampler: call.sampler,
            max_runs: Some(call.max_runs),
            batch_size: None,
            seed: Some(call.seed),
            question: call.question,
            base_params: (!call.base_params.is_empty()).then_some(call.base_params),
        };
        let mut attempts = 0;
        let id = loop {
            match self.research.start(user, body.clone()) {
                Ok(id) => break id,
                // The research pool is bounded; another sweep holds the permit.
                Err(StartError::Busy) if attempts < BUSY_RETRIES && !cancelled() => {
                    attempts += 1;
                    tokio::time::sleep(SWEEP_POLL).await;
                }
                Err(e) => return Err(e.to_string()),
            }
        };
        loop {
            if cancelled() {
                self.research.cancel(user, id);
            }
            let snap = self
                .research
                .get(user, id)
                .ok_or_else(|| "the sweep vanished".to_string())?;
            match snap.status {
                SweepStatus::Completed => {
                    let report = snap
                        .report
                        .ok_or_else(|| "the sweep completed with no report".to_string())?;
                    return Ok(SweepOutcome {
                        candidate: report.carried_forward,
                        trials_consumed: report.trials_consumed,
                        n_sampled: report.n_sampled,
                        n_feasible: report.n_feasible,
                        plateau: report.neighbourhood_plateau,
                        study_ids: report.study_ids,
                        sample_scores: report.sample_scores,
                    });
                }
                SweepStatus::Failed => {
                    return Err(snap.error.unwrap_or_else(|| "the sweep failed".into()))
                }
                SweepStatus::Cancelled => return Err("the sweep was cancelled".into()),
                SweepStatus::Queued | SweepStatus::Running => {
                    tokio::time::sleep(SWEEP_POLL).await;
                }
            }
        }
    }

    async fn replicate_metrics(
        &self,
        user: Uuid,
        call: ReplicateCall,
        _cancelled: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<Vec<Option<f64>>, String> {
        let suite = Arc::clone(&self.suite);
        let n_windows = call.windows.len();
        let metric = call.metric;
        tokio::task::spawn_blocking(move || {
            let view = suite
                .run_study(
                    user,
                    call.experiment,
                    RunStudySpec {
                        study_id: call.study_id,
                        kind: StudyKind::WalkForward,
                        vary: VarySpec::DataWindows {
                            windows: call.windows,
                        },
                        metric,
                        question: call.question,
                        selection_rule: None,
                        null_ref: None,
                        base_params: (!call.params.is_empty()).then_some(call.params),
                        cost_scale_bp: call.cost_scale_bp,
                    },
                )
                .map_err(|e| e.to_string())?;
            if view.members.len() != n_windows {
                return Err(format!(
                    "the study returned {} members for {n_windows} windows; a misaligned pair \
                     is worse than none",
                    view.members.len()
                ));
            }
            Ok(view
                .members
                .iter()
                .map(|rid| {
                    suite
                        .run_result(user, rid)
                        .filter(|r| r.status == RunStatus::Ok)
                        .map(|r| r.metrics.value(metric))
                })
                .collect())
        })
        .await
        .map_err(|e| format!("the study task panicked: {e}"))?
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// the random arm (Set R-b RB-3.1, EV-09)
// ───────────────────────────────────────────────────────────────────────────────

/// A sweep's scores as an arm records them: exactly `budget` of them, in draw order.
///
/// A sweep that drew fewer than `budget` points (a small int or enum space is exhausted
/// before the budget is) is padded with `-inf`, the failure value: the draws it could not
/// make are trials that scored nothing, and `compare_to_random` refuses arms of unequal
/// budget, so both arms are shaped by this one function and are always the same length.
#[must_use]
pub fn matched_scores(scores: &[f64], budget: usize) -> Vec<f64> {
    let mut v: Vec<f64> = scores.iter().copied().take(budget).collect();
    v.resize(budget, f64::NEG_INFINITY);
    v
}

/// What recording a campaign's arms did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArmStatus {
    /// The campaign has no arm row (defined before EV-09 reached the driver). Nothing is
    /// recorded; Gate 3 refuses the campaign for it.
    NoArm,
    /// Both arms were already complete: this is a later generation, which is outside the
    /// matched comparison (ADR-RW-03).
    AlreadyMatched,
    /// This call recorded (the rest of) the arms.
    Recorded,
}

/// Record the campaign's two arms after its first EXPERIMENT sweep.
///
/// The agent arm is that sweep's per-sample scores; the random arm is a second sweep of the
/// **declared** space (no narrowing: ADR-RB-08), random sampler, the platform's seed for the
/// arm ([`research::random_arm::arm_seed`]), the same fixed parameters and the same budget,
/// through the same evaluator, so its Runs are counted trials under the campaign (INV-1).
/// Idempotent: a retried job records only what is missing.
///
/// # Errors
/// The database, or the random sweep failing.
#[allow(clippy::too_many_arguments)]
pub async fn record_arms(
    pg: &PgPool,
    evaluator: &dyn CampaignEvaluator,
    user: Uuid,
    campaign_id: Uuid,
    experiment: Uuid,
    base_params: &ParamMap,
    agent: &SweepOutcome,
    cancelled: &(dyn Fn() -> bool + Send + Sync),
) -> Result<ArmStatus, String> {
    let store = ArmStore::new(pg.clone());
    let Some((arm, agent_scores)) = store.load(campaign_id).await.map_err(|e| format!("reading the arm: {e}"))? else {
        return Ok(ArmStatus::NoArm);
    };
    if !agent_scores.is_empty() && arm.complete() {
        return Ok(ArmStatus::AlreadyMatched);
    }
    // The agent's side is recorded once, from the first sweep only. A retry after the agent
    // side was written but the random sweep failed finds it there and does not rewrite it.
    if agent_scores.is_empty() {
        store
            .record_scores(campaign_id, "agent", &matched_scores(&agent.sample_scores, arm.budget))
            .await
            .map_err(|e| format!("recording the agent arm: {e}"))?;
    }
    if !arm.complete() {
        let random = evaluator
            .sweep(
                user,
                SweepCall {
                    experiment,
                    narrowing: BTreeMap::new(),
                    base_params: base_params.clone(),
                    seed: arm.seed,
                    question: format!("campaign {campaign_id}: the matched random-search arm (EV-09)"),
                    sampler: SamplerKind::Random,
                    max_runs: u32::try_from(arm.budget).unwrap_or(u32::MAX),
                },
                cancelled,
            )
            .await
            .map_err(|e| format!("the random arm's sweep: {e}"))?;
        store
            .record_scores(campaign_id, "random", &matched_scores(&random.sample_scores, arm.budget))
            .await
            .map_err(|e| format!("recording the random arm: {e}"))?;
    }
    Ok(ArmStatus::Recorded)
}

// ───────────────────────────────────────────────────────────────────────────────
// the regime-conditional prior (Set R-b RB-4.5)
// ───────────────────────────────────────────────────────────────────────────────

/// One window's contribution to `regime_causal.regime_strategy_prior`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegimeObservation {
    pub regime: RegimeLabel,
    pub score: f64,
    /// The declared rule: the primary metric over the window is positive.
    pub success: bool,
}

/// Pair each window's regime with its measured score. A window with no label in force or
/// whose run failed contributes nothing (an unlabelled window is dropped, never given the
/// nearest label: nearest in that direction is the future).
#[must_use]
pub fn regime_observations(labels: &[Option<RegimeLabel>], values: &[Option<f64>]) -> Vec<RegimeObservation> {
    labels
        .iter()
        .zip(values)
        .filter_map(|(l, v)| {
            let (regime, score) = ((*l)?, (*v)?);
            score.is_finite().then_some(RegimeObservation { regime, score, success: score > 0.0 })
        })
        .collect()
}

/// Fold a regime-sliced study into the prior: each window of `windows` is labelled with the
/// filtered regime in force **at its start** (what a strategy entering it could have known),
/// and its score goes into the `(instrument, regime, strategy family)` cell.
///
/// Returns how many windows were folded. An instrument with no asset class, no configured or
/// an ambiguous scope, or no labels yet folds nothing and says so with `0`: the prior is
/// advisory and a missing label is not a reason to fail a campaign.
///
/// # Errors
/// The database.
pub async fn fold_regime_prior(
    pg: &PgPool,
    tenant: &str,
    spec: &EvaluationSpec,
    windows: &[(DateTime<Utc>, DateTime<Utc>)],
    values: &[Option<f64>],
) -> Result<usize, String> {
    use crate::knowledge::{
        labels_as_of, RegimeScopeStore, RegimeStore, ScopeResolution, LABEL_LOOKBEHIND_DAYS, RULE_MODEL_VERSION,
    };
    let (Some(first), Some(last)) = (windows.iter().map(|w| w.0).min(), windows.iter().map(|w| w.1).max()) else {
        return Ok(0);
    };
    let asset_class: Option<(String,)> = sqlx::query_as("SELECT asset_class FROM asset_lifecycle WHERE symbol = $1")
        .bind(&spec.universe_ref)
        .fetch_optional(pg)
        .await
        .map_err(|e| e.to_string())?;
    let Some((asset_class,)) = asset_class else { return Ok(0) };
    let scope = match RegimeScopeStore::new(pg.clone())
        .resolve(&asset_class, Some(&spec.universe_ref))
        .await
        .map_err(|e| e.to_string())?
    {
        ScopeResolution::Scope(s) => s,
        ScopeResolution::Ambiguous(_) | ScopeResolution::Unconfigured => return Ok(0),
    };
    let history = RegimeStore::new(pg.clone())
        .history(&scope.market_scope, RULE_MODEL_VERSION, first - chrono::Duration::days(LABEL_LOOKBEHIND_DAYS), last)
        .await
        .map_err(|e| e.to_string())?;
    let starts: Vec<DateTime<Utc>> = windows.iter().map(|w| w.0).collect();
    let obs = regime_observations(&labels_as_of(&history, &starts), values);
    let store = PriorStore::new(pg.clone());
    for o in &obs {
        store
            .update(tenant, &spec.universe_ref, o.regime.as_str(), &spec.strategy_type, o.score, o.success)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(obs.len())
}

// ───────────────────────────────────────────────────────────────────────────────
// the workers
// ───────────────────────────────────────────────────────────────────────────────

fn logic(code: &'static str, detail: impl Into<String>) -> JobError {
    JobError::logic(
        ledger::TerminalReason::IntegrityRejected,
        code,
        detail.into(),
    )
}

fn campaign_and_seq(manifest: &Value, kind: &str) -> Result<(Uuid, i64), JobError> {
    let campaign_id = manifest
        .get("campaign_id")
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| {
            logic(
                "invalid_manifest",
                format!(
                    "a {kind} job on this pool is a campaign phase and needs a campaign_id; \
                     standalone jobs of this kind are not run by this worker"
                ),
            )
        })?;
    Ok((
        campaign_id,
        manifest.get("seq").and_then(Value::as_i64).unwrap_or(0),
    ))
}

/// Performs EXPERIMENT (`JobKind::Study`).
pub struct StudyWorker {
    store: Arc<JobStore>,
    pg: PgPool,
    evaluator: Arc<dyn CampaignEvaluator>,
}

impl StudyWorker {
    #[must_use]
    pub fn new(store: Arc<JobStore>, pg: PgPool, evaluator: Arc<dyn CampaignEvaluator>) -> Self {
        Self {
            store,
            pg,
            evaluator,
        }
    }

    /// The most recent HYPOTHESIZE child's proposal, or `Null`.
    async fn latest_proposal(&self, tenant: &str, campaign_id: Uuid) -> Result<Value, JobError> {
        let state = ledger::pg::PgTrialLedger::new(self.pg.clone())
            .campaign_state_async(tenant, campaign_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("folding the campaign: {e}")))?;
        let Some(job_id) = state
            .children
            .iter()
            .rev()
            .find(|(phase, _, _)| *phase == CampaignPhase::Hypothesize)
            .map(|(_, _, id)| id.clone())
        else {
            return Ok(Value::Null);
        };
        let job = self
            .store
            .get(&job_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading {job_id}: {e}")))?;
        Ok(job
            .result
            .and_then(|r| r.get("proposal").cloned())
            .unwrap_or(Value::Null))
    }
}

#[async_trait::async_trait]
impl Worker for StudyWorker {
    fn kind(&self) -> JobKind {
        JobKind::Study
    }

    fn estimate(&self, _manifest: &Value) -> Cost {
        Cost::default()
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let (campaign_id, seq) = campaign_and_seq(manifest, "study")?;
        let me = self
            .store
            .get(&ctx.job_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading this job: {e}")))?;
        let user = me.user_id;
        let tenant = user.to_string();

        let (delta, _profile, search_space) =
            campaign_delta_and_profile(&self.pg, &tenant, campaign_id).await?;
        let spec = evaluation_spec(&search_space)?;
        let proposal = self.latest_proposal(&tenant, campaign_id).await?;
        let narrowing = narrowing_from_proposal(&proposal)
            .map_err(|why| logic("proposal_ranges_unreadable", why))?;
        let move_kind = proposal
            .get("move")
            .and_then(Value::as_str)
            .map(str::to_string);
        let incumbent = standing_incumbent(&self.pg, &tenant, campaign_id, &spec)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading the incumbent: {e}")))?;

        ctx.progress(Progress {
            pct: Some(0.05),
            stage: Some("experiment".into()),
            message: Some("preparing the experiment".into()),
        })
        .await;
        let experiment = self
            .evaluator
            .ensure_experiment(user, campaign_id, &spec, delta)
            .await
            .map_err(|e| JobError::infrastructure(format!("the experiment: {e}")))?;

        let cancel = || ctx.is_cancelled();
        let outcome = self
            .evaluator
            .sweep(
                user,
                SweepCall {
                    experiment,
                    narrowing: narrowing.clone(),
                    base_params: incumbent.clone(),
                    seed: replicate_seed(campaign_id, seq),
                    question: proposal
                        .get("hypothesis")
                        .and_then(Value::as_str)
                        .map_or_else(
                            || format!("campaign {campaign_id}: search around the incumbent"),
                            str::to_string,
                        ),
                    sampler: spec.sampler,
                    max_runs: spec.max_runs,
                },
                &cancel,
            )
            .await
            .map_err(|e| JobError::infrastructure(format!("the sweep: {e}")))?;

        // EV-09: the first generation also records the two matched arms, so a campaign
        // always has the random-search baseline its Gate 3 asks for.
        let arms = record_arms(&self.pg, self.evaluator.as_ref(), user, campaign_id, experiment, &incumbent, &outcome, &cancel)
            .await
            .map_err(|e| JobError::infrastructure(format!("the random arm: {e}")))?;

        Ok(JobOutput {
            summary: Some(format!(
                "swept {} runs ({} feasible); candidate {}",
                outcome.n_sampled,
                outcome.n_feasible,
                if outcome.candidate.is_some() {
                    "carried forward"
                } else {
                    "none survived"
                }
            )),
            result: json!({
                "campaign_id": campaign_id,
                "experiment": experiment,
                "candidate_params": outcome.candidate,
                "incumbent_params": incumbent,
                "move": move_kind,
                // See the module's ADR-C-04: ranges are applied, graph edits are
                // not, and the record says so.
                "structural_move_applied": false,
                "narrowed": narrowing.keys().collect::<Vec<_>>(),
                "sweep": outcome,
                "arms": arms,
            }),
            ..Default::default()
        })
    }
}

/// Performs BASELINE (`JobKind::Backtest` on a campaign's child).
///
/// The benchmark is the incumbent as declared at DEFINE, measured once over the
/// whole research window through the same engine every later comparison uses, so
/// that "better than baseline" is a statement about one engine.
pub struct BaselineWorker {
    store: Arc<JobStore>,
    pg: PgPool,
    evaluator: Arc<dyn CampaignEvaluator>,
}

impl BaselineWorker {
    #[must_use]
    pub fn new(store: Arc<JobStore>, pg: PgPool, evaluator: Arc<dyn CampaignEvaluator>) -> Self {
        Self {
            store,
            pg,
            evaluator,
        }
    }
}

#[async_trait::async_trait]
impl Worker for BaselineWorker {
    fn kind(&self) -> JobKind {
        JobKind::Backtest
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let (campaign_id, seq) = campaign_and_seq(manifest, "backtest")?;
        let me = self
            .store
            .get(&ctx.job_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading this job: {e}")))?;
        let user = me.user_id;
        let tenant = user.to_string();
        let (delta, _profile, search_space) =
            campaign_delta_and_profile(&self.pg, &tenant, campaign_id).await?;
        let spec = evaluation_spec(&search_space)?;
        let experiment = self
            .evaluator
            .ensure_experiment(user, campaign_id, &spec, delta)
            .await
            .map_err(|e| JobError::infrastructure(format!("the experiment: {e}")))?;
        let cancel = || ctx.is_cancelled();
        let metric = spec.objective.primary;
        let values = self
            .evaluator
            .replicate_metrics(
                user,
                ReplicateCall {
                    experiment,
                    study_id: format!("base-{}-{seq}", campaign_id.simple()),
                    params: spec.incumbent_params.clone(),
                    windows: vec![(spec.research.start, spec.research.end)],
                    metric,
                    question: format!("campaign {campaign_id} baseline: the declared incumbent"),
                    cost_scale_bp: 0,
                },
                &cancel,
            )
            .await
            .map_err(|e| JobError::infrastructure(format!("the baseline run: {e}")))?;
        let value = values.first().copied().flatten();
        Ok(JobOutput {
            summary: Some(format!(
                "baseline {metric:?}: {}",
                value.map_or("run failed".to_string(), |v| format!("{v:.4}"))
            )),
            result: json!({
                "campaign_id": campaign_id,
                "experiment": experiment,
                "metric": format!("{metric:?}"),
                "baseline_value": value,
                "params": spec.incumbent_params,
            }),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn good() -> Value {
        json!({
            "evaluation": {
                "strategy_ref": "ema_cross",
                "strategy_type": "trend",
                "universe_ref": "BTC-USD",
                "research": { "start": "2024-01-01T00:00:00Z", "end": "2024-09-01T00:00:00Z" },
                "holdout":  { "start": "2024-09-01T00:00:00Z", "end": "2024-12-01T00:00:00Z" },
                "objective": { "primary": "sharpe" }
            }
        })
    }

    /// A campaign that does not say what it searches fails with the fix named,
    /// not with a campaign that runs against nothing.
    #[test]
    fn a_campaign_without_an_evaluation_block_is_refused_with_the_fix() {
        let err = EvaluationSpec::from_search_space(&json!({})).unwrap_err();
        assert!(err.contains("evaluation"), "{err}");
        assert!(err.contains("redefined"), "{err}");
        let job = evaluation_spec(&json!({})).unwrap_err();
        assert_eq!(job.code, "no_evaluation_spec");
    }

    #[test]
    fn a_complete_block_parses_with_declared_defaults() {
        let spec = EvaluationSpec::from_search_space(&good()).unwrap();
        assert_eq!(spec.strategy_ref, "ema_cross");
        assert_eq!(spec.max_runs, 24);
        assert_eq!(spec.replicates, 29);
        assert!(spec.incumbent_params.is_empty());
        assert_eq!(spec.research.start, Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap());
    }

    /// The holdout is sealed: a research window that reaches into it is refused
    /// at DEFINE-read time, before the engine's own check.
    #[test]
    fn a_research_window_that_overlaps_the_holdout_is_refused() {
        let mut v = good();
        v["evaluation"]["research"]["end"] = json!("2024-10-15T00:00:00Z");
        let err = EvaluationSpec::from_search_space(&v).unwrap_err();
        assert!(err.contains("holdout"), "{err}");
    }

    #[test]
    fn bad_declarations_are_named() {
        let mut v = good();
        v["evaluation"]["max_runs"] = json!(0);
        assert!(EvaluationSpec::from_search_space(&v).unwrap_err().contains("max_runs"));
        let mut v = good();
        v["evaluation"]["rope_scale"] = json!(-1.0);
        assert!(EvaluationSpec::from_search_space(&v).unwrap_err().contains("rope_scale"));
        let mut v = good();
        v["evaluation"]["research"]["end"] = json!("2023-01-01T00:00:00Z");
        assert!(EvaluationSpec::from_search_space(&v).unwrap_err().contains("research"));
    }

    /// The seed is a function of position alone, and it moves with the position.
    #[test]
    fn the_replicate_seed_is_deterministic_and_position_dependent() {
        let c = Uuid::new_v4();
        assert_eq!(replicate_seed(c, 7), replicate_seed(c, 7));
        assert_ne!(replicate_seed(c, 7), replicate_seed(c, 8));
        assert_ne!(replicate_seed(c, 7), replicate_seed(Uuid::new_v4(), 7));
    }

    /// The budget rule finds the suite's trials by this exact name.
    #[test]
    fn the_experiment_name_is_the_one_the_budget_rule_counts() {
        let c = Uuid::new_v4();
        assert_eq!(experiment_name(c), format!("campaign-{c}"));
    }

    /// Both arms are always the same length, whatever the sampler could draw.
    #[test]
    fn an_arm_is_exactly_its_budget_long_with_undrawn_trials_scored_as_failures() {
        assert_eq!(matched_scores(&[1.0, 2.0], 4), vec![1.0, 2.0, f64::NEG_INFINITY, f64::NEG_INFINITY]);
        assert_eq!(matched_scores(&[1.0, 2.0, 3.0], 2), vec![1.0, 2.0], "extra draws are not scored");
        assert_eq!(matched_scores(&[], 3).len(), 3);
    }

    /// An unlabelled window or a failed run adds nothing to the prior.
    #[test]
    fn only_a_labelled_window_with_a_finite_score_reaches_the_prior() {
        let labels = [Some(RegimeLabel::LowVol), None, Some(RegimeLabel::HighVol), Some(RegimeLabel::HighVol)];
        let values = [Some(0.4), Some(9.0), None, Some(-0.1)];
        let obs = regime_observations(&labels, &values);
        assert_eq!(obs.len(), 2);
        assert_eq!((obs[0].regime, obs[0].success), (RegimeLabel::LowVol, true));
        assert_eq!((obs[1].regime, obs[1].success), (RegimeLabel::HighVol, false));
        assert!(regime_observations(&[Some(RegimeLabel::MidVol)], &[Some(f64::NAN)]).is_empty());
    }

    #[test]
    fn proposed_ranges_become_a_narrowing() {
        let proposal = json!({
            "move": "change_sizing",
            "hypothesis": "narrower fast window",
            "ranges": [
                { "name": "fast", "kind": { "kind": "int", "min": 5, "max": 20, "step": 1 } },
                { "name": "thresh", "kind": { "kind": "float", "min": 0.1, "max": 0.9, "log": false } },
                { "name": "mode", "kind": { "kind": "enum", "choices": ["a", "b"] } }
            ]
        });
        let n = narrowing_from_proposal(&proposal).unwrap();
        assert_eq!(n["fast"].min, Some(5.0));
        assert_eq!(n["fast"].max, Some(20.0));
        assert_eq!(n["thresh"].max, Some(0.9));
        assert_eq!(n["mode"].choices.as_deref(), Some(&["a".to_string(), "b".to_string()][..]));
        // No proposal, or one without ranges, searches the declared space.
        assert!(narrowing_from_proposal(&Value::Null).unwrap().is_empty());
        assert!(narrowing_from_proposal(&json!({ "move": "simplify" })).unwrap().is_empty());
        // A malformed range is an error, not a skipped dimension.
        assert!(narrowing_from_proposal(&json!({ "ranges": [{ "name": "x" }] })).is_err());
    }

    /// A standalone backtest job on the Backtest pool is not a campaign baseline,
    /// and the worker says so rather than guessing.
    #[test]
    fn a_manifest_without_a_campaign_is_not_this_workers() {
        let err = campaign_and_seq(&json!({}), "backtest").unwrap_err();
        assert_eq!(err.code, "invalid_manifest");
        let (c, seq) = campaign_and_seq(
            &json!({ "campaign_id": "3f9a7d2e-1111-4222-8333-444455556666", "seq": 4 }),
            "study",
        )
        .unwrap();
        assert_eq!(seq, 4);
        assert_eq!(c.to_string(), "3f9a7d2e-1111-4222-8333-444455556666");
    }
}
