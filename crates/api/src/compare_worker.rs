//! The COMPARE phase's worker, and the campaign's two remaining terminal rules
//! (SPEC §10, §11.5; checklist 2.1, 2.12; ROADMAP Stage 2; OPEN-TASKS C1.1).
//!
//! # What was built and not reached
//!
//! `backtest::stats::compare::ComparisonPlan::judge` — paired replicates,
//! anytime-valid confidence sequences, the ROPE, the win-rate rule — was complete,
//! tested and sealed, and had **no production caller**: `campaign_driver` mapped
//! COMPARE to `JobKind::EvalTask`, no worker was registered for that kind, and a
//! campaign reaching the phase failed with `no_worker`. Because nothing judged a
//! candidate against an incumbent, `DIMINISHING_RETURNS` and `CONVERGED` could not
//! be computed either; the driver reported them `NotComputable`.
//!
//! This is the missing worker. It measures the proposed candidate and the standing
//! incumbent on the **same replicates**, calls `judge`, and writes the outcome to
//! `mlops.decision` (kind `promote` or `reject`, policy [`COMPARE_POLICY`]) with the
//! campaign's id. The decision log is the record the stopping rules read, because
//! it is already append-only, hash-chained and tenant-scoped, and a second table
//! recording the same fact would be a second answer to the same question.
//!
//! # The replicate grid is windows only (ADR-C-02)
//!
//! §11.5's nuisance product is window × cost perturbation × fill seed. The study
//! engine perturbs cost through *named* cost models (`VarySpec::CostLadder`) and
//! the platform has no named −20 % / +20 % pair, so this worker varies the
//! **window** and pins the other two axes: every replicate carries `cost_bp = 0`
//! and a seed derived from the campaign, and the record says so
//! (`replicate_axes: ["window"]`). A narrower nuisance set makes the paired
//! differences *less* variable than the protocol intends, so the verdict is
//! optimistic about separation; it is stated rather than hidden, and the cost axis
//! is a one-line extension once the cost models exist.
//!
//! # The two stopping rules (ADR-C-03)
//!
//! Both are pure functions of the decision history so that they can be tested
//! without a database and cannot drift from what the worker writes.
//!
//! * **`DIMINISHING_RETURNS`** — SPEC §10: *stop when the posterior probability
//!   that any remaining candidate exceeds the incumbent by `delta_practical` falls
//!   below 0.05, estimated over the last 20 % of trials.* Each comparison yields a
//!   Bernoulli outcome, "could this candidate exceed the incumbent by delta":
//!   false exactly when the confidence sequence's **upper** bound on the paired
//!   difference lies below delta (verdict `reject`), true otherwise — including
//!   `practically_equivalent`, because a sequence that straddles delta has not
//!   excluded it. The posterior over that rate is Beta(½ + k, ½ + n − k) (the
//!   Jeffreys prior); the predictive probability that the *next* candidate could
//!   exceed delta is its mean. The rule fires when that mean is below
//!   [`DR_THRESHOLD`] over the last [`DR_WINDOW_FRACTION`] of the comparisons
//!   (never fewer than [`DR_MIN_COMPARISONS`]). *Units:* §10 says "the last 20 %
//!   of trials". A comparison is one candidate looked at, and a campaign cycle
//!   spends ~60-100 *counted* trials on one of them, so a window measured in
//!   counted trials would hold one or two comparisons and the rule could never
//!   reach ten; measured in candidates it can fire after ten dry cycles.
//!   *Rejected alternatives:* the window in counted trials (never fires), and an
//!   upper credible bound below 0.05 (needs about sixty clean comparisons, so it
//!   would only fire after the budget was gone).
//! * **`CONVERGED`** — the incumbent has been unchanged for
//!   `max(20, 0.25 · trials)` trials **and** the diminishing-returns test agrees.
//!   "Unchanged" is measured from the last `promote` decision (or from trial zero
//!   if there has never been one).

#![allow(clippy::result_large_err)]

use std::sync::Arc;

use backtest::run::{MetricKind, ParamMap};
use backtest::stats::compare::{
    replicate_grid, ComparisonFlags, ComparisonPlan, Measurement, RopeScale, Verdict,
    DEFAULT_ALPHA, MIN_REPLICATES, TARGET_REPLICATES,
};
use chrono::{DateTime, Duration, Utc};
use jobs::store::JobStore;
use jobs::{Cost, JobContext, JobError, JobKind, JobOutput, Progress, Worker};
use ledger::phase::CampaignPhase;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use super::study_worker::{
    campaign_delta_and_profile, evaluation_spec, replicate_seed, CampaignEvaluator, EvaluationSpec,
    ReplicateCall, Window,
};

