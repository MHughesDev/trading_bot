//! The campaign driver (SPEC §10, checklist 2.1, ADR-P2-04, AT-59).
//!
//! A campaign is a fold over `mlops.campaign_event`. This worker is the thing
//! that advances the fold: it reads the log, decides the next phase, dispatches
//! that phase's work as a **child job**, records what it dispatched, and waits.
//! Then it does it again.
//!
//! Everything durable about it comes from two places that already exist:
//!
//! * the **log** — append-only, immutable, transition-checked by a database
//!   trigger. A driver killed at any point recomputes its state by replaying it,
//!   because there is nothing else to recompute from ([`ledger::phase`]);
//! * the **child job's position** — `(campaign job, seq)`, where `seq` is the
//!   fold's own length. `submit_child_once` returns the child already at that
//!   position, so a restart between "created the child" and "recorded that I
//!   created the child" reuses it instead of starting a second one. That is the
//!   exactly-once property a workflow engine would have been adopted for.
//!
//! ## The three mechanical terminal rules
//!
//! All three are computed, and each from something recorded rather than
//! remembered:
//!
//! * `BUDGET_EXHAUSTED` — the campaign's `max_trials` against the trials the
//!   ledger has actually counted for it.
//! * `DIMINISHING_RETURNS` — every comparison recorded over the last 20 % of
//!   the campaign's trials has an anytime-valid upper bound below
//!   `delta_practical`. The interval is the confidence sequence
//!   `ComparisonPlan::judge` produced (§11.5), so "the posterior probability
//!   that any remaining candidate exceeds the incumbent by `delta_practical`
//!   falls below 0.05" is read off the recorded bound rather than approximated.
//! * `CONVERGED` — the incumbent has been unchanged for `max(20, 0.25 · trials)`
//!   trials **and** the diminishing-returns test agrees. A promotion is the only
//!   thing that moves the incumbent, so "unchanged" is measured from the last
//!   comparison whose verdict was `promote`, or from the campaign's first trial
//!   if there has never been one.
//!
//! Each rule still reports [`TerminationCheck::NotComputable`] when its inputs
//! are absent — a campaign with no recorded comparison cannot be said to have
//! converged — and the reasons travel in the job's output, so a reader can see
//! which rules were live for a given campaign rather than assuming all of them
//! were.
//!
//! `HALTED` is a human appending the event; the driver only observes it.

use std::sync::Arc;
use std::time::Duration;

use jobs::store::{JobStore, Submission};
use jobs::{JobContext, JobError, JobKind, JobOutput, JobState, Progress, Worker};
use ledger::phase::{CampaignEvent, CampaignPhase, CampaignState};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// How often the driver looks at a running child.
const CHILD_POLL: Duration = Duration::from_secs(5);

/// Why a campaign ended, or why it cannot be said to have ended yet.
#[derive(Debug, Clone, PartialEq)]
pub enum TerminationCheck {
    /// Keep going.
    Continue,
    /// End, in this phase.
    End(CampaignPhase, String),
    /// A §10 rule that needs machinery this build does not have. Named, so a
    /// reader of the campaign can see which rules are live and which are not.
    NotComputable(&'static str),
}

/// Drives one campaign per job.
pub struct CampaignWorker {
    store: Arc<JobStore>,
    pg: PgPool,
}

impl CampaignWorker {
    #[must_use]
    pub fn new(store: Arc<JobStore>, pg: PgPool) -> Self {
        Self { store, pg }
    }

    fn ledger(&self) -> ledger::pg::PgTrialLedger {
        ledger::pg::PgTrialLedger::new(self.pg.clone())
    }

