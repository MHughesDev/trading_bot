//! Durable job store on Postgres (COMP-005 §3, §5, §10).

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::manifest;
use crate::types::*;

/// How long a lease lasts before a job is considered abandoned (COMP-005 §5).
pub const LEASE_SECONDS: i64 = 60;

/// The artifact type a resumable training checkpoint is stored under.
pub const CHECKPOINT_TYPE: &str = "checkpoint";

/// The only `job_artifacts` role a checkpoint may occupy.
///
/// Its own role, and only its own: a checkpoint under `model` or `result` is a
/// partial model in the slot a finished one belongs in, which is how a half
/// trained network ends up serving or being scored.
pub const CHECKPOINT_ROLE: &str = "checkpoint";

/// Infrastructure re-queues allowed before a job fails with `lost_worker`.
pub const MAX_ATTEMPTS: i16 = 2;

#[derive(Debug, thiserror::Error)]
pub enum JobStoreError {
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("experiment_required: kind {0} counts as a trial and needs an experiment_id")]
    ExperimentRequired(String),
    #[error("max_gpu_hours_required: {0} runs on the trainer pool and must declare max_gpu_hours; there is no platform default")]
    MaxGpuHoursRequired(String),
    #[error("budget_exhausted: {0}")]
    BudgetExhausted(String),
    #[error("not_found: {0}")]
    NotFound(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("ledger: {0}")]
    Ledger(#[from] ledger::LedgerError),
}

/// A submission request.
#[derive(Debug, Clone)]
pub struct Submission {
    pub kind: JobKind,
    pub project_id: Option<Uuid>,
    pub user_id: Uuid,
    pub session_id: Option<Uuid>,
    pub submitted_by: SubmittedBy,
    pub experiment_id: Option<String>,
    pub parent_job_id: Option<String>,
    pub member_index: Option<i32>,
    pub priority: i16,
    pub manifest: Value,
    pub code_hashes: Vec<String>,
    pub data_snapshot_id: String,
    pub estimate: Option<Cost>,
}

impl Submission {
    pub fn new(kind: JobKind, user_id: Uuid, manifest: Value) -> Self {
        Self {
            kind,
            project_id: None,
            user_id,
            session_id: None,
            submitted_by: SubmittedBy::System,
            experiment_id: None,
            parent_job_id: None,
            member_index: None,
            priority: 5,
            manifest,
            code_hashes: vec![],
            data_snapshot_id: "latest".into(),
            estimate: None,
        }
    }

    pub fn manifest_hash(&self) -> String {
        manifest::hash(
            &self.manifest,
            &self.code_hashes,
            &self.data_snapshot_id,
            self.kind.as_str(),
        )
    }
}

/// The outcome of a submission.
#[derive(Debug, Clone)]
pub struct Submitted {
    pub job_id: String,
    /// True when an identical job already existed (JB-02). No new trial is counted.
    pub deduplicated: bool,
    pub state: JobState,
}

/// A job row.
#[derive(Debug, Clone)]
pub struct Job {
    /// The trial this job dispatches, for trial-counted kinds (INV-16).
    pub trial_id: Option<Uuid>,
    pub job_id: String,
    pub kind: JobKind,
    pub project_id: Option<Uuid>,
    pub user_id: Uuid,
    pub session_id: Option<Uuid>,
    /// The principal the job service stamped at submission from the auth token's
    /// scope (§8, ADR-0025). A child job inherits it, so "agent X acting for
    /// user Y" survives a campaign phase without the agent ever writing it.
    pub submitted_by: SubmittedBy,
    pub experiment_id: Option<String>,
    pub parent_job_id: Option<String>,
    pub queue: Queue,
    pub worker_class: WorkerClass,
    pub priority: i16,
    pub manifest: Value,
    pub manifest_hash: String,
    pub state: JobState,
    pub progress: Value,
    pub result_summary: Option<String>,
    pub result: Option<Value>,
    pub error: Option<Value>,
    pub attempts: i16,
    pub cancel_requested: bool,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    /// Wall-clock seconds this job has consumed across **every** attempt
    /// (migration 0252).
    ///
    /// Charged by the database when an attempt ends, from the lease rather than
    /// from a worker's self-report — a killed worker reports nothing, and that is
    /// precisely the attempt whose spend must still count against
    /// `max_gpu_hours`.
    pub gpu_seconds_spent: f64,
}

/// Hook called inside the submission transaction to count a trial (INV-1, JB-03).
///
/// It takes the transaction, so registration and job insert commit or roll back
/// together. That is the whole point: a trial that was counted for a job that does
/// not exist would inflate the count, and a job that exists without a counted trial
/// would deflate it. Neither is recoverable after the fact.
#[async_trait::async_trait]
pub trait TrialCounter: Send + Sync {
    /// Register the trial this job dispatches, inside the submission transaction,
    /// and return its id.
    async fn register_trial(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        submission: &Submission,
        job_id: &str,
    ) -> Result<Uuid, JobStoreError>;
}

/// A counter that refuses everything — the safe default when no evaluation service
/// is wired in. Submitting a trial-counted kind then fails loudly rather than
/// quietly skipping the count.
pub struct RefuseTrials;

#[async_trait::async_trait]
impl TrialCounter for RefuseTrials {
    async fn register_trial(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        submission: &Submission,
        _job_id: &str,
    ) -> Result<Uuid, JobStoreError> {
        Err(JobStoreError::Invalid(format!(
            "no evaluation service is wired in, so the trial for a {} job cannot be \
             counted; refusing the submission rather than losing the count",
            submission.kind
        )))
    }
}

/// Registers each trial-counted job in the Trial Ledger (`mlops.trial`), inside the
/// submission transaction, so the job row and its trial commit together.
///
/// Job submissions do not yet carry a logged propensity or a campaign-declared
/// effect size, so they register under the legacy marker: counted in N_eff,
/// excluded from off-policy estimators (ADR-P0-10). Campaign-driven dispatch
/// replaces this with fully pre-registered trials.
pub struct PgTrialCounter;