/// The policy under which a campaign's comparisons are recorded in
/// `mlops.decision`, and its version. Bumped when what a comparison measures
/// changes in a way that would change what it decides.
pub const COMPARE_POLICY: &str = "campaign_compare";
pub const COMPARE_POLICY_VERSION: i32 = 1;

/// SPEC §10's diminishing-returns threshold.
pub const DR_THRESHOLD: f64 = 0.05;
/// The fraction of the comparisons made that the rule looks back over (see the
/// module's ADR-C-03 for why candidates and not counted trials).
pub const DR_WINDOW_FRACTION: f64 = 0.20;
/// Fewer comparisons than this and the posterior is the prior. The rule reports
/// "not enough evidence" rather than firing on three observations.
pub const DR_MIN_COMPARISONS: usize = 10;
/// `CONVERGED`'s floor on how long the incumbent must stand.
pub const CONVERGED_MIN_TRIALS: i64 = 20;
/// ... and the fraction of all trials counted it must have stood for.
pub const CONVERGED_FRACTION: f64 = 0.25;

/// The ROPE half-width as a multiple of the campaign's `delta_practical`, when
/// the campaign's evaluation block does not declare one. Declared at DEFINE
/// through `search_space.evaluation.rope_scale`, never chosen after the data.
pub const DEFAULT_SCALE_MULTIPLE: f64 = 20.0;

// ───────────────────────────────────────────────────────────────────────────────
// what a comparison leaves behind
// ───────────────────────────────────────────────────────────────────────────────

/// One recorded comparison, as the stopping rules read it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CompareRecord {
    /// Trials the campaign had counted when this comparison was made.
    pub trials_at: i64,
    /// Whether the confidence sequence failed to exclude "the candidate exceeds
    /// the incumbent by delta".
    pub could_exceed: bool,
    /// Whether the candidate was promoted to incumbent.
    pub promoted: bool,
}

/// Why a campaign should or should not stop on the evidence so far.
#[derive(Debug, Clone, PartialEq)]
pub enum StopCheck {
    Continue,
    /// Not enough comparisons in the window to say anything. Named so a reader
    /// can tell "the rule looked and abstained" from "the rule did not look".
    NotEnoughEvidence {
        in_window: usize,
        needed: usize,
    },
    End(CampaignPhase, String),
}

/// The Jeffreys posterior over "a candidate could exceed the incumbent by delta",
/// over the last [`DR_WINDOW_FRACTION`] of the comparisons (at least
/// [`DR_MIN_COMPARISONS`], when that many exist).
///
/// `history` is oldest first. Returns `(predictive_probability,
/// comparisons_in_window)`.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn exceed_posterior(history: &[CompareRecord]) -> (f64, usize) {
    let lookback = ((history.len() as f64) * DR_WINDOW_FRACTION).ceil() as usize;
    let take = lookback.max(DR_MIN_COMPARISONS).min(history.len());
    let window = &history[history.len() - take..];
    let n = window.len();
    let k = window.iter().filter(|r| r.could_exceed).count();
    ((0.5 + k as f64) / (1.0 + n as f64), n)
}

/// Evaluate `DIMINISHING_RETURNS` then `CONVERGED` against the decision history.
///
/// `counted` is the number of trials the campaign has counted now. Order matters
/// only for the report: `CONVERGED` implies the diminishing-returns test agrees,
/// so when both hold the campaign ends as `converged`, the stronger statement.
#[must_use]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub fn stop_check(history: &[CompareRecord], counted: i64) -> StopCheck {
    let (posterior, n) = exceed_posterior(history);
    if n < DR_MIN_COMPARISONS {
        return StopCheck::NotEnoughEvidence {
            in_window: n,
            needed: DR_MIN_COMPARISONS,
        };
    }
    if posterior >= DR_THRESHOLD {
        return StopCheck::Continue;
    }
    // The diminishing-returns test agrees. CONVERGED additionally needs the
    // incumbent to have stood long enough.
    let last_promotion = history
        .iter()
        .filter(|r| r.promoted)
        .map(|r| r.trials_at)
        .max()
        .unwrap_or(0);
    let unchanged_for = counted - last_promotion;
    let needed = CONVERGED_MIN_TRIALS.max(((counted as f64) * CONVERGED_FRACTION).ceil() as i64);
    if unchanged_for >= needed {
        return StopCheck::End(
            CampaignPhase::Converged,
            format!(
                "the incumbent has stood for {unchanged_for} trials (needed {needed}) and the \
                 posterior that a candidate could exceed it by delta is {posterior:.3} over \
                 {n} comparisons, below {DR_THRESHOLD}"
            ),
        );
    }
    StopCheck::End(
        CampaignPhase::DiminishingReturns,
        format!(
            "the posterior that a candidate could exceed the incumbent by delta is \
             {posterior:.3} over the last {n} comparisons, below {DR_THRESHOLD}"
        ),
    )
}