    /// The campaign's DEFINE facts, read back from the row they were locked in.
    async fn budget_max_trials(&self, tenant: &str, campaign_id: Uuid) -> Result<i64, JobError> {
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant)
            .await
            .map_err(|e| JobError::infrastructure(format!("tenant transaction: {e}")))?;
        let row: Option<(Value,)> =
            sqlx::query_as("SELECT budget FROM mlops.campaign WHERE campaign_id = $1")
                .bind(campaign_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| JobError::infrastructure(format!("reading the campaign: {e}")))?;
        let _ = tx.commit().await;
        let budget = row.ok_or_else(|| {
            JobError::logic(
                ledger::TerminalReason::IntegrityRejected,
                "unknown_campaign",
                "the manifest names a campaign this tenant has not defined",
            )
        })?;
        budget
            .0
            .get("max_trials")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "campaign_budget_unreadable",
                    "the campaign's budget has no max_trials; every budget dimension is REQUIRED at DEFINE (§8)",
                )
            })
    }

    /// The §10 terminal rules, evaluated against what the ledger actually holds,
    /// in the order they are checked.
    ///
    /// Reported as a list rather than collapsed to the first hit, so the job's
    /// output can say which rules were live and which could not be computed. A
    /// campaign that ran to its budget because nothing else was computable is a
    /// different fact from one that ran to its budget with every rule watching.
    async fn termination_report(
        &self,
        tenant: &str,
        campaign_id: Uuid,
    ) -> Result<Vec<(&'static str, TerminationCheck)>, JobError> {
        let max_trials = self.budget_max_trials(tenant, campaign_id).await?;
        let trial_times = self
            .ledger()
            .campaign_trial_times_async(tenant, campaign_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading the trial timeline: {e}")))?;
        let comparisons = self
            .ledger()
            .comparisons_async(tenant, campaign_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading the comparisons: {e}")))?;

        let counted = i64::try_from(trial_times.len()).unwrap_or(i64::MAX);
        let budget = if counted >= max_trials {
            TerminationCheck::End(
                CampaignPhase::BudgetExhausted,
                format!("{counted} trials counted against a budget of {max_trials}"),
            )
        } else {
            TerminationCheck::Continue
        };

        let diminishing = diminishing_returns(&trial_times, &comparisons);
        let converged = converged(&trial_times, &comparisons, &diminishing);

        // Budget first: it is the one limit that is external to the campaign's
        // own evidence, and spending past it is the failure the other two are
        // trying to avoid. CONVERGED before DIMINISHING_RETURNS because it is
        // the stronger statement and subsumes the weaker one.
        Ok(vec![
            ("budget_exhausted", budget),
            ("converged", converged),
            ("diminishing_returns", diminishing),
        ])
    }

    /// The first §10 rule that says to stop, if any.
    async fn should_end(
        &self,
        tenant: &str,
        campaign_id: Uuid,
    ) -> Result<TerminationCheck, JobError> {
        for (_, check) in self.termination_report(tenant, campaign_id).await? {
            if matches!(check, TerminationCheck::End(_, _)) {
                return Ok(check);
            }
        }
        Ok(TerminationCheck::Continue)
    }

    /// The child job kind that does this phase's work.
    ///
    /// `None` for the phases the driver performs itself — `hypothesize` is the
    /// agent (or a human) proposing, and `prune`/`reallocate` are decisions,
    /// which are written to `mlops.decision` with their propensity rather than
    /// executed as compute.
    fn child_kind(phase: CampaignPhase) -> Option<JobKind> {
        match phase {
            // The benchmark, run once, through the ledger like anything else.
            CampaignPhase::Baseline => Some(JobKind::Backtest),
            CampaignPhase::Diagnose => Some(JobKind::ResearchRun),
            CampaignPhase::Experiment => Some(JobKind::Study),
            CampaignPhase::Compare => Some(JobKind::EvalTask),
            CampaignPhase::Gate => Some(JobKind::GateAdvance),
            _ => None,
        }
    }

    /// Wait for a child to reach a terminal state, reporting progress meanwhile.
    ///
    /// Returns the terminal state. A cancelled *driver* does not cancel the
    /// child: the child is a counted look that is already under way, and
    /// abandoning it without settling its trial is exactly what INV-17 forbids.
    async fn await_child(
        &self,
        ctx: &JobContext,
        child_id: &str,
        phase: CampaignPhase,
    ) -> Result<JobState, JobError> {
        loop {
            let child = self
                .store
                .get(child_id)
                .await
                .map_err(|e| JobError::infrastructure(format!("reading child {child_id}: {e}")))?;
            if child.state.is_terminal() {
                return Ok(child.state);
            }
            ctx.progress(Progress {
                pct: None,
                stage: Some(phase.as_str().to_string()),
                message: Some(format!("waiting on {child_id} ({})", child.state.as_str())),
            })
            .await;
            tokio::time::sleep(CHILD_POLL).await;
        }
    }

    /// Dispatch one phase and record it, or record it alone when the phase has
    /// no compute.
    ///
    /// The order matters and is the opposite of the intuitive one: the child is
    /// created *first* and the event written *second*. A crash in between leaves
    /// a child nothing points at, which the next fold re-derives and reuses. A
    /// crash the other way round would leave an event claiming a child that does
    /// not exist, and no amount of replaying fixes that.
    async fn advance(
        &self,
        ctx: &JobContext,
        tenant: &str,
        campaign_id: Uuid,
        state: &CampaignState,
        phase: CampaignPhase,
    ) -> Result<(), JobError> {
        let key = state.child_key(campaign_id, phase);
        let mut detail = json!({ "idempotency_key": key });

        if let Some(kind) = Self::child_kind(phase) {
            let parent = self
                .store
                .get(&ctx.job_id)
                .await
                .map_err(|e| JobError::infrastructure(format!("reading this job: {e}")))?;
            let mut sub = Submission::new(
                kind,
                parent.user_id,
                json!({
                    "campaign_id": campaign_id,
                    "phase": phase.as_str(),
                    "seq": state.seq,
                    "idempotency_key": key,
                }),
            );
            sub.project_id = parent.project_id;
            sub.parent_job_id = Some(ctx.job_id.clone());
            sub.member_index = Some(i32::try_from(state.seq).unwrap_or(i32::MAX));
            sub.submitted_by = parent.submitted_by;
            // Counted kinds need an experiment to be counted against; the
            // campaign's own phase position is that experiment.
            if kind.counts_trial() {
                sub.experiment_id = Some(format!("campaign:{campaign_id}:{}", phase.as_str()));
            }
            let child = self
                .store
                .submit_child_once(sub)
                .await
                .map_err(|e| JobError::infrastructure(format!("dispatching {phase:?}: {e}")))?;
            detail["job_id"] = json!(child.job_id);
            detail["kind"] = json!(kind.as_str());

            self.append(tenant, campaign_id, phase, detail).await?;
            let outcome = self.await_child(ctx, &child.job_id, phase).await?;
            if outcome != JobState::Succeeded {
                // The campaign is *not* terminated by this. A phase whose work
                // failed leaves the log where it is; re-running the driver
                // resumes at the same position, and §10's endings stay reserved
                // for the four things that actually end a campaign.
                return Err(JobError::infrastructure(format!(
                    "child job {} for phase {} ended {}",
                    child.job_id,
                    phase.as_str(),
                    outcome.as_str()
                )));
            }
            return Ok(());
        }

        self.append(tenant, campaign_id, phase, detail).await
    }

    async fn append(
        &self,
        tenant: &str,
        campaign_id: Uuid,
        phase: CampaignPhase,
        detail: Value,
    ) -> Result<(), JobError> {
        self.ledger()
            .append_campaign_event_async(tenant, campaign_id, &CampaignEvent::new(phase, detail))
            .await
            .map_err(|e| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "campaign_transition_refused",
                    format!("appending {} to campaign {campaign_id}: {e}", phase.as_str()),
                )
            })?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Worker for CampaignWorker {
    fn kind(&self) -> JobKind {
        JobKind::Campaign
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let campaign_id: Uuid = manifest
            .get("campaign_id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "invalid_manifest",
                    "a campaign job needs a campaign_id",
                )
            })?;
        let parent = self
            .store
            .get(&ctx.job_id)
            .await
            .map_err(|e| JobError::infrastructure(format!("reading this job: {e}")))?;
        let tenant = parent.user_id.to_string();

        let mut phases_run = 0_u32;
        loop {
            let state = self
                .ledger()
                .campaign_state_async(&tenant, campaign_id)
                .await
                .map_err(|e| {
                    JobError::logic(
                        ledger::TerminalReason::IntegrityRejected,
                        "campaign_log_unreadable",
                        format!("folding campaign {campaign_id}: {e}"),
                    )
                })?;

            if let Some(end) = state.terminal {
                // Which §10 rules were live when it ended, and which could not
                // be computed. A campaign that reached its budget with two rules
                // unevaluable is a different fact from one that reached it with
                // all three watching, and the output has to be able to say so.
                let rules: Vec<Value> = self
                    .termination_report(&tenant, campaign_id)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(name, check)| match check {
                        TerminationCheck::Continue => json!({ "rule": name, "state": "live" }),
                        TerminationCheck::End(phase, why) => json!({
                            "rule": name, "state": "fired", "phase": phase.as_str(), "why": why
                        }),
                        TerminationCheck::NotComputable(why) => {
                            json!({ "rule": name, "state": "not_computable", "why": why })
                        }
                    })
                    .collect();
                return Ok(JobOutput {
                    summary: Some(format!(
                        "campaign {campaign_id} ended in {} after {} events ({phases_run} advanced here)",
                        end.as_str(),
                        state.seq
                    )),
                    result: json!({
                        "campaign_id": campaign_id,
                        "terminal": end.as_str(),
                        "seq": state.seq,
                        "cycles": state.cycles,
                        "termination_rules": rules,
                    }),
                    ..Default::default()
                });
            }

            // A driver asked to stop leaves the campaign exactly where it is.
            // The log is the state, so there is nothing to unwind.
            if ctx.is_cancelled() {
                return Ok(JobOutput {
                    summary: Some(format!(
                        "driver cancelled with campaign {campaign_id} in {} at seq {}",
                        state.phase.as_str(),
                        state.seq
                    )),
                    result: json!({ "campaign_id": campaign_id, "phase": state.phase.as_str() }),
                    ..Default::default()
                });
            }

            if let TerminationCheck::End(phase, why) = self.should_end(&tenant, campaign_id).await? {
                self.append(&tenant, campaign_id, phase, json!({ "reason": why })).await?;
                continue;
            }

            let Some(next) = state.next_phase() else {
                return Err(JobError::logic(
                    ledger::TerminalReason::IntegrityRejected,
                    "campaign_stuck",
                    format!(
                        "campaign {campaign_id} is in {} with no successor and no ending",
                        state.phase.as_str()
                    ),
                ));
            };
            self.advance(ctx, &tenant, campaign_id, &state, next).await?;
            phases_run += 1;
        }
    }
}