#[async_trait::async_trait]
impl TrialCounter for PgTrialCounter {
    async fn register_trial(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        submission: &Submission,
        job_id: &str,
    ) -> Result<Uuid, JobStoreError> {
        let actor_kind = match submission.submitted_by {
            SubmittedBy::Agent => ledger::ActorKind::Agent,
            SubmittedBy::System => ledger::ActorKind::Scheduler,
            _ => ledger::ActorKind::Human,
        };
        let mut ctx = ledger::DispatchContext::legacy(
            submission.user_id.to_string(),
            actor_kind,
            submission.user_id.to_string(),
        );
        ctx.experiment_id.clone_from(&submission.experiment_id);
        // A campaign's children are counted *against that campaign*. The driver
        // reads `count(*) FROM mlops.trial WHERE campaign_id = $1` to decide
        // BUDGET_EXHAUSTED, and a child registered with no campaign was counted
        // in N_eff but invisible to that query, so no campaign's budget could
        // ever run out (2026-10-03, campaign track). Only a *child* job whose
        // manifest names a campaign qualifies: a top-level submission that merely
        // carries a `campaign_id` field is not thereby driven by one.
        ctx.campaign_id = campaign_of(submission);
        if let Some(session) = submission.session_id {
            ctx.on_behalf_of = Some(format!("session:{session}"));
        }
        let config = serde_json::json!({ "kind": submission.kind.as_str(), "manifest": submission.manifest });
        let subject = ledger::TrialSubject {
            config_hash: submission.manifest_hash(),
            config,
            dataset_id: format!("snapshot:{}", submission.data_snapshot_id),
            split_spec_id: None,
            code_hash: submission.code_hashes.join(","),
            image_digest: format!("worker:{}", submission.kind.worker_class().as_str()),
            seed_set: Vec::new(),
            non_reproducible: false,
            overlapping_labels_unweighted: false,
            split_overrides: serde_json::json!([]),
            planned_steps: None,
        };
        let ticket =
            ledger::pg::register_in_tx(tx, &ledger::Registration::unlogged(&ctx, &subject)).await?;
        let trial_id = ticket.trial_id();
        tracing::debug!(job = job_id, trial = %trial_id, "trial registered");
        Ok(trial_id)
    }
}

/// The campaign a child job is counted against, when it is one.
///
/// Pure so the rule is testable without a database: a submission has a campaign
/// only if it has a parent job (the driver's) *and* its manifest carries a
/// `campaign_id` that parses as a UUID.
#[must_use]
pub fn campaign_of(submission: &Submission) -> Option<Uuid> {
    submission.parent_job_id.as_ref()?;
    submission
        .manifest
        .get("campaign_id")
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
}

/// Append a lifecycle event for a job's trial in the job's transaction.
async fn trial_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job_id: &str,
    event: impl FnOnce(ledger::TrialState) -> Vec<ledger::TrialEvent>,
) -> Result<(), JobStoreError> {
    let row: Option<(Option<Uuid>, Uuid)> =
        sqlx::query_as("SELECT trial_id, user_id FROM jobs WHERE job_id=$1")
            .bind(job_id)
            .fetch_optional(&mut **tx)
            .await?;
    let Some((Some(trial_id), user_id)) = row else {
        return Ok(());
    };
    let tenant = user_id.to_string();
    let Some(state) = ledger::pg::state_in_tx(tx, &tenant, trial_id).await? else {
        return Err(JobStoreError::Invalid(format!(
            "job {job_id} references unknown trial {trial_id}"
        )));
    };
    if state.is_terminal() {
        return Ok(());
    }
    for ev in event(state) {
        ledger::pg::append_event_in_tx(tx, &tenant, trial_id, &ev).await?;
    }
    Ok(())
}

/// A terminal failure from wherever the trial currently is. A lost worker is a
/// preemption that was never recovered: right-censored, not failed.
fn fail_events(
    state: ledger::TrialState,
    reason: ledger::TerminalReason,
    detail: String,
) -> Vec<ledger::TrialEvent> {
    let mut evs = Vec::new();
    if reason == ledger::TerminalReason::PreemptedAbandoned && state == ledger::TrialState::Running
    {
        evs.push(ledger::TrialEvent::to(ledger::TrialState::Preempted));
    }
    evs.push(ledger::TrialEvent::failed(reason, detail));
    evs
}

/// Every `Trainer`-class manifest declares its own GPU ceiling (SPEC §8,
/// ADR-P2-05, AT-66).
///
/// There is **no platform default**, and that is the whole mechanism. A default
/// is a number nobody chose about somebody's money and somebody else's GPU, and
/// once one exists every submission inherits it silently. A preset may *suggest*
/// a value the user confirms (checklist 5.7); nothing may supply one.
///
/// The ceiling is checked here, at submission, rather than in the worker alone,
/// because a job that was never allowed to be queued cannot exhaust anything.
/// The worker enforces the same number at the other end by killing at the limit
/// and settling `budget_exceeded`, which INV-17 censors `right_budget`.
fn require_gpu_budget(req: &Submission) -> Result<(), JobStoreError> {
    if req.kind.worker_class() != WorkerClass::Trainer {
        return Ok(());
    }
    let declared = req
        .manifest
        .get("max_gpu_hours")
        .and_then(serde_json::Value::as_f64);
    match declared {
        Some(h) if h.is_finite() && h > 0.0 => Ok(()),
        _ => Err(JobStoreError::MaxGpuHoursRequired(req.kind.to_string())),
    }
}

// -- Budget admission (COMP-005 §7, JB-08) -----------------------------------

/// What a project is allowed to spend. `None` on an axis means no limit.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProjectBudget {
    pub compute_s: Option<f64>,
    pub usd_day: Option<f64>,
    pub usd_total: Option<f64>,
}

/// What a project has spent or has committed to spend: `actual` where a job has
/// finished, its `estimate` where it has not.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProjectSpend {
    pub compute_s: f64,
    pub usd_day: f64,
    pub usd_total: f64,
}

/// The admission rule, pure so it can be tested without a database.
///
/// A submission is refused when its estimate would take any limited axis past
/// its limit. It is also refused when the axis is already **spent** (at or over
/// its limit) even if the submission carries no estimate for that axis: an
/// exhausted budget has no room for work whose cost nobody can state. A job
/// with no estimate on a still-open axis is admitted; its `actual` is charged
/// when it finishes.
///
/// # Errors
/// `BudgetExhausted` naming the axis, what was used, what was asked, the limit.
pub fn check_budget(
    budget: &ProjectBudget,
    spent: &ProjectSpend,
    estimate: Option<&Cost>,
) -> Result<(), JobStoreError> {
    let axes = [
        (
            "compute_s",
            budget.compute_s,
            spent.compute_s,
            estimate.and_then(|c| c.compute_s),
        ),
        (
            "usd_day",
            budget.usd_day,
            spent.usd_day,
            estimate.and_then(|c| c.cost_usd),
        ),
        (
            "usd_total",
            budget.usd_total,
            spent.usd_total,
            estimate.and_then(|c| c.cost_usd),
        ),
    ];
    for (axis, limit, used, asked) in axes {
        let Some(limit) = limit else { continue };
        let asked_amount = asked.unwrap_or(0.0);
        if used >= limit || used + asked_amount > limit {
            return Err(JobStoreError::BudgetExhausted(format!(
                "{axis}: {used:.2} used + {asked_amount:.2} estimated exceeds the project's \
                 limit of {limit:.2}; raise the project budget or wait for the day to roll over"
            )));
        }
    }
    Ok(())
}