/// Trials counted against a campaign.
///
/// Three things are the campaign's: trials registered with its `campaign_id`
/// (the driver's own child jobs, since the job store started stamping it), and
/// the trials the Backtest Suite registered while running its experiment, which
/// the suite files under `experiment_id` and cannot tag with a campaign. The
/// experiment is named `campaign-<id>` by [`crate::study_worker`] so that this
/// query can find it; the second form `campaign:<id>:<phase>` is the driver's
/// own convention for counted children.
///
/// # Errors
/// Backend failures.
pub async fn campaign_trials_counted(
    pg: &PgPool,
    tenant: &str,
    campaign_id: Uuid,
) -> Result<i64, sqlx::Error> {
    let mut tx = ledger::pg::tenant_tx(pg, tenant)
        .await
        .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
    let (n,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM mlops.trial
          WHERE campaign_id = $1 OR experiment_id = $2 OR experiment_id LIKE $3",
    )
    .bind(campaign_id)
    .bind(format!("campaign-{campaign_id}"))
    .bind(format!("campaign:{campaign_id}:%"))
    .fetch_one(&mut *tx)
    .await?;
    let _ = tx.commit().await;
    Ok(n)
}

/// A campaign's comparison history and its standing incumbent, oldest first.
///
/// # Errors
/// Backend failures.
pub async fn compare_history(
    pg: &PgPool,
    tenant: &str,
    campaign_id: Uuid,
) -> Result<Vec<(CompareRecord, Value)>, sqlx::Error> {
    let mut tx = ledger::pg::tenant_tx(pg, tenant)
        .await
        .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
    let rows: Vec<(Value,)> = sqlx::query_as(
        "SELECT chosen FROM mlops.decision
          WHERE campaign_id = $1 AND policy_id = $2
          ORDER BY seq",
    )
    .bind(campaign_id)
    .bind(COMPARE_POLICY)
    .fetch_all(&mut *tx)
    .await?;
    let _ = tx.commit().await;
    Ok(rows
        .into_iter()
        .filter_map(|(chosen,)| parse_record(&chosen).map(|r| (r, chosen)))
        .collect())
}

fn parse_record(chosen: &Value) -> Option<CompareRecord> {
    Some(CompareRecord {
        trials_at: chosen.get("trials_at")?.as_i64()?,
        could_exceed: chosen.get("could_exceed")?.as_bool()?,
        promoted: chosen.get("promoted")?.as_bool()?,
    })
}

/// The incumbent's parameters: the last promoted candidate, else the campaign's
/// declared starting point.
///
/// # Errors
/// Backend failures.
pub async fn standing_incumbent(
    pg: &PgPool,
    tenant: &str,
    campaign_id: Uuid,
    spec: &EvaluationSpec,
) -> Result<ParamMap, sqlx::Error> {
    let history = compare_history(pg, tenant, campaign_id).await?;
    Ok(history
        .iter()
        .rev()
        .find(|(r, _)| r.promoted)
        .and_then(|(_, chosen)| chosen.get("candidate_params"))
        .and_then(|p| serde_json::from_value::<ParamMap>(p.clone()).ok())
        .unwrap_or_else(|| spec.incumbent_params.clone()))
}

// ───────────────────────────────────────────────────────────────────────────────
// replicates
// ───────────────────────────────────────────────────────────────────────────────

/// The windows a comparison is replicated over.
///
/// `k` windows, each covering half of the research slice, with starts spaced
/// evenly so the first begins at the slice's start and the last ends at its end.
/// A pure function of its arguments: a third party can rebuild the grid, and the
/// worker has no argument through which a friendlier one could be chosen.
#[must_use]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub fn replicate_windows(research: &Window, k: usize) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let total = (research.end - research.start).num_seconds();
    if k == 0 || total <= 0 {
        return Vec::new();
    }
    let len = (total / 2).max(1);
    let slack = total - len;
    (0..k)
        .map(|i| {
            let offset = if k == 1 {
                0
            } else {
                (slack as f64 * i as f64 / (k - 1) as f64).round() as i64
            };
            let start = research.start + Duration::seconds(offset);
            (start, start + Duration::seconds(len))
        })
        .collect()
}