// ── §10's evidence-driven terminal rules ─────────────────────────────────────

/// The fraction of a campaign's trials the diminishing-returns test looks at
/// (§10: "estimated over the last 20 % of trials").
const RECENT_TRIAL_FRACTION: f64 = 0.20;

/// The two halves of CONVERGED's "unchanged for long enough" (PHASE-2-5-PLAN
/// §2.1): an absolute floor, and a quarter of the campaign's trials so far.
const CONVERGED_MIN_TRIALS: i64 = 20;
const CONVERGED_TRIAL_FRACTION: f64 = 0.25;

/// The first index of the campaign's most recent `fraction` of trials.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn recent_window_start(
    trial_times: &[chrono::DateTime<chrono::Utc>],
    fraction: f64,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let n = trial_times.len();
    if n == 0 {
        return None;
    }
    let take = ((n as f64) * fraction).ceil().max(1.0) as usize;
    trial_times.get(n.saturating_sub(take)).copied()
}

/// §10's DIMINISHING_RETURNS, computed rather than approximated.
///
/// "Stop when the posterior probability that any remaining candidate exceeds
/// the incumbent by `delta_practical` falls below 0.05, estimated over the last
/// 20 % of trials."
///
/// The posterior is the confidence sequence `ComparisonPlan::judge` already
/// produced and the ledger already stored. It is anytime-valid at `alpha`, so
/// an upper bound below `δ` *is* the statement that the probability of a
/// `δ`-sized improvement is at most `alpha` — and it is valid after any number
/// of peeks, which is the only reason a rule like this can be evaluated on
/// every pass of a driver loop.
///
/// **Every** comparison in the window has to rule `δ` out. One candidate still
/// in contention is a remaining candidate, and §10 asks about *any*.
fn diminishing_returns(
    trial_times: &[chrono::DateTime<chrono::Utc>],
    comparisons: &[ledger::evidence::StoredComparison],
) -> TerminationCheck {
    let Some(from) = recent_window_start(trial_times, RECENT_TRIAL_FRACTION) else {
        return TerminationCheck::NotComputable("no trials have been counted for this campaign");
    };
    let recent: Vec<&ledger::evidence::StoredComparison> =
        comparisons.iter().filter(|c| c.recorded_at >= from).collect();
    if recent.is_empty() {
        return TerminationCheck::NotComputable(
            "no comparison has been recorded over the campaign's most recent trials",
        );
    }
    if recent.iter().all(|c| c.rules_out_delta()) {
        let worst = recent
            .iter()
            .map(|c| c.record.cs_upper)
            .fold(f64::NEG_INFINITY, f64::max);
        let delta = recent[0].record.delta_practical;
        return TerminationCheck::End(
            CampaignPhase::DiminishingReturns,
            format!(
                "every one of the {} comparisons over the last {:.0}% of trials bounds the \
                 improvement below the declared effect size (worst upper bound {worst:+.4} < \
                 δ {delta:.4})",
                recent.len(),
                RECENT_TRIAL_FRACTION * 100.0,
            ),
        );
    }
    TerminationCheck::Continue
}