/// Checks the project's budget inside the submission transaction.
///
/// A transaction-scoped advisory lock keyed on the project serialises
/// concurrent submissions, so two submissions cannot each see room that only
/// one of them has. System jobs (no project) and projects with no budget are
/// not limited.
async fn admit_budget(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    req: &Submission,
    estimate: Option<&Cost>,
) -> Result<(), JobStoreError> {
    let Some(project_id) = req.project_id else {
        return Ok(());
    };
    let Some(row) = sqlx::query(
        "SELECT budget_compute_s::float8 AS compute_s, \
                budget_usd_day::float8 AS usd_day, budget_usd_total::float8 AS usd_total \
           FROM research_projects WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(());
    };
    let budget = ProjectBudget {
        compute_s: row.get("compute_s"),
        usd_day: row.get("usd_day"),
        usd_total: row.get("usd_total"),
    };
    if budget == ProjectBudget::default() {
        return Ok(());
    }

    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("jobs_budget:{project_id}"))
        .execute(&mut **tx)
        .await?;

    let spent = sqlx::query(
        "SELECT \
           COALESCE(SUM(COALESCE((actual->>'compute_s')::float8, (estimate->>'compute_s')::float8, 0)), 0) AS compute_s, \
           COALESCE(SUM(COALESCE((actual->>'cost_usd')::float8, (estimate->>'cost_usd')::float8, 0)), 0) AS usd_total, \
           COALESCE(SUM(COALESCE((actual->>'cost_usd')::float8, (estimate->>'cost_usd')::float8, 0)) \
                    FILTER (WHERE created_at >= date_trunc('day', now())), 0) AS usd_day \
           FROM jobs WHERE project_id = $1 AND state <> 'cancelled'",
    )
    .bind(project_id)
    .fetch_one(&mut **tx)
    .await?;
    let spent = ProjectSpend {
        compute_s: spent.get("compute_s"),
        usd_day: spent.get("usd_day"),
        usd_total: spent.get("usd_total"),
    };
    check_budget(&budget, &spent, estimate)
}

pub struct JobStore {
    pool: PgPool,
    counter: std::sync::Arc<dyn TrialCounter>,
    /// Per-kind cost predictors, filled by `WorkerPool::register`, so a
    /// submission that carries no estimate still gets one for budget admission.
    estimators: std::sync::RwLock<
        std::collections::HashMap<JobKind, std::sync::Arc<dyn crate::worker::Worker>>,
    >,
}

impl JobStore {
    pub fn new(pool: PgPool, counter: std::sync::Arc<dyn TrialCounter>) -> Self {
        Self {
            pool,
            counter,
            estimators: std::sync::RwLock::default(),
        }
    }

    /// Registers the worker whose `estimate` predicts this kind's cost at
    /// submission (JB-08). Called by `WorkerPool::register`.
    pub fn register_estimator(&self, worker: std::sync::Arc<dyn crate::worker::Worker>) {
        if let Ok(mut map) = self.estimators.write() {
            map.insert(worker.kind(), worker);
        }
    }

