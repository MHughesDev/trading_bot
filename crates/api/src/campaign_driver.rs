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
//! ## What is not here yet, and is not pretended to be
//!
//! §10 has three mechanical terminal rules. Only one of them can be computed
//! today:
//!
//! * `BUDGET_EXHAUSTED` — implemented: the campaign's `max_trials` against the
//!   trials the ledger has actually counted for it.
//! * `DIMINISHING_RETURNS` and `CONVERGED` — both need the posterior from the
//!   comparison protocol's confidence sequence (checklist 2.12), which is not
//!   built. [`TerminationCheck`] names them as `NotComputable` rather than
//!   approximating them, because a campaign that stops early on a made-up
//!   posterior is worse than one that runs to its budget.
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

    /// Trials the ledger has counted for this campaign — every look, including
    /// failures, cache hits and gate failures, because the budget is a budget on
    /// looks the same way the exploration floor is (§4.5, §8).
    async fn trials_counted(&self, tenant: &str, campaign_id: Uuid) -> Result<i64, JobError> {
        let mut tx = ledger::pg::tenant_tx(&self.pg, tenant)
            .await
            .map_err(|e| JobError::infrastructure(format!("tenant transaction: {e}")))?;
        let count: (i64,) =
            sqlx::query_as("SELECT count(*) FROM mlops.trial WHERE campaign_id = $1")
                .bind(campaign_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| JobError::infrastructure(format!("counting trials: {e}")))?;
        let _ = tx.commit().await;
        Ok(count.0)
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

    /// The §10 terminal rules, evaluated against what the ledger actually holds.
    async fn should_end(
        &self,
        tenant: &str,
        campaign_id: Uuid,
    ) -> Result<TerminationCheck, JobError> {
        let max_trials = self.budget_max_trials(tenant, campaign_id).await?;
        let counted = self.trials_counted(tenant, campaign_id).await?;
        if counted >= max_trials {
            return Ok(TerminationCheck::End(
                CampaignPhase::BudgetExhausted,
                format!("{counted} trials counted against a budget of {max_trials}"),
            ));
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