/// §10's CONVERGED: the incumbent unchanged for `max(20, 0.25 · trials)` trials
/// **and** the diminishing-returns test agreeing.
///
/// Only a promotion moves the incumbent, so "unchanged since" is the time of
/// the last comparison whose verdict was `promote` — or the campaign's first
/// trial, because a campaign that has never promoted anything has had the same
/// incumbent, the declared benchmark, from the beginning.
///
/// The conjunction is not decoration. A campaign can sit on one incumbent
/// because its search has stalled rather than because it has finished, and
/// without the second half this rule would call that convergence and stop
/// spending a budget that was still buying information.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn converged(
    trial_times: &[chrono::DateTime<chrono::Utc>],
    comparisons: &[ledger::evidence::StoredComparison],
    diminishing: &TerminationCheck,
) -> TerminationCheck {
    match diminishing {
        TerminationCheck::NotComputable(why) => return TerminationCheck::NotComputable(why),
        TerminationCheck::Continue => {
            return TerminationCheck::Continue;
        }
        TerminationCheck::End(_, _) => {}
    }
    let Some(first) = trial_times.first().copied() else {
        return TerminationCheck::NotComputable("no trials have been counted for this campaign");
    };
    let since = comparisons
        .iter()
        .filter(|c| c.promoted())
        .map(|c| c.recorded_at)
        .max()
        .unwrap_or(first);
    let unchanged_for = i64::try_from(trial_times.iter().filter(|t| **t >= since).count())
        .unwrap_or(i64::MAX);
    let n = trial_times.len() as f64;
    let required = CONVERGED_MIN_TRIALS.max((n * CONVERGED_TRIAL_FRACTION).ceil() as i64);
    if unchanged_for >= required {
        return TerminationCheck::End(
            CampaignPhase::Converged,
            format!(
                "the incumbent has been unchanged for {unchanged_for} trials (needs {required}) \
                 and the diminishing-returns test agrees"
            ),
        );
    }
    TerminationCheck::Continue
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every phase that §10 says dispatches work has a child kind, and every
    /// phase that does not, does not. The two lists are stated in different
    /// crates — `has_child_job` in the ledger, `child_kind` here — so this is
    /// what keeps them the same list.
    #[test]
    fn the_phases_with_child_jobs_are_the_phases_with_child_kinds() {
        for p in CampaignPhase::ALL {
            assert_eq!(
                p.has_child_job(),
                CampaignWorker::child_kind(p).is_some(),
                "{p:?}: the ledger and the driver disagree about whether this phase dispatches work"
            );
        }
    }

    /// A campaign's compute goes through counted job kinds. If a phase's work
    /// stopped counting, the campaign would be searching without the trial
    /// counter climbing, which is the one thing the ledger exists to prevent.
    #[test]
    fn the_phases_that_look_at_out_of_sample_behaviour_count_trials() {
        for p in [CampaignPhase::Baseline, CampaignPhase::Experiment, CampaignPhase::Gate] {
            let kind = CampaignWorker::child_kind(p).expect("has a child kind");
            assert!(kind.counts_trial(), "{p:?} dispatches {kind}, which is not counted");
        }
    }
}