    /// The estimate a submission is admitted on: the caller's own, else the
    /// registered worker's prediction. A prediction with no figure in it is
    /// `None`, not a zero: an unmeasured cost must not read as a free job.
    fn estimate_for(&self, req: &Submission) -> Option<Cost> {
        let cost = req.estimate.clone().or_else(|| {
            let map = self.estimators.read().ok()?;
            map.get(&req.kind).map(|w| w.estimate(&req.manifest))
        })?;
        (cost.compute_s.is_some() || cost.gpu_s.is_some() || cost.cost_usd.is_some())
            .then_some(cost)
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Submits a job, deduplicating by `(project_id, manifest_hash)` (JB-02).
    ///
    /// For trial-counted kinds the trial is registered in the same transaction
    /// (JB-03, COMP-005 §10). A deduplicated submission spends no compute but is
    /// still a look: it registers its own trial and settles it `deduplicated`,
    /// naming the original job's trial (SPEC §9, §12.4).
    /// Submit a child job at `(parent_job_id, member_index)`, returning the one
    /// that is already there if it exists.
    ///
    /// This is the exactly-once side effect the campaign driver is built on
    /// (ADR-P2-04). A driver killed between creating a child and recording that
    /// it did so re-derives the same `(parent, member_index)` from the fold and
    /// gets the same child back, so the phase runs once however many times the
    /// driver restarts. Ordinary `submit` cannot do this: children are
    /// deliberately *not* deduplicated by manifest hash — two members of a sweep
    /// may legitimately carry the same manifest — so the position is the
    /// identity, and `jobs_member` is the unique index that decides the race.
    ///
    /// # Errors
    /// A submission with no parent or no member index, and backend failures.
    pub async fn submit_child_once(&self, req: Submission) -> Result<Submitted, JobStoreError> {
        let (Some(parent), Some(member)) = (req.parent_job_id.clone(), req.member_index) else {
            return Err(JobStoreError::Invalid(
                "submit_child_once needs both a parent job and a member index: the pair is the \
                 identity that makes the submission idempotent"
                    .into(),
            ));
        };

        if let Some(existing) = self.child_at(&parent, member).await? {
            return Ok(existing);
        }
        match self.submit(req).await {
            Ok(s) => Ok(s),
            // Lost the race against another driver holding the same lease-less
            // view. Whoever won created the child this one would have; take it.
            Err(e) => match self.child_at(&parent, member).await? {
                Some(existing) => Ok(existing),
                None => Err(e),
            },
        }
    }

    async fn child_at(
        &self,
        parent: &str,
        member: i32,
    ) -> Result<Option<Submitted>, JobStoreError> {
        let row = sqlx::query(
            "SELECT job_id, state FROM jobs WHERE parent_job_id = $1 AND member_index = $2",
        )
        .bind(parent)
        .bind(member)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| {
            let state: String = r.get("state");
            Submitted {
                job_id: r.get("job_id"),
                deduplicated: true,
                state: JobState::parse(&state).unwrap_or(JobState::Queued),
            }
        }))
    }

    pub async fn submit(&self, req: Submission) -> Result<Submitted, JobStoreError> {
        let hash = req.manifest_hash();
        let is_child = req.parent_job_id.is_some();

        if req.kind.counts_trial() && req.experiment_id.is_none() {
            return Err(JobStoreError::ExperimentRequired(req.kind.to_string()));
        }
        require_gpu_budget(&req)?;

        // Existing top-level job with this manifest? Return it, and record the look.
        if !is_child {
            if let Some(row) = sqlx::query(
                "SELECT job_id, state, trial_id FROM jobs \
                 WHERE coalesce(project_id,'00000000-0000-0000-0000-000000000000'::uuid) \
                       = coalesce($1,'00000000-0000-0000-0000-000000000000'::uuid) \
                   AND manifest_hash = $2 AND parent_job_id IS NULL",
            )
            .bind(req.project_id)
            .bind(&hash)
            .fetch_optional(&self.pool)
            .await?
            {
                let state: String = row.get("state");
                let prior: Option<Uuid> = row.get("trial_id");
                if let (true, Some(prior)) = (req.kind.counts_trial(), prior) {
                    let mut tx = self.pool.begin().await?;
                    let job_id: String = row.get("job_id");
                    let trial_id = self.counter.register_trial(&mut tx, &req, &job_id).await?;
                    let tenant = req.user_id.to_string();
                    ledger::pg::append_event_in_tx(
                        &mut tx,
                        &tenant,
                        trial_id,
                        &ledger::TrialEvent::deduplicated(prior, format!("job:{job_id}")),
                    )
                    .await?;
                    tx.commit().await?;
                }
                return Ok(Submitted {
                    job_id: row.get("job_id"),
                    deduplicated: true,
                    state: JobState::parse(&state).unwrap_or(JobState::Queued),
                });
            }
        }

        let job_id = new_job_id();
        let queue = req.submitted_by.default_queue();
        let worker_class = req.kind.worker_class();
        let estimate_cost = self.estimate_for(&req);
        let estimate = estimate_cost
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?;

        let mut tx = self.pool.begin().await?;

        // JB-08: refuse before anything is registered or inserted, so a
        // refused submission counts no trial and leaves no row.
        admit_budget(&mut tx, &req, estimate_cost.as_ref()).await?;

        // Register first: the trial must exist before the job it authorizes.
        let trial_id = if req.kind.counts_trial() {
            let id = self.counter.register_trial(&mut tx, &req, &job_id).await?;
            ledger::pg::append_event_in_tx(
                &mut tx,
                &req.user_id.to_string(),
                id,
                &ledger::TrialEvent::to(ledger::TrialState::Queued),
            )
            .await?;
            Some(id)
        } else {
            None
        };

        sqlx::query(
            "INSERT INTO jobs (job_id, kind, project_id, user_id, session_id, submitted_by, \
                               experiment_id, parent_job_id, member_index, queue, worker_class, \
                               priority, manifest, manifest_hash, state, estimate, trial_id) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,'queued',$15,$16)",
        )
        .bind(&job_id)
        .bind(req.kind.as_str())
        .bind(req.project_id)
        .bind(req.user_id)
        .bind(req.session_id)
        .bind(req.submitted_by.as_str())
        .bind(&req.experiment_id)
        .bind(&req.parent_job_id)
        .bind(req.member_index)
        .bind(queue.as_str())
        .bind(worker_class.as_str())
        .bind(req.priority)
        .bind(&req.manifest)
        .bind(&hash)
        .bind(&estimate)
        .bind(trial_id)
        .execute(&mut *tx)
        .await?;

        append_event(
            &mut tx,
            &job_id,
            "state",
            serde_json::json!({"state": "queued"}),
        )
        .await?;
        tx.commit().await?;

        Ok(Submitted {
            job_id,
            deduplicated: false,
            state: JobState::Queued,
        })
    }

    /// Claims the next runnable job for a worker class (COMP-005 §5, §7).
    ///
    /// Ordering is queue weight, then priority, then oldest-first. `FOR UPDATE SKIP
    /// LOCKED` lets many workers claim concurrently without blocking one another.
    pub async fn claim(
        &self,
        worker_class: WorkerClass,
        worker_id: &str,
    ) -> Result<Option<Job>, JobStoreError> {
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query(
            "SELECT job_id FROM jobs \
              WHERE state = 'queued' AND worker_class = $1 AND cancel_requested = false \
              ORDER BY CASE queue WHEN 'human' THEN 0 WHEN 'agent' THEN 1 ELSE 2 END, \
                       priority DESC, created_at \
              FOR UPDATE SKIP LOCKED LIMIT 1",
        )
        .bind(worker_class.as_str())
        .fetch_optional(&mut *tx)
        .await?;

        let Some(row) = row else {
            tx.commit().await?;
            return Ok(None);
        };
        let job_id: String = row.get("job_id");
        let expires = Utc::now() + Duration::seconds(LEASE_SECONDS);

        sqlx::query(
            "UPDATE jobs SET state='running', lease_owner=$2, lease_expires_at=$3, \
                             heartbeat_at=now(), started_at=coalesce(started_at, now()), \
                             attempt_started_at=now() \
             WHERE job_id=$1",
        )
        .bind(&job_id)
        .bind(worker_id)
        .bind(expires)
        .execute(&mut *tx)
        .await?;

        trial_event(&mut tx, &job_id, |state| match state {
            ledger::TrialState::Preempted => vec![
                ledger::TrialEvent::to(ledger::TrialState::Recovering),
                ledger::TrialEvent::to(ledger::TrialState::Running),
            ],
            ledger::TrialState::Running => Vec::new(),
            _ => vec![ledger::TrialEvent::to(ledger::TrialState::Running)],
        })
        .await?;

        append_event(
            &mut tx,
            &job_id,
            "state",
            serde_json::json!({"state": "running", "worker": worker_id}),
        )
        .await?;
        tx.commit().await?;

        self.get(&job_id).await.map(Some)
    }

    /// Extends a lease. Returns false when the job was cancelled meanwhile, which is
    /// how a running worker learns to stop.
    pub async fn heartbeat(&self, job_id: &str, worker_id: &str) -> Result<bool, JobStoreError> {
        let expires = Utc::now() + Duration::seconds(LEASE_SECONDS);
        let row = sqlx::query(
            "UPDATE jobs SET heartbeat_at=now(), lease_expires_at=$3 \
             WHERE job_id=$1 AND lease_owner=$2 AND state='running' \
             RETURNING cancel_requested",
        )
        .bind(job_id)
        .bind(worker_id)
        .bind(expires)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => Ok(!r.get::<bool, _>("cancel_requested")),
            // Lease lost (expired and re-queued, or cancelled). Stop working.
            None => Ok(false),
        }
    }

    pub async fn report_progress(
        &self,
        job_id: &str,
        progress: &Progress,
    ) -> Result<(), JobStoreError> {
        let value = serde_json::to_value(progress)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE jobs SET progress=$2 WHERE job_id=$1 AND state='running'")
            .bind(job_id)
            .bind(&value)
            .execute(&mut *tx)
            .await?;
        append_event(&mut tx, job_id, "progress", value).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Store a mid-run checkpoint manifest for a job (SPEC §9, checklist 2.4).
    ///
    /// **Refuses a manifest that does not satisfy the contract**, by field name.
    /// A partial checkpoint is the classic silent-divergence bug — the run
    /// resumes, the numbers look fine, and the result is not the one the
    /// uninterrupted run would have produced — and the only place it can be
    /// caught is before it is stored. `plan_resume` treats a stored invalid
    /// checkpoint as `Abandon`, which is why none may get in.
    ///
    /// # Errors
    /// `Invalid("checkpoint_invalid: ...")` naming the missing or empty field.
    pub async fn record_checkpoint(
        &self,
        job_id: &str,
        manifest: &Value,
    ) -> Result<(), JobStoreError> {
        crate::checkpoint::CheckpointManifest::parse(manifest)
            .map_err(|e| JobStoreError::Invalid(format!("checkpoint_invalid: {e}")))?;
        let mut tx = self.pool.begin().await?;
        append_event(&mut tx, job_id, "checkpoint", manifest.clone()).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The newest checkpoint manifest a job recorded, if any.
    ///
    /// The newest, not the best: a later checkpoint is further along, and
    /// `plan_resume` is what judges whether it may be used.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn latest_checkpoint(&self, job_id: &str) -> Result<Option<Value>, JobStoreError> {
        let row: Option<(Value,)> = sqlx::query_as(
            "SELECT payload FROM job_events WHERE job_id = $1 AND kind = 'checkpoint'
              ORDER BY id DESC LIMIT 1",
        )
        .bind(job_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(v,)| v))
    }

    pub async fn complete(&self, job_id: &str, output: JobOutput) -> Result<(), JobStoreError> {
        let output = output.clamped();
        let actual = serde_json::to_value(&output.actual)?;
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "UPDATE jobs SET state='succeeded', result_summary=$2, result=$3, actual=$4, \
                             finished_at=now(), lease_owner=NULL, lease_expires_at=NULL \
             WHERE job_id=$1 AND state='running'",
        )
        .bind(job_id)
        .bind(&output.summary)
        .bind(&output.result)
        .bind(&actual)
        .execute(&mut *tx)
        .await?;

        charge_attempt(&mut tx, job_id, AttemptEnd::Now).await?;

        for (handle, role) in &output.artifacts {
            // A checkpoint written mid-run is a **partial** model, and nothing
            // downstream may read one as if it were a finished artifact. This is
            // the one place a job's outputs become its evidence, so it is the
            // place the distinction has to be a mechanism rather than a habit:
            // a `checkpoint` artifact may be attached under the `checkpoint`
            // role and no other. Resuming from a half-trained model is correct;
            // serving from one, scoring one, or citing one in a verdict is not,
            // and the two are the same bytes on disk.
            let stored_type: Option<String> =
                sqlx::query_scalar("SELECT type FROM artifacts WHERE handle = $1")
                    .bind(handle)
                    .fetch_optional(&mut *tx)
                    .await?;
            if stored_type.as_deref() == Some(CHECKPOINT_TYPE) && role != CHECKPOINT_ROLE {
                return Err(JobStoreError::Invalid(format!(
                    "artifact {handle} is a checkpoint and was attached to job {job_id} as \
                     `{role}`; a checkpoint is a partial model and may only be attached as \
                     `{CHECKPOINT_ROLE}`"
                )));
            }

            sqlx::query(
                "INSERT INTO job_artifacts (job_id, handle, role) VALUES ($1,$2,$3) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(job_id)
            .bind(handle)
            .bind(role)
            .execute(&mut *tx)
            .await?;
        }

        let run_ref = format!("job:{job_id}");
        trial_event(&mut tx, job_id, |state| {
            let mut evs = Vec::new();
            if state != ledger::TrialState::Running {
                evs.push(ledger::TrialEvent::to(ledger::TrialState::Running));
            }
            evs.push(ledger::TrialEvent::completed(run_ref, None));
            evs
        })
        .await?;

        append_event(
            &mut tx,
            job_id,
            "state",
            serde_json::json!({"state": "succeeded"}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Fails a job. Logic failures are terminal; infrastructure failures re-queue
    /// until `MAX_ATTEMPTS`, then fail as `lost_worker` (JB-07, COMP-005 §5).
    pub async fn fail(&self, job_id: &str, error: JobError) -> Result<JobState, JobStoreError> {
        let error = error.clamped();
        let retryable = error.retryable;
        let payload = serde_json::to_value(&error)?;
        let mut tx = self.pool.begin().await?;

        let attempts: i16 = sqlx::query("SELECT attempts FROM jobs WHERE job_id=$1")
            .bind(job_id)
            .fetch_one(&mut *tx)
            .await?
            .get("attempts");

        // Charged before the branch. An attempt that failed spent the time it
        // spent, and whether the job is about to be re-queued or ended has
        // nothing to do with it — a job that re-queues is exactly the case where
        // forgetting to charge hands the next attempt the whole ceiling again.
        charge_attempt(&mut tx, job_id, AttemptEnd::Now).await?;

        let state = if retryable && attempts + 1 < MAX_ATTEMPTS {
            sqlx::query(
                "UPDATE jobs SET state='queued', attempts=attempts+1, lease_owner=NULL, \
                                 lease_expires_at=NULL WHERE job_id=$1",
            )
            .bind(job_id)
            .execute(&mut *tx)
            .await?;
            trial_event(&mut tx, job_id, |state| match state {
                ledger::TrialState::Running => {
                    vec![ledger::TrialEvent::to(ledger::TrialState::Preempted)]
                }
                _ => Vec::new(),
            })
            .await?;
            JobState::Queued
        } else {
            sqlx::query(
                "UPDATE jobs SET state='failed', error=$2, attempts=attempts+1, \
                                 finished_at=now(), lease_owner=NULL, lease_expires_at=NULL \
                 WHERE job_id=$1",
            )
            .bind(job_id)
            .bind(&payload)
            .execute(&mut *tx)
            .await?;
            // The reason travels with the failure (ADR-P2-30). Nothing here reads
            // the message to guess how the trial ended.
            let reason = error.terminal;
            let detail = format!("{}: {}", error.code, error.fix.clone().unwrap_or_default());
            trial_event(&mut tx, job_id, |state| fail_events(state, reason, detail)).await?;
            JobState::Failed
        };

        append_event(
            &mut tx,
            job_id,
            "state",
            serde_json::json!({"state": state.as_str(), "error": payload}),
        )
        .await?;
        tx.commit().await?;
        Ok(state)
    }

    /// Requests cancellation. A queued job cancels immediately; a running one is
    /// signalled and stops at its next heartbeat. Children cancel with their parent.
    pub async fn cancel(&self, job_id: &str) -> Result<JobState, JobStoreError> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("UPDATE jobs SET cancel_requested=true WHERE job_id=$1 OR parent_job_id=$1")
            .bind(job_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query(
            "UPDATE jobs SET state='cancelled', finished_at=now(), lease_owner=NULL, \
                             lease_expires_at=NULL \
             WHERE (job_id=$1 OR parent_job_id=$1) AND state IN ('queued','paused')",
        )
        .bind(job_id)
        .execute(&mut *tx)
        .await?;

        let state: String = sqlx::query("SELECT state FROM jobs WHERE job_id=$1")
            .bind(job_id)
            .fetch_one(&mut *tx)
            .await?
            .get("state");
        if state == "cancelled" {
            let children: Vec<String> = sqlx::query_scalar("SELECT job_id FROM jobs WHERE (job_id=$1 OR parent_job_id=$1) AND state='cancelled'")
                .bind(job_id)
                .fetch_all(&mut *tx)
                .await?;
            for child in children {
                trial_event(&mut tx, &child, |s| {
                    fail_events(
                        s,
                        ledger::TerminalReason::Cancelled,
                        "cancelled before it ran".into(),
                    )
                })
                .await?;
            }
        }

        append_event(
            &mut tx,
            job_id,
            "state",
            serde_json::json!({"state": state, "cancel_requested": true}),
        )
        .await?;
        tx.commit().await?;
        Ok(JobState::parse(&state).unwrap_or(JobState::Cancelled))
    }

    /// Re-queues jobs whose lease expired (JB-04).
    ///
    /// Called periodically by the platform. This is what makes a job survive a
    /// worker that died without reporting: the lease simply stops being renewed.
    pub async fn reap_expired_leases(&self) -> Result<usize, JobStoreError> {
        let rows = sqlx::query(
            "SELECT job_id, attempts FROM jobs \
             WHERE state='running' AND lease_expires_at < now()",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut reaped = 0usize;
        for row in rows {
            let job_id: String = row.get("job_id");
            let attempts: i16 = row.get("attempts");
            let mut tx = self.pool.begin().await?;
            // The attempt nobody reported on. This is the whole reason the spend
            // is charged by the database: the worker that was holding this lease
            // is gone, it will never tell anyone what it used, and if that time
            // went uncharged then "die and be re-queued" would be a way to spend
            // more than `max_gpu_hours`. Charged to the lease expiry rather than
            // to now, because that is the last instant the slot was held and the
            // reaper's own latency is not the job's to pay for.
            charge_attempt(&mut tx, &job_id, AttemptEnd::LeaseExpiry).await?;
            if attempts + 1 < MAX_ATTEMPTS {
                sqlx::query(
                    "UPDATE jobs SET state='queued', attempts=attempts+1, lease_owner=NULL, \
                                     lease_expires_at=NULL \
                     WHERE job_id=$1 AND state='running'",
                )
                .bind(&job_id)
                .execute(&mut *tx)
                .await?;
                trial_event(&mut tx, &job_id, |state| match state {
                    ledger::TrialState::Running => {
                        vec![ledger::TrialEvent::to(ledger::TrialState::Preempted)]
                    }
                    _ => Vec::new(),
                })
                .await?;
            } else {
                let error = serde_json::to_value(JobError {
                    code: "lost_worker".into(),
                    // A worker that stopped reporting on its last attempt is a
                    // preemption nobody recovered: right-censored, never `failed`.
                    terminal: ledger::TerminalReason::PreemptedAbandoned,
                    field: None,
                    rule: None,
                    fix: Some("the worker stopped reporting; resubmit if still wanted".into()),
                    detail_ref: None,
                    retryable: false,
                })?;
                sqlx::query(
                    "UPDATE jobs SET state='failed', error=$2, finished_at=now(), \
                                     lease_owner=NULL, lease_expires_at=NULL \
                     WHERE job_id=$1 AND state='running'",
                )
                .bind(&job_id)
                .bind(&error)
                .execute(&mut *tx)
                .await?;
                trial_event(&mut tx, &job_id, |state| {
                    fail_events(
                        state,
                        ledger::TerminalReason::PreemptedAbandoned,
                        "lost_worker: lease expired on the final attempt".into(),
                    )
                })
                .await?;
            }
            tx.commit().await?;
            reaped += 1;
        }
        Ok(reaped)
    }

    pub async fn get(&self, job_id: &str) -> Result<Job, JobStoreError> {
        let row = sqlx::query("SELECT * FROM jobs WHERE job_id=$1")
            .bind(job_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| JobStoreError::NotFound(job_id.to_string()))?;
        row_to_job(&row)
    }

    pub async fn list(
        &self,
        project_id: Option<Uuid>,
        state: Option<JobState>,
        kind: Option<JobKind>,
        limit: i64,
    ) -> Result<Vec<Job>, JobStoreError> {
        let rows = sqlx::query(
            "SELECT * FROM jobs \
             WHERE ($1::uuid IS NULL OR project_id = $1) \
               AND ($2::text IS NULL OR state = $2) \
               AND ($3::text IS NULL OR kind = $3) \
             ORDER BY created_at DESC LIMIT $4",
        )
        .bind(project_id)
        .bind(state.map(|s| s.as_str()))
        .bind(kind.map(|k| k.as_str()))
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_job).collect()
    }

    /// Events after `after_id`, for SSE resumption (JB-06).
    pub async fn events_after(
        &self,
        after_id: i64,
        project_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<(i64, String, String, Value)>, JobStoreError> {
        let rows = sqlx::query(
            "SELECT e.id, e.job_id, e.kind, e.payload FROM job_events e \
             JOIN jobs j ON j.job_id = e.job_id \
             WHERE e.id > $1 AND ($2::uuid IS NULL OR j.project_id = $2) \
             ORDER BY e.id LIMIT $3",
        )
        .bind(after_id)
        .bind(project_id)
        .bind(limit.clamp(1, 1000))
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get::<i64, _>("id"),
                    r.get::<String, _>("job_id"),
                    r.get::<String, _>("kind"),
                    r.get::<Value, _>("payload"),
                )
            })
            .collect())
    }

    /// Appends to the exploration ledger (JB-11, D-13).
    pub async fn log_exploration(&self, entry: ExplorationEntry) -> Result<i64, JobStoreError> {
        let row = sqlx::query(
            "INSERT INTO exploration_ledger \
               (project_id, user_id, session_id, source, instruments, timeframe, \
                window_start, window_end, variables, description, handle) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING id",
        )
        .bind(entry.project_id)
        .bind(entry.user_id)
        .bind(entry.session_id)
        .bind(entry.source)
        .bind(&entry.instruments)
        .bind(&entry.timeframe)
        .bind(entry.window_start)
        .bind(entry.window_end)
        .bind(&entry.variables)
        .bind(&entry.description)
        .bind(&entry.handle)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get("id"))
    }

    pub async fn exploration_count(&self, project_id: Uuid) -> Result<i64, JobStoreError> {
        let row = sqlx::query("SELECT count(*) AS n FROM exploration_ledger WHERE project_id=$1")
            .bind(project_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get("n"))
    }
}