/// Pair two sides' per-window metrics, dropping any window either side failed on.
///
/// Dropping is the only honest option: a window where one side has a value and
/// the other does not carries no paired difference, and imputing one would be
/// inventing a replicate. The count dropped is returned so the record can say.
#[must_use]
pub fn pair_measurements(
    candidate: &[Option<f64>],
    incumbent: &[Option<f64>],
    base_seed: u64,
) -> (Vec<Measurement>, Vec<Measurement>, usize) {
    let grid = replicate_grid(candidate.len().max(incumbent.len()), base_seed);
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut dropped = 0;
    for (i, replicate) in grid.iter().enumerate() {
        // Windows only vary; the cost axis stays pinned at the modelled cost.
        let replicate = backtest::stats::compare::Replicate {
            cost_bp: 0,
            ..*replicate
        };
        match (
            candidate.get(i).copied().flatten(),
            incumbent.get(i).copied().flatten(),
        ) {
            (Some(x), Some(y)) if x.is_finite() && y.is_finite() => {
                a.push(Measurement {
                    replicate,
                    value: x,
                });
                b.push(Measurement {
                    replicate,
                    value: y,
                });
            }
            _ => dropped += 1,
        }
    }
    (a, b, dropped)
}

/// Everything a comparison concluded, in the shape that is written to the
/// decision log and read back by [`parse_record`].
#[derive(Debug, Clone, Serialize)]
pub struct CompareOutcome {
    pub verdict: &'static str,
    pub promoted: bool,
    pub could_exceed: bool,
    pub achieved_k: usize,
    pub dropped_windows: usize,
    pub p_a_better: f64,
    pub p_a_better_lower: f64,
    pub mean_difference: f64,
    pub interval: [f64; 2],
    pub flags: Vec<&'static str>,
}

/// Judge a candidate against an incumbent on paired replicates.
///
/// The one call site of [`ComparisonPlan::judge`] in production. Everything that
/// qualifies the verdict travels in `plan`, and `judge` refuses to work without
/// it.
///
/// # Errors
/// The protocol's own refusal, stated: too few replicates, or a paired difference
/// outside the declared scale.
pub fn judge_pair(
    plan: &ComparisonPlan,
    candidate: &[Measurement],
    incumbent: &[Measurement],
    dropped: usize,
) -> Result<CompareOutcome, String> {
    let c = plan
        .judge(candidate, incumbent)
        .map_err(|e| e.to_string())?;
    let verdict = c.verdict();
    let interval = c.interval();
    Ok(CompareOutcome {
        verdict: verdict.as_str(),
        promoted: verdict == Verdict::Promote,
        // `reject` is the one verdict that excludes "exceeds delta"; everything
        // else leaves it open.
        could_exceed: verdict != Verdict::Reject,
        achieved_k: c.achieved_k(),
        dropped_windows: dropped,
        p_a_better: c.p_a_better(),
        p_a_better_lower: c.p_a_better_lower(),
        mean_difference: c.mean_difference(),
        interval: [interval.lower, interval.upper],
        flags: c.flags().raised(),
    })
}

// ───────────────────────────────────────────────────────────────────────────────
// the worker
// ───────────────────────────────────────────────────────────────────────────────

/// Performs the COMPARE phase (`JobKind::EvalTask`).
pub struct CompareWorker {
    store: Arc<JobStore>,
    pg: PgPool,
    evaluator: Arc<dyn CampaignEvaluator>,
}

fn logic(code: &'static str, detail: impl Into<String>) -> JobError {
    JobError::logic(
        ledger::TerminalReason::IntegrityRejected,
        code,
        detail.into(),
    )
}

impl CompareWorker {
    #[must_use]
    pub fn new(store: Arc<JobStore>, pg: PgPool, evaluator: Arc<dyn CampaignEvaluator>) -> Self {
        Self {
            store,
            pg,
            evaluator,
        }
    }

    /// The EXPERIMENT child this COMPARE follows: its result names the candidate.
    async fn latest_experiment_result(
        &self,
        tenant: &str,
        campaign_id: Uuid,
    ) -> Result<Value, JobError> {
        let state = ledger::pg::PgTrialLedger::new(self.pg.clone())
            .campaign_state_async(tenant, campaign_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("folding the campaign: {e}")))?;
        let job_id = state
            .children
            .iter()
            .rev()
            .find(|(phase, _, _)| *phase == CampaignPhase::Experiment)
            .map(|(_, _, id)| id.clone())
            .ok_or_else(|| {
                logic(
                    "no_experiment_to_compare",
                    "COMPARE ran before any EXPERIMENT child was recorded",
                )
            })?;
        let job = self
            .store
            .get(&job_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading {job_id}: {e}")))?;
        Ok(job.result.unwrap_or(Value::Null))
    }
}