#[cfg(test)]
mod termination_tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use ledger::evidence::{ComparisonRecord, StoredComparison};

    fn t0() -> chrono::DateTime<chrono::Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn trials(n: usize) -> Vec<chrono::DateTime<chrono::Utc>> {
        (0..n).map(|i| t0() + Duration::minutes(i as i64)).collect()
    }

    fn comparison(at_minute: i64, cs_upper: f64, verdict: &str) -> StoredComparison {
        StoredComparison {
            record: ComparisonRecord {
                campaign_seq: at_minute,
                arm_a: "challenger:1".into(),
                arm_b: "incumbent".into(),
                metric: "sharpe_net".into(),
                verdict: verdict.into(),
                cs_centre: 0.0,
                cs_lower: -1.0,
                cs_upper,
                alpha: 0.05,
                mean_difference: 0.0,
                p_a_better: 0.5,
                p_a_better_lower: 0.2,
                achieved_k: 9,
                wins: 4,
                ties: 1,
                losses: 4,
                profile_id: "strict_v1".into(),
                n_eff: 10.0,
                delta_practical: 0.25,
                trials_a: 9,
                trials_b: 9,
                flags: serde_json::json!({}),
                headline: "h".into(),
            },
            recorded_at: t0() + Duration::minutes(at_minute),
        }
    }

    /// Absent evidence stays absent. A campaign with no trials, or with no
    /// comparison recorded over the window §10 names, cannot be said to have hit
    /// diminishing returns — and the rule says so rather than defaulting either
    /// way. This is the behaviour that used to be the *only* behaviour.
    #[test]
    fn diminishing_returns_is_not_computable_without_a_comparison_in_the_window() {
        assert!(matches!(
            diminishing_returns(&[], &[]),
            TerminationCheck::NotComputable(_)
        ));
        assert!(matches!(
            diminishing_returns(&trials(50), &[]),
            TerminationCheck::NotComputable(_)
        ));
        // Recorded before the window opened: 20% of 50 trials is the last 10, so
        // the window starts at minute 40 and a comparison at minute 5 is old news.
        assert!(matches!(
            diminishing_returns(&trials(50), &[comparison(5, 0.01, "reject")]),
            TerminationCheck::NotComputable(_)
        ));
    }

    /// The rule fires only when every comparison in the window bounds the
    /// improvement below δ. One candidate still in contention is a remaining
    /// candidate, and §10 asks about *any* of them.
    #[test]
    fn one_candidate_still_in_contention_keeps_the_campaign_running() {
        let ts = trials(50);
        let closed = comparison(45, 0.01, "reject");
        let open = comparison(46, 0.90, "keep_sampling");

        assert!(matches!(
            diminishing_returns(&ts, &[closed.clone()]),
            TerminationCheck::End(CampaignPhase::DiminishingReturns, _)
        ));
        assert!(matches!(
            diminishing_returns(&ts, &[closed, open]),
            TerminationCheck::Continue
        ));
    }

    /// CONVERGED requires both halves. A campaign whose incumbent has stood for
    /// a long time but whose candidates are still in contention has not
    /// converged — it is still learning something, and stopping it would leave
    /// budget unspent on a question still open.
    #[test]
    fn converged_needs_the_diminishing_returns_test_to_agree() {
        let ts = trials(40);
        let contested = vec![comparison(33, 0.01, "reject"), comparison(34, 0.9, "keep_sampling")];
        let dr = diminishing_returns(&ts, &contested);
        assert!(matches!(dr, TerminationCheck::Continue));
        assert!(matches!(converged(&ts, &contested, &dr), TerminationCheck::Continue));

        // With every candidate ruled out and no promotion ever recorded, the
        // incumbent has stood for all 40 trials against a requirement of
        // max(20, 10) = 20.
        let settled = vec![comparison(33, 0.01, "reject")];
        let dr = diminishing_returns(&ts, &settled);
        assert!(matches!(dr, TerminationCheck::End(CampaignPhase::DiminishingReturns, _)));
        assert!(matches!(
            converged(&ts, &settled, &dr),
            TerminationCheck::End(CampaignPhase::Converged, _)
        ));
    }

    /// A recent promotion moves the incumbent, so the clock restarts. This is
    /// the half that stops a campaign being declared converged on the strength
    /// of a candidate it adopted moments ago.
    #[test]
    fn a_recent_promotion_restarts_the_incumbents_clock() {
        let ts = trials(40);
        let after_a_promotion = vec![
            comparison(36, 0.01, "promote"),
            comparison(37, 0.01, "reject"),
        ];
        let dr = diminishing_returns(&ts, &after_a_promotion);
        assert!(matches!(dr, TerminationCheck::End(CampaignPhase::DiminishingReturns, _)));
        // Only trials at minute 36 and later count as "unchanged": 4 of 40,
        // against a requirement of 20.
        assert!(matches!(
            converged(&ts, &after_a_promotion, &dr),
            TerminationCheck::Continue
        ));
    }

    /// The window is the last 20 % of *trials*, measured on the trial timeline
    /// rather than on a wall-clock duration — a campaign that ran twice as fast
    /// gets the same window.
    #[test]
    fn the_window_is_the_last_fifth_of_the_trial_timeline() {
        let ts = trials(100);
        assert_eq!(recent_window_start(&ts, 0.20), Some(t0() + Duration::minutes(80)));
        // Small campaigns still get a window rather than an empty one.
        assert_eq!(recent_window_start(&trials(1), 0.20), Some(t0()));
        assert_eq!(recent_window_start(&[], 0.20), None);
    }

    /// CONVERGED's requirement is `max(20, 0.25 · trials)`, so a long campaign
    /// needs proportionally more quiet and a short one still needs twenty.
    #[test]
    fn the_converged_requirement_scales_with_the_campaign() {
        // 200 trials ⇒ needs 50 unchanged; the last promotion was 40 trials ago.
        let ts = trials(200);
        let cs = vec![comparison(160, 0.01, "promote"), comparison(199, 0.01, "reject")];
        let dr = diminishing_returns(&ts, &cs);
        assert!(matches!(dr, TerminationCheck::End(CampaignPhase::DiminishingReturns, _)));
        assert!(matches!(converged(&ts, &cs, &dr), TerminationCheck::Continue));

        // Move the promotion back to 60 trials ago and it clears 50.
        let cs = vec![comparison(140, 0.01, "promote"), comparison(199, 0.01, "reject")];
        let dr = diminishing_returns(&ts, &cs);
        assert!(matches!(
            converged(&ts, &cs, &dr),
            TerminationCheck::End(CampaignPhase::Converged, _)
        ));
    }
}