/// One exploration-ledger entry.
#[derive(Debug, Clone)]
pub struct ExplorationEntry {
    pub project_id: Uuid,
    pub user_id: Uuid,
    pub session_id: Option<Uuid>,
    /// `data_api` | `job` | `desk`
    pub source: &'static str,
    pub instruments: Vec<String>,
    pub timeframe: Option<String>,
    pub window_start: Option<DateTime<Utc>>,
    pub window_end: Option<DateTime<Utc>>,
    pub variables: Vec<String>,
    pub description: String,
    pub handle: Option<String>,
}

async fn append_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job_id: &str,
    kind: &str,
    payload: Value,
) -> Result<(), JobStoreError> {
    sqlx::query("INSERT INTO job_events (job_id, kind, payload) VALUES ($1,$2,$3)")
        .bind(job_id)
        .bind(kind)
        .bind(payload)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn row_to_job(row: &sqlx::postgres::PgRow) -> Result<Job, JobStoreError> {
    let kind: String = row.get("kind");
    let state: String = row.get("state");
    let queue: String = row.get("queue");
    let worker_class: String = row.get("worker_class");
    let submitted_by: String = row.get("submitted_by");
    Ok(Job {
        trial_id: row.get("trial_id"),
        job_id: row.get("job_id"),
        kind: JobKind::parse(&kind)
            .ok_or_else(|| JobStoreError::Invalid(format!("unknown kind {kind}")))?,
        project_id: row.get("project_id"),
        user_id: row.get("user_id"),
        session_id: row.get("session_id"),
        submitted_by: SubmittedBy::parse(&submitted_by).unwrap_or(SubmittedBy::System),
        experiment_id: row.get("experiment_id"),
        parent_job_id: row.get("parent_job_id"),
        queue: Queue::parse(&queue).unwrap_or(Queue::System),
        worker_class: WorkerClass::parse(&worker_class).unwrap_or(WorkerClass::Research),
        priority: row.get("priority"),
        manifest: row.get("manifest"),
        manifest_hash: row.get("manifest_hash"),
        state: JobState::parse(&state)
            .ok_or_else(|| JobStoreError::Invalid(format!("unknown state {state}")))?,
        progress: row.get("progress"),
        result_summary: row.get("result_summary"),
        result: row.get("result"),
        error: row.get("error"),
        attempts: row.get("attempts"),
        cancel_requested: row.get("cancel_requested"),
        created_at: row.get("created_at"),
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        gpu_seconds_spent: row.get("gpu_seconds_spent"),
    })
}

/// When an attempt is considered to have ended, for charging purposes.
///
/// The distinction exists because the two ways an attempt ends are observed at
/// different moments. A worker that reported its own ending is charged to now; a
/// worker that stopped answering is charged to the moment its lease ran out,
/// which is the last instant it was holding the slot and the last instant anyone
/// can say it was not working. Charging a reaped attempt to `now()` instead would
/// bill it for however long the reaper took to notice.
#[derive(Clone, Copy, Debug)]
enum AttemptEnd {
    Now,
    LeaseExpiry,
}

/// Add this attempt's wall-clock to the job's running total (SPEC §8, AT-66).
///
/// Idempotent by construction: it clears `attempt_started_at`, so charging twice
/// charges once. That matters because `fail` can be reached for a job the reaper
/// has already handled.
async fn charge_attempt(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    job_id: &str,
    end: AttemptEnd,
) -> Result<(), JobStoreError> {
    let until = match end {
        AttemptEnd::Now => "now()",
        // `lease_expires_at` can be NULL if the lease was cleared already; then
        // the coalesce falls back to the start, charging zero rather than
        // charging from the epoch.
        AttemptEnd::LeaseExpiry => "coalesce(lease_expires_at, attempt_started_at)",
    };
    let sql = format!(
        "UPDATE jobs \
            SET gpu_seconds_spent = gpu_seconds_spent \
                  + greatest(0, extract(epoch from ({until} - attempt_started_at))), \
                attempt_started_at = NULL \
          WHERE job_id = $1 AND attempt_started_at IS NOT NULL"
    );
    sqlx::query(&sql).bind(job_id).execute(&mut **tx).await?;
    Ok(())
}

/// A sortable, time-prefixed job id (`job_` + 26-char ULID-style string).
///
/// Monotonic by creation time so that a plain `ORDER BY job_id` matches submission
/// order, which makes debugging a queue far easier than random UUIDs would.
pub fn new_job_id() -> String {
    const ENCODING: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let ms = Utc::now().timestamp_millis() as u128;
    let random = Uuid::new_v4().as_u128();
    // 48 bits of timestamp, 80 bits of randomness — the ULID layout.
    let value = (ms << 80) | (random >> 48);
    let mut out = [0u8; 26];
    let mut v = value;
    for slot in out.iter_mut().rev() {
        *slot = ENCODING[(v & 0x1f) as usize];
        v >>= 5;
    }
    format!("job_{}", std::str::from_utf8(&out).expect("ascii"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_ids_are_prefixed_and_fixed_width() {
        let id = new_job_id();
        assert!(id.starts_with("job_"));
        assert_eq!(id.len(), 4 + 26);
    }

    #[test]
    fn job_ids_sort_by_creation_time() {
        // The timestamp occupies the high bits, so lexicographic order over the
        // base-32 text equals chronological order. A queue listing sorted by id is
        // then also sorted by age, which is what an operator expects.
        let first = new_job_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = new_job_id();
        assert!(first < second, "{first} should sort before {second}");
    }

    #[test]
    fn job_ids_are_unique_within_a_millisecond() {
        let ids: std::collections::HashSet<String> = (0..1000).map(|_| new_job_id()).collect();
        assert_eq!(ids.len(), 1000, "collision in 1000 ids");
    }

    #[test]
    fn submission_hash_ignores_fields_that_are_not_identity() {
        let user = Uuid::new_v4();
        let mut a = Submission::new(JobKind::Backtest, user, serde_json::json!({"s": 1}));
        a.priority = 1;
        let mut b = Submission::new(
            JobKind::Backtest,
            Uuid::new_v4(),
            serde_json::json!({"s": 1}),
        );
        b.priority = 9;
        // Priority and submitter are scheduling concerns, not identity: the same work
        // asked for twice at different priorities is still the same work.
        assert_eq!(a.manifest_hash(), b.manifest_hash());
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    fn trainer(manifest: Value) -> Submission {
        Submission::new(JobKind::Train, Uuid::nil(), manifest)
    }

    /// AT-66 ⛔ — a `Trainer`-class manifest without `max_gpu_hours` is refused,
    /// and there is no default that would let it through.
    #[test]
    fn at66_a_trainer_manifest_must_declare_its_gpu_ceiling() {
        for missing in [
            serde_json::json!({}),
            serde_json::json!({ "max_gpu_hours": null }),
            serde_json::json!({ "max_gpu_hours": 0 }),
            serde_json::json!({ "max_gpu_hours": -1.0 }),
            serde_json::json!({ "max_gpu_hours": "lots" }),
        ] {
            let err = require_gpu_budget(&trainer(missing.clone())).unwrap_err();
            assert!(
                matches!(err, JobStoreError::MaxGpuHoursRequired(_)),
                "{missing} should be refused, got {err}"
            );
        }
        require_gpu_budget(&trainer(serde_json::json!({ "max_gpu_hours": 2.5 })))
            .expect("a declared ceiling is accepted");
    }

    /// Every kind the trainer pool runs is covered, and no other kind is burdened
    /// with a GPU ceiling it has no use for.
    #[test]
    fn the_requirement_follows_the_worker_class_not_a_list() {
        for kind in JobKind::ALL {
            let sub = Submission::new(*kind, Uuid::nil(), serde_json::json!({}));
            let refused = require_gpu_budget(&sub).is_err();
            assert_eq!(
                refused,
                kind.worker_class() == WorkerClass::Trainer,
                "{kind} is on the {:?} pool",
                kind.worker_class()
            );
        }
    }

    /// The ceiling exists to become `budget_exceeded`, which INV-17 censors
    /// `right_budget` — a trial the platform stopped, not one that failed.
    #[test]
    fn exceeding_the_budget_is_censored_not_failed() {
        let e = JobError::terminal(
            ledger::TerminalReason::BudgetExceeded,
            "budget_exceeded",
            "stopped at the limit",
        );
        assert_eq!(e.censoring(), ledger::Censoring::RightBudget);
        assert_ne!(e.censoring(), ledger::Censoring::Failed);
    }
}

#[cfg(test)]
mod campaign_count_tests {
    use super::*;

    fn child(manifest: Value, parent: Option<&str>) -> Submission {
        let mut s = Submission::new(JobKind::Study, Uuid::nil(), manifest);
        s.parent_job_id = parent.map(str::to_string);
        s
    }

    /// The budget rule reads `trial.campaign_id`; a child that does not set it
    /// is counted in N_eff and invisible to `BUDGET_EXHAUSTED`.
    #[test]
    fn a_campaign_child_is_counted_against_its_campaign() {
        let id = Uuid::new_v4();
        let sub = child(
            serde_json::json!({ "campaign_id": id.to_string() }),
            Some("job_x"),
        );
        assert_eq!(campaign_of(&sub), Some(id));
    }

    /// A top-level job that merely carries the field is not campaign-driven, and
    /// a malformed id is not a campaign.
    #[test]
    fn only_a_child_naming_a_real_campaign_qualifies() {
        let id = Uuid::new_v4();
        assert_eq!(
            campaign_of(&child(
                serde_json::json!({ "campaign_id": id.to_string() }),
                None
            )),
            None
        );
        assert_eq!(
            campaign_of(&child(
                serde_json::json!({ "campaign_id": "nope" }),
                Some("job_x")
            )),
            None
        );
        assert_eq!(
            campaign_of(&child(serde_json::json!({}), Some("job_x"))),
            None
        );
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;

    fn cost(compute_s: Option<f64>, usd: Option<f64>) -> Cost {
        Cost {
            compute_s,
            gpu_s: None,
            cost_usd: usd,
        }
    }

    #[test]
    fn no_limit_means_no_refusal() {
        let b = ProjectBudget::default();
        let spent = ProjectSpend {
            compute_s: 1e9,
            usd_day: 1e9,
            usd_total: 1e9,
        };
        assert!(check_budget(&b, &spent, Some(&cost(Some(1e9), Some(1e9)))).is_ok());
    }

    #[test]
    fn an_estimate_past_the_limit_is_refused_naming_the_axis() {
        let b = ProjectBudget {
            compute_s: Some(100.0),
            ..Default::default()
        };
        let spent = ProjectSpend {
            compute_s: 70.0,
            ..Default::default()
        };
        assert!(check_budget(&b, &spent, Some(&cost(Some(29.0), None))).is_ok());
        let err = check_budget(&b, &spent, Some(&cost(Some(30.5), None))).unwrap_err();
        match err {
            JobStoreError::BudgetExhausted(m) => assert!(m.starts_with("compute_s:"), "{m}"),
            other => panic!("wrong error: {other}"),
        }
    }

    #[test]
    fn a_spent_budget_refuses_even_an_unestimated_job() {
        let b = ProjectBudget {
            usd_total: Some(10.0),
            ..Default::default()
        };
        let spent = ProjectSpend {
            usd_total: 10.0,
            ..Default::default()
        };
        assert!(check_budget(&b, &spent, None).is_err());
        // ...but an open budget admits a job nobody could price.
        let open = ProjectSpend {
            usd_total: 9.99,
            ..Default::default()
        };
        assert!(check_budget(&b, &open, None).is_ok());
    }

    #[test]
    fn the_daily_cap_binds_before_the_total() {
        let b = ProjectBudget {
            usd_day: Some(5.0),
            usd_total: Some(500.0),
            ..Default::default()
        };
        let spent = ProjectSpend {
            usd_day: 4.0,
            usd_total: 100.0,
            ..Default::default()
        };
        let err = check_budget(&b, &spent, Some(&cost(None, Some(2.0)))).unwrap_err();
        assert!(err.to_string().contains("usd_day"), "{err}");
    }
}