#[async_trait::async_trait]
impl Worker for CompareWorker {
    fn kind(&self) -> JobKind {
        JobKind::EvalTask
    }

    fn estimate(&self, _manifest: &Value) -> Cost {
        Cost::default()
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let campaign_id: Uuid = manifest
            .get("campaign_id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| logic("invalid_manifest", "an eval_task job needs a campaign_id"))?;
        let seq = manifest.get("seq").and_then(Value::as_i64).unwrap_or(0);
        let me = self
            .store
            .get(&ctx.job_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading this job: {e}")))?;
        let user = me.user_id;
        let tenant = user.to_string();

        let (delta, profile_id, search_space) =
            campaign_delta_and_profile(&self.pg, &tenant, campaign_id).await?;
        let spec = evaluation_spec(&search_space)?;
        let experiment_result = self.latest_experiment_result(&tenant, campaign_id).await?;
        let experiment: Option<Uuid> = experiment_result
            .get("experiment")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok());
        let candidate_params: Option<ParamMap> = experiment_result
            .get("candidate_params")
            .filter(|v| !v.is_null())
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let counted = campaign_trials_counted(&self.pg, &tenant, campaign_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("counting trials: {e}")))?;
        let incumbent_params = standing_incumbent(&self.pg, &tenant, campaign_id, &spec)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading the incumbent: {e}")))?;

        ctx.progress(Progress {
            pct: Some(0.1),
            stage: Some("compare".into()),
            message: None,
        })
        .await;

        // A sweep that found no feasible candidate is an answer, not a failure:
        // recording it as a rejection is what lets the diminishing-returns rule
        // see that the search is no longer producing anything.
        // The candidate's per-window scores, kept for the regime prior once the decision is
        // logged (a retry before that point must not fold the same windows twice).
        let mut slices: Option<(Vec<(DateTime<Utc>, DateTime<Utc>)>, Vec<Option<f64>>)> = None;
        let (outcome, why_none): (Option<CompareOutcome>, Option<&str>) =
            match (experiment, &candidate_params) {
                (Some(experiment), Some(candidate_params)) => {
                    let seed = replicate_seed(campaign_id, seq);
                    let k = spec.replicates.clamp(MIN_REPLICATES, TARGET_REPLICATES);
                    let windows = replicate_windows(&spec.research, k);
                    let metric: MetricKind = spec.objective.primary;
                    let cancel = || ctx.is_cancelled();
                    let cand = self
                        .evaluator
                        .replicate_metrics(
                            user,
                            ReplicateCall {
                                experiment,
                                study_id: format!("cmp-{}-{seq}-cand", campaign_id.simple()),
                                params: candidate_params.clone(),
                                windows: windows.clone(),
                                metric,
                                question: format!(
                                    "campaign {campaign_id} compare: candidate across {k} windows"
                                ),
                                cost_scale_bp: 0,
                            },
                            &cancel,
                        )
                        .await
                        .map_err(|e| {
                            JobError::infrastructure(format!("candidate replicates: {e}"))
                        })?;
                    slices = Some((windows.clone(), cand.clone()));
                    let inc = self
                        .evaluator
                        .replicate_metrics(
                            user,
                            ReplicateCall {
                                experiment,
                                study_id: format!("cmp-{}-{seq}-inc", campaign_id.simple()),
                                params: incumbent_params.clone(),
                                windows,
                                metric,
                                question: format!(
                                    "campaign {campaign_id} compare: incumbent across {k} windows"
                                ),
                                cost_scale_bp: 0,
                            },
                            &cancel,
                        )
                        .await
                        .map_err(|e| {
                            JobError::infrastructure(format!("incumbent replicates: {e}"))
                        })?;
                    let (a, b, dropped) = pair_measurements(&cand, &inc, seed);
                    let scale =
                        RopeScale::new(spec.rope_scale.unwrap_or(DEFAULT_SCALE_MULTIPLE * delta))
                            .map_err(|e| logic("bad_rope_scale", e.to_string()))?;
                    let n_eff = ledger::pg::PgTrialLedger::new(self.pg.clone())
                        .n_eff_async(&tenant)
                        .await
                        .map_err(|e| JobError::infrastructure(format!("reading N_eff: {e}")))?
                        .value();
                    let plan = ComparisonPlan {
                        profile_id: profile_id.clone(),
                        n_eff,
                        delta_practical: delta,
                        scale,
                        trials_a: counted,
                        trials_b: counted,
                        k_max: a.len(),
                        flags: ComparisonFlags::default(),
                        alpha: DEFAULT_ALPHA,
                    };
                    let outcome = judge_pair(&plan, &a, &b, dropped).map_err(|why| {
                        // Refused, not recorded: a comparison the protocol will
                        // not make says nothing about the candidate, and filing
                        // it as a rejection would bias the stopping rule toward
                        // stopping.
                        logic("comparison_refused", why)
                    })?;
                    (Some(outcome), None)
                }
                (None, _) => (None, Some("the experiment child recorded no experiment")),
                (_, None) => (None, Some("no_feasible_candidate")),
            };

        let promoted = outcome.as_ref().is_some_and(|o| o.promoted);
        let could_exceed = outcome.as_ref().is_some_and(|o| o.could_exceed);
        let chosen = json!({
            "verdict": outcome.as_ref().map_or("no_candidate", |o| o.verdict),
            "promoted": promoted,
            "could_exceed": could_exceed,
            "trials_at": counted,
            "candidate_params": candidate_params,
            "incumbent_params": incumbent_params,
            "outcome": outcome,
            "reason": why_none,
            "replicate_axes": ["window"],
            "delta_practical": delta,
            "profile_id": profile_id,
        });
        let context_hash = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(
                serde_json::to_vec(&json!([campaign_id, seq, experiment_result]))
                    .unwrap_or_default()
            ))
        );
        let decision = ledger::Decision {
            kind: if promoted {
                ledger::DecisionKind::Promote
            } else {
                ledger::DecisionKind::Reject
            },
            context_hash,
            context_uri: Some(format!("job:{}", ctx.job_id)),
            candidate_set: vec![
                json!({ "name": "candidate", "params": candidate_params }),
                json!({ "name": "incumbent", "params": incumbent_params }),
            ],
            chosen: chosen.clone(),
            // The protocol is deterministic given the measurements: the
            // probability that this policy makes this choice is one, and saying
            // so is true.
            propensity: Some(1.0),
            exploration_flag: false,
            decision_tier: ledger::DecisionTier::Rule,
            rationale: Some(format!(
                "paired comparison on {} windows: {}",
                outcome.as_ref().map_or(0, |o| o.achieved_k),
                outcome.as_ref().map_or("no candidate", |o| o.verdict),
            )),
        };
        let dctx = ledger::DispatchContext::agent(
            &tenant,
            &ctx.job_id,
            COMPARE_POLICY,
            COMPARE_POLICY_VERSION,
            delta,
        )
        .with_campaign(campaign_id);
        ledger::pg::PgTrialLedger::new(self.pg.clone())
            .log_decision_async(&dctx, &decision)
            .await
            .map_err(|e| JobError::infrastructure(format!("logging the comparison: {e}")))?;

        // The same windows are a regime-sliced study of the candidate: fold each window's
        // score into the regime-conditional prior (RB-4.5). Advisory, so a failure is logged
        // and does not fail a comparison that has already been recorded.
        if let Some((windows, values)) = slices {
            match super::study_worker::fold_regime_prior(
                &self.pg, &tenant, &spec, &windows, &values,
            )
            .await
            {
                Ok(n) => {
                    tracing::info!(campaign = %campaign_id, windows = n, "regime prior updated")
                }
                Err(e) => {
                    tracing::warn!(campaign = %campaign_id, error = %e, "the regime prior was not updated")
                }
            }
        }

        Ok(JobOutput {
            summary: Some(format!(
                "candidate vs incumbent: {}{}",
                chosen["verdict"].as_str().unwrap_or("?"),
                if promoted { " (promoted)" } else { "" }
            )),
            result: json!({
                "campaign_id": campaign_id,
                "verdict": chosen["verdict"],
                "promoted": promoted,
                "could_exceed": could_exceed,
                "trials_at": counted,
                "outcome": outcome,
            }),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn rec(trials_at: i64, could_exceed: bool, promoted: bool) -> CompareRecord {
        CompareRecord {
            trials_at,
            could_exceed,
            promoted,
        }
    }

    fn plan(delta: f64, scale: f64, k_max: usize) -> ComparisonPlan {
        ComparisonPlan {
            profile_id: "strict_v1".into(),
            n_eff: 10.0,
            delta_practical: delta,
            scale: RopeScale::new(scale).unwrap(),
            trials_a: 50,
            trials_b: 50,
            k_max,
            flags: ComparisonFlags::default(),
            alpha: DEFAULT_ALPHA,
        }
    }

    fn series(values: &[f64]) -> Vec<Option<f64>> {
        values.iter().map(|v| Some(*v)).collect()
    }

    // ---- the stopping rules --------------------------------------------------

    /// The rule abstains on a short history instead of firing on the prior.
    #[test]
    fn diminishing_returns_abstains_until_there_is_a_window_of_evidence() {
        let history: Vec<_> = (0..5).map(|i| rec(i * 10, false, false)).collect();
        match stop_check(&history, 50) {
            StopCheck::NotEnoughEvidence { in_window, needed } => {
                assert!(in_window < needed);
                assert_eq!(needed, DR_MIN_COMPARISONS);
            }
            other => panic!("{other:?}"),
        }
    }

    /// Ten comparisons in the window, none of which could exceed delta: the
    /// Jeffreys predictive is 0.5/11 = 0.045 < 0.05, so the campaign ends.
    #[test]
    fn a_clean_window_ends_the_campaign_as_diminishing_returns() {
        // The incumbent was last promoted at trial 80 of 100: 20 < max(20, 25)
        // trials ago, so this is not CONVERGED, and the promotion is the
        // eleventh-most-recent comparison, outside the window of ten.
        let mut history: Vec<_> = (0..10).map(|i| rec(90 + i, false, false)).collect();
        history.insert(0, rec(80, true, true));
        match stop_check(&history, 100) {
            StopCheck::End(CampaignPhase::DiminishingReturns, why) => {
                assert!(why.contains("below 0.05"), "{why}")
            }
            other => panic!("{other:?}"),
        }
    }

    /// One candidate in the window that could still exceed delta keeps it going:
    /// 1.5/11 = 0.136.
    #[test]
    fn one_live_candidate_in_the_window_keeps_the_campaign_running() {
        let mut history: Vec<_> = (0..10).map(|i| rec(90 + i, false, false)).collect();
        history[3].could_exceed = true;
        assert_eq!(stop_check(&history, 100), StopCheck::Continue);
    }

    /// Only the most recent comparisons are read. Early live candidates do not
    /// keep a campaign running once the recent window is dry.
    #[test]
    fn the_window_is_the_most_recent_fifth_of_the_comparisons_and_at_least_ten() {
        let mut history: Vec<_> = (0..40).map(|i| rec(i, true, false)).collect();
        history.extend((0..10).map(|i| rec(90 + i, false, false)));
        let (p, n) = exceed_posterior(&history);
        assert_eq!(n, 10, "20 % of 50 comparisons is 10");
        assert!(p < DR_THRESHOLD, "{p}");
        // 200 comparisons: the window grows to 40, and ten dry ones no longer
        // outweigh thirty live ones.
        let mut long: Vec<_> = (0..160).map(|i| rec(i, true, false)).collect();
        long.extend((0..40).map(|i| rec(1000 + i, i < 30, false)));
        let (p, n) = exceed_posterior(&long);
        assert_eq!(n, 40);
        assert!(p > DR_THRESHOLD, "{p}");
    }

    /// CONVERGED needs the incumbent to have stood for max(20, 0.25 * trials),
    /// and the diminishing-returns test to agree.
    #[test]
    fn converged_needs_a_long_standing_incumbent_and_a_dry_window() {
        // Promoted at trial 10; counted 100 -> unchanged 90 >= max(20, 25).
        let mut history = vec![rec(10, true, true)];
        history.extend((0..12).map(|i| rec(85 + i, false, false)));
        match stop_check(&history, 100) {
            StopCheck::End(CampaignPhase::Converged, why) => {
                assert!(why.contains("stood for 90"), "{why}")
            }
            other => panic!("{other:?}"),
        }
        // Promoted at trial 78, outside the ten-comparison window: unchanged
        // 22 < 25 -> only diminishing returns.
        let mut history = vec![rec(78, true, true)];
        history.extend((0..12).map(|i| rec(85 + i, false, false)));
        assert!(matches!(
            stop_check(&history, 100),
            StopCheck::End(CampaignPhase::DiminishingReturns, _)
        ));
        // A dry window is not enough on its own to converge a *young* campaign.
        let history: Vec<_> = (0..12).map(|i| rec(i, false, false)).collect();
        // counted 15: window is trials >= 12; unchanged 15 < max(20, 4)
        assert!(matches!(
            stop_check(&history, 15),
            StopCheck::End(CampaignPhase::DiminishingReturns, _)
                | StopCheck::NotEnoughEvidence { .. }
        ));
    }

    #[test]
    fn a_record_round_trips_through_the_decision_payload() {
        let chosen = json!({ "trials_at": 12, "could_exceed": true, "promoted": false });
        assert_eq!(parse_record(&chosen), Some(rec(12, true, false)));
        // A payload missing a field is not a record; it is skipped, not zeroed.
        assert_eq!(parse_record(&json!({ "trials_at": 12 })), None);
    }

    // ---- replicates ----------------------------------------------------------

    #[test]
    fn windows_cover_the_research_slice_and_are_reproducible() {
        let research = Window {
            start: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2024, 7, 1, 0, 0, 0).unwrap(),
        };
        let w = replicate_windows(&research, 29);
        assert_eq!(w.len(), 29);
        assert_eq!(w[0].0, research.start);
        assert_eq!(w[28].1, research.end);
        for (s, e) in &w {
            assert!(*s >= research.start && *e <= research.end);
            assert!(e > s);
        }
        assert_eq!(w, replicate_windows(&research, 29));
        assert!(replicate_windows(&research, 0).is_empty());
    }

    #[test]
    fn a_failed_window_on_either_side_drops_the_pair() {
        let cand = vec![Some(1.0), None, Some(1.0), Some(f64::NAN), Some(2.0)];
        let inc = vec![Some(0.5), Some(0.5), None, Some(0.5), Some(0.5)];
        let (a, b, dropped) = pair_measurements(&cand, &inc, 7);
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 2);
        assert_eq!(dropped, 3);
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.replicate, y.replicate, "paired on one replicate");
            assert_eq!(x.replicate.cost_bp, 0, "the cost axis is pinned");
        }
    }

    // ---- judge ---------------------------------------------------------------

    /// A candidate far above the incumbent on every window is promoted, and the
    /// outcome says so in the shape the stopping rule reads.
    #[test]
    fn a_clear_winner_is_promoted_through_the_protocol() {
        let cand = series(&[2.0; 29]);
        let inc = series(&[0.0; 29]);
        let (a, b, dropped) = pair_measurements(&cand, &inc, 1);
        let out = judge_pair(&plan(0.15, 3.0, a.len()), &a, &b, dropped).unwrap();
        assert_eq!(out.verdict, "promote");
        assert!(out.promoted && out.could_exceed);
        assert_eq!(out.achieved_k, 29);
    }

    /// A candidate clearly below the incumbent is rejected, which is the one
    /// verdict that says "could not have exceeded delta".
    #[test]
    fn a_clear_loser_is_rejected_and_cannot_exceed() {
        let cand = series(&[0.0; 29]);
        let inc = series(&[2.0; 29]);
        let (a, b, dropped) = pair_measurements(&cand, &inc, 1);
        let out = judge_pair(&plan(0.15, 3.0, a.len()), &a, &b, dropped).unwrap();
        assert_eq!(out.verdict, "reject");
        assert!(!out.promoted && !out.could_exceed);
    }

    /// Identical results are indistinguishable, not a loss, and they leave the
    /// question of exceeding delta open.
    #[test]
    fn identical_candidates_are_practically_equivalent() {
        let cand = series(&[1.0; 29]);
        let (a, b, dropped) = pair_measurements(&cand, &cand, 1);
        let out = judge_pair(&plan(0.15, 3.0, a.len()), &a, &b, dropped).unwrap();
        assert_eq!(out.verdict, "practically_equivalent");
        assert!(!out.promoted && out.could_exceed);
    }

    /// Too few windows survived: the protocol refuses and the caller fails the
    /// job rather than filing a verdict from four numbers.
    #[test]
    fn too_few_surviving_windows_is_a_refusal_not_a_verdict() {
        let cand = series(&[1.0, 2.0, 3.0]);
        let (a, b, dropped) = pair_measurements(&cand, &cand, 1);
        let err = judge_pair(&plan(0.15, 3.0, 3), &a, &b, dropped).unwrap_err();
        assert!(err.contains("at least"), "{err}");
    }

    /// A difference outside the declared scale is refused, never clipped.
    #[test]
    fn a_difference_outside_the_declared_scale_is_refused() {
        let cand = series(&[10.0; 29]);
        let inc = series(&[0.0; 29]);
        let (a, b, dropped) = pair_measurements(&cand, &inc, 1);
        let err = judge_pair(&plan(0.15, 1.0, a.len()), &a, &b, dropped).unwrap_err();
        assert!(err.contains("outside the declared scale"), "{err}");
    }
}
