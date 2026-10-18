//! The driver half of a sub-run (AGENT-010 §9, AGENT-013 §8.5; Set S task
//! S-10.11).
//!
//! `harness::subrun` owns the rule — which projection a role is meaningful
//! under, and what a dispatch must satisfy before it happens. This owns the IO:
//! writing the pinned brief down at DEFINE, claiming a position in the
//! campaign's fold exactly once, and recording what came back.
//!
//! # Why the brief is written at DEFINE and read at HYPOTHESIZE
//!
//! The parent Research run is suspended on `Effect::Await` for the whole life of
//! its campaign. Its context store lives in the loop that is no longer running,
//! so the only way to project it at dispatch time would be to replay the
//! parent's entire effect log — once per generation, inside a job worker, to
//! arrive at something the parent already had in hand when it called
//! `define_campaign`.
//!
//! Writing it once also buys the property fan-out actually needs: every worker
//! reads the same bytes, so a provider cache sees one prefix instead of one per
//! worker. [`CampaignBrief::digest`] is what makes that checkable after the fact
//! rather than merely intended.
//!
//! # Claiming before dispatching
//!
//! [`claim`] writes the sub-run row *before* the job exists, keyed on
//! `(campaign_id, phase, seq)`. That is the opposite order from
//! `campaign_driver::advance`, deliberately: the driver creates its child first
//! because a child nothing points at is recoverable, and here the row is what
//! *carries* the job id, so a second driver arriving between the two finds the
//! claim and waits rather than dispatching a second worker against the same
//! generation. Two workers at one position would be two proposals for one
//! `propose` decision, and the decision log has room for one `chosen`.

use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use harness::context::projection::{self, Projection};
use harness::context::store::RunStore;
use harness::subrun::{BudgetLease, Delegation, DelegationError, LeaseError, Remaining, Role};

/// sha256 of the brief's bytes, in the shape the rest of the platform writes
/// digests.
#[must_use]
pub fn digest(text: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(text.as_bytes())))
}

/// The pinned projection a campaign's workers all read.
#[derive(Debug, Clone)]
pub struct CampaignBrief {
    pub campaign_id: Uuid,
    pub run_id: Uuid,
    pub projection: String,
    pub brief: String,
    pub digest: String,
    pub shown: Vec<String>,
    pub withheld: i32,
}

/// Build the campaign's brief from the parent's store.
///
/// The projection is [`projection::campaign_worker`], which derives the pinned
/// set from the store by kind rather than taking a list — so the parent's
/// reasoning is excluded because it is not there, not because a caller
/// remembered to leave it out.
#[must_use]
pub fn assemble(
    parent: &RunStore,
    query: &harness::context::select::Query,
    budget: u32,
    policy: &harness::context::policy::ContextPolicy,
    manifest_tokens: u32,
) -> projection::Brief {
    let projected = projection::project(parent, &projection::campaign_worker(parent));
    projection::brief(&projected, query, budget, policy, manifest_tokens)
}

/// A brief as it travels: the bytes and the account, with no borrow of the store
/// it came from.
///
/// [`projection::Brief`] holds `RecordId`s and a selection report, which are the
/// right shape inside the process that assembled it and the wrong shape crossing
/// an HTTP boundary. The driver assembles, converts here, and the route writes —
/// so the ids that reach the row are the ids that were rendered, rather than a
/// second list built from whatever the route happened to have.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PinnedBrief {
    pub text: String,
    pub projection: String,
    pub shown: Vec<String>,
    pub withheld: usize,
}

impl PinnedBrief {
    #[must_use]
    pub fn of(b: &projection::Brief) -> Self {
        Self {
            text: b.text.clone(),
            projection: b.projection.to_string(),
            shown: b.shown.iter().map(ToString::to_string).collect(),
            withheld: b.withheld.len(),
        }
    }
}

/// Why a campaign's brief could not be written.
#[derive(Debug, thiserror::Error)]
pub enum BriefError {
    #[error("the sub-run this brief is for would be refused: {0}")]
    Refused(#[from] DelegationError),
    #[error("writing the campaign brief: {0}")]
    Db(#[from] sqlx::Error),
    /// The parent has nothing left to give a worker. Refused rather than
    /// dispatched on a budget that does not exist (S-2.12).
    #[error("the parent run cannot fund this sub-run: {0}")]
    Unfunded(#[from] LeaseError),
}

/// Write the campaign's row: the registration it came from, the job driving it,
/// and the brief its workers read.
///
/// The [`Delegation::check`] runs here rather than at dispatch, which is the
/// earlier of the two places it can: a brief written under the wrong projection
/// would be a brief every generation then reads, and discovering that at the
/// first dispatch would mean discovering it once per campaign instead of once.
///
/// # Errors
/// [`BriefError`] when the delegation is refused or the write fails.
#[allow(clippy::too_many_arguments)]
pub async fn record_campaign(
    pg: &PgPool,
    campaign_id: Uuid,
    run_id: Uuid,
    tenant_id: &str,
    prereg_id: Uuid,
    prereg_hash: &str,
    job_id: &str,
    brief: &PinnedBrief,
    max_depth: u32,
) -> Result<CampaignBrief, BriefError> {
    let text_digest = digest(&brief.text);
    let shown: Vec<String> = brief.shown.clone();
    Delegation {
        role: Role::Worker,
        projection: Some(brief.projection.clone()),
        brief_digest: text_digest.clone(),
        withheld: brief.withheld,
        depth: 1,
    }
    .check(max_depth)?;

    sqlx::query(
        "INSERT INTO agent_lifecycle.campaign_run
             (campaign_id, run_id, tenant_id, prereg_id, prereg_hash, job_id,
              projection, brief, brief_digest, shown, withheld)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (campaign_id) DO NOTHING",
    )
    .bind(campaign_id)
    .bind(run_id)
    .bind(tenant_id)
    .bind(prereg_id)
    .bind(prereg_hash)
    .bind(job_id)
    .bind(&brief.projection)
    .bind(&brief.text)
    .bind(&text_digest)
    .bind(Value::from(shown.clone()))
    .bind(i32::try_from(brief.withheld).unwrap_or(i32::MAX))
    .execute(pg)
    .await?;

    Ok(CampaignBrief {
        campaign_id,
        run_id,
        projection: brief.projection.clone(),
        brief: brief.text.clone(),
        digest: text_digest,
        shown,
        withheld: i32::try_from(brief.withheld).unwrap_or(i32::MAX),
    })
}

/// Read a campaign's pinned brief.
///
/// # Errors
/// The database failed. A campaign with no brief is `Ok(None)` — a distinct
/// answer from a failed read, because the driver's response differs: no brief
/// means the campaign was defined by something other than a Research run, and a
/// failed read means try again.
pub async fn campaign_brief(
    pg: &PgPool,
    campaign_id: Uuid,
) -> Result<Option<CampaignBrief>, sqlx::Error> {
    let row: Option<(Uuid, String, String, String, Value, i32)> = sqlx::query_as(
        "SELECT run_id, projection, brief, brief_digest, shown, withheld
           FROM agent_lifecycle.campaign_run WHERE campaign_id = $1",
    )
    .bind(campaign_id)
    .fetch_optional(pg)
    .await?;
    Ok(row.map(
        |(run_id, projection, brief, digest, shown, withheld)| CampaignBrief {
            campaign_id,
            run_id,
            projection,
            brief,
            digest,
            shown: shown
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            withheld,
        },
    ))
}

/// A claimed position in a campaign's fold.
///
/// Not `Clone`, because it may carry a [`BudgetLease`], which is spent by exactly
/// one run.
#[derive(Debug)]
pub struct Claim {
    pub sub_run_id: Uuid,
    /// What this sub-run may spend, split from its parent's remaining budget.
    /// `Some` only on the call that created the claim (`fresh`): a lease handed
    /// out once is not handed out again to the driver that finds the row. `None`
    /// also when the parent recorded no cap to split from.
    pub lease: Option<BudgetLease>,
    /// The job carrying it, if one has been dispatched for this position.
    pub job_id: Option<String>,
    /// True when this call created the row rather than finding it.
    pub fresh: bool,
}

/// Claim `(campaign, phase, seq)` for a worker sub-run, or return the claim that
/// is already there.
///
/// The unique index on the position is what makes this exactly-once; the
/// `ON CONFLICT` turns the second caller's refusal into the first caller's row,
/// which is the answer it wanted.
///
/// # Errors
/// The delegation is refused, or the write fails.
#[allow(clippy::too_many_arguments)]
pub async fn claim(
    pg: &PgPool,
    campaign_id: Uuid,
    parent_run_id: Uuid,
    tenant_id: &str,
    phase: &str,
    seq: i64,
    brief: &CampaignBrief,
    max_depth: u32,
) -> Result<Claim, BriefError> {
    claim_leased(
        pg,
        campaign_id,
        parent_run_id,
        tenant_id,
        phase,
        seq,
        brief,
        max_depth,
        WORKER_LEASE,
    )
    .await
}

/// What one worker generation is worth: it proposes one move, so it is given a
/// fraction of the parent's remaining tokens, no trials (a proposal is not a
/// look), and a short step ceiling.
pub const WORKER_LEASE: LeasePolicy = LeasePolicy {
    token_share_denominator: 4,
    token_ceiling: 100_000,
    steps: 12,
};

/// How a sub-run's lease is sized from its parent's remaining budget.
#[derive(Debug, Clone, Copy)]
pub struct LeasePolicy {
    /// The lease asks for `remaining / this` tokens.
    pub token_share_denominator: u64,
    /// ...and never more than this, so one generation cannot take a whole
    /// campaign's allowance.
    pub token_ceiling: u64,
    pub steps: u32,
}

/// What the parent started with and has used, read from its own rows.
///
/// The cap and the step ceiling come from the `RunSpec` the parent's effect log
/// opens with (its profile, as it was when the run started, which is what a
/// later edit to a profile file must not change). What it has spent comes from
/// `agent_runs`, and what it has handed out from the sub-run rows. `None` when
/// the parent has no `RunSpec`: there is no recorded cap to subtract from.
async fn parent_remaining(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    parent_run_id: Uuid,
) -> Result<Option<Remaining>, sqlx::Error> {
    let spec: Option<Value> = sqlx::query_scalar(
        "SELECT payload_json FROM agent_effect_log
          WHERE run_id = $1 AND direction = 'state' AND kind = 'RunSpec'
          ORDER BY seq LIMIT 1",
    )
    .bind(parent_run_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(spec) = spec else {
        return Ok(None);
    };
    let cap_tokens = spec
        .pointer("/profile/budgets/max_tokens_per_task")
        .and_then(Value::as_u64)
        .filter(|c| *c > 0);
    let Some(cap_tokens) = cap_tokens else {
        return Ok(None);
    };
    let max_steps = spec
        .pointer("/profile/orchestration/max_steps")
        .and_then(Value::as_u64)
        .and_then(|s| u32::try_from(s).ok())
        .unwrap_or(0);
    let max_trials = spec
        .pointer("/max_trials")
        .and_then(Value::as_u64)
        .and_then(|t| u32::try_from(t).ok());

    let (spent_tokens, spent_trials, spent_steps): (i64, i32, i32) = sqlx::query_as(
        "SELECT COALESCE(prompt_tokens, 0) + COALESCE(tokens_out, 0),
                COALESCE(trials_spent, 0), COALESCE(iterations, 0)
           FROM agent_runs WHERE run_id = $1",
    )
    .bind(parent_run_id)
    .fetch_one(&mut **tx)
    .await?;
    let (leased_tokens, leased_trials, leased_steps): (i64, i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(lease_tokens), 0)::BIGINT, COALESCE(SUM(lease_trials), 0)::BIGINT,
                COALESCE(SUM(lease_steps), 0)::BIGINT
           FROM agent_lifecycle.sub_run WHERE parent_run_id = $1",
    )
    .bind(parent_run_id)
    .fetch_one(&mut **tx)
    .await?;

    let left = |cap: u64, spent: i64, leased: i64| {
        cap.saturating_sub(u64::try_from(spent.max(0)).unwrap_or(0))
            .saturating_sub(u64::try_from(leased.max(0)).unwrap_or(0))
    };
    Ok(Some(Remaining {
        tokens: left(cap_tokens, spent_tokens, leased_tokens),
        trials: u32::try_from(left(
            u64::from(max_trials.unwrap_or(0)),
            i64::from(spent_trials),
            leased_trials,
        ))
        .unwrap_or(0),
        steps: u32::try_from(left(u64::from(max_steps), i64::from(spent_steps), leased_steps))
            .unwrap_or(0),
    }))
}

/// [`claim`] with an explicit lease policy.
///
/// The lease is split from the parent's remaining budget **in the same
/// transaction that inserts the claim**, under an advisory lock on the parent, so
/// two claims racing for one parent cannot both be told the same remainder. A
/// claim that finds its row already there hands out nothing: the lease was
/// issued the first time.
///
/// # Errors
/// The delegation is refused, the parent cannot fund the lease, or a write fails.
#[allow(clippy::too_many_arguments)]
pub async fn claim_leased(
    pg: &PgPool,
    campaign_id: Uuid,
    parent_run_id: Uuid,
    tenant_id: &str,
    phase: &str,
    seq: i64,
    brief: &CampaignBrief,
    max_depth: u32,
    policy: LeasePolicy,
) -> Result<Claim, BriefError> {
    Delegation {
        role: Role::Worker,
        projection: Some(brief.projection.clone()),
        brief_digest: brief.digest.clone(),
        withheld: usize::try_from(brief.withheld).unwrap_or(0),
        depth: 1,
    }
    .check(max_depth)?;

    let mut tx = pg.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(parent_run_id.to_string())
        .execute(&mut *tx)
        .await?;

    // Already claimed: the lease went out the first time.
    let already: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT sub_run_id, job_id FROM agent_lifecycle.sub_run
          WHERE campaign_id = $1 AND phase = $2 AND seq = $3",
    )
    .bind(campaign_id)
    .bind(phase)
    .bind(seq)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((existing, job_id)) = already {
        tx.commit().await?;
        return Ok(Claim {
            sub_run_id: existing,
            lease: None,
            job_id: (!job_id.starts_with("unclaimed:")).then_some(job_id),
            fresh: false,
        });
    }

    // Split before inserting, so an unfunded parent leaves no row behind.
    let lease = match parent_remaining(&mut tx, parent_run_id).await? {
        Some(mut remaining) => {
            let want = Remaining {
                tokens: (remaining.tokens / policy.token_share_denominator.max(1))
                    .min(policy.token_ceiling),
                trials: 0,
                steps: policy.steps.min(remaining.steps),
            };
            Some(BudgetLease::split(&mut remaining, want)?)
        }
        // No recorded cap to split from; stated as a missing lease, never a
        // zero one.
        None => None,
    };

    let sub_run_id = Uuid::new_v4();
    // `job_id` is NOT NULL on the table, and a claim exists precisely because
    // the job does not yet. The empty-string CHECK forbids a placeholder, so the
    // claim carries the sub-run's own id until the dispatch overwrites it —
    // which is a truthful value (nothing is running yet) rather than a fake job
    // handle something might try to poll.
    let placeholder = format!("unclaimed:{sub_run_id}");
    let inserted: Option<(Uuid,)> = sqlx::query_as(
        "INSERT INTO agent_lifecycle.sub_run
             (sub_run_id, parent_run_id, tenant_id, role, projection, job_id,
              campaign_id, phase, seq, brief_digest, withheld,
              lease_tokens, lease_trials, lease_steps)
         VALUES ($1, $2, $3, 'worker', $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
         ON CONFLICT (campaign_id, phase, seq) WHERE campaign_id IS NOT NULL DO NOTHING
         RETURNING sub_run_id",
    )
    .bind(sub_run_id)
    .bind(parent_run_id)
    .bind(tenant_id)
    .bind(&brief.projection)
    .bind(&placeholder)
    .bind(campaign_id)
    .bind(phase)
    .bind(seq)
    .bind(&brief.digest)
    .bind(brief.withheld)
    .bind(lease.as_ref().map(|l| i64::try_from(l.tokens()).unwrap_or(i64::MAX)))
    .bind(lease.as_ref().map(|l| i32::try_from(l.trials()).unwrap_or(i32::MAX)))
    .bind(lease.as_ref().map(|l| i32::try_from(l.steps()).unwrap_or(i32::MAX)))
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;

    // The advisory lock made the pre-check authoritative, so a conflict here is
    // not expected; if it happens the row is whoever's it is and the lease the
    // loser split is dropped unspent, which is the safe direction.
    let (sub_run_id, fresh, lease) = match inserted {
        Some(_) => (sub_run_id, true, lease),
        None => {
            let existing: Uuid = sqlx::query_scalar(
                "SELECT sub_run_id FROM agent_lifecycle.sub_run
                  WHERE campaign_id = $1 AND phase = $2 AND seq = $3",
            )
            .bind(campaign_id)
            .bind(phase)
            .bind(seq)
            .fetch_one(pg)
            .await?;
            (existing, false, None)
        }
    };
    if fresh {
        let persisted = lease.as_ref().map(|l| PersistedLease {
            tokens: Some(l.tokens()),
            steps: Some(l.steps()),
        });
        record_delegation(
            pg,
            parent_run_id,
            sub_run_id,
            &Delegation {
                role: Role::Worker,
                projection: Some(brief.projection.clone()),
                brief_digest: brief.digest.clone(),
                withheld: usize::try_from(brief.withheld).unwrap_or(0),
                depth: 1,
            },
            &format!("propose one move for campaign {campaign_id}, {phase} #{seq}"),
            persisted.as_ref(),
        )
        .await;
    }
    Ok(Claim {
        sub_run_id,
        lease,
        job_id: None,
        fresh,
    })
}

// ── Both sides of a delegation, and the sub-run's own log (S-2.12) ──────────

/// Append one row to a sub-run's own effect log (`agent_lifecycle.sub_run_effect`,
/// migration 0235). The sequence is assigned here and a collision (two writers
/// racing for one number) is retried, which is what the primary key is for.
///
/// A failed write is logged, not propagated: like the run effect log, a lost row
/// costs a replay, and failing a worker's proposal to protect its record would
/// trade the contingent thing for the certain one.
pub async fn log_effect(pg: &PgPool, sub_run_id: Uuid, direction: &str, kind: &str, payload: &Value) {
    for _ in 0..5 {
        let r = sqlx::query(
            "INSERT INTO agent_lifecycle.sub_run_effect
                 (sub_run_id, seq, direction, kind, payload_json, harness_version)
             SELECT $1, COALESCE(MAX(seq), 0) + 1, $2, $3, $4, $5
               FROM agent_lifecycle.sub_run_effect WHERE sub_run_id = $1",
        )
        .bind(sub_run_id)
        .bind(direction)
        .bind(kind)
        .bind(payload)
        .bind(super::local_driver::HARNESS_VERSION)
        .execute(pg)
        .await;
        match r {
            Ok(_) => return,
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {}
            Err(e) => {
                tracing::warn!(%sub_run_id, kind, error = %e, "the sub-run effect log did not record a row");
                return;
            }
        }
    }
    tracing::warn!(%sub_run_id, kind, "the sub-run effect log lost five sequence races");
}

/// Record the dispatch on **both** sides (AGENT-010 §9: a `Delegation` trace on
/// the parent and on the child).
///
/// The parent's side is a `state` / `Delegation` row in the parent run's own
/// `agent_effect_log` — which `trace_from_effects` projects to
/// `Event::Delegation` — and the child's is the first row of its own log. They
/// carry the same digest, so "what did the worker see" is answerable from either
/// end and checkable against the other.
pub async fn record_delegation(
    pg: &PgPool,
    parent_run_id: Uuid,
    sub_run_id: Uuid,
    delegation: &Delegation,
    goal: &str,
    lease: Option<&PersistedLease>,
) {
    let lease_json = lease.map(|l| serde_json::json!({ "tokens": l.tokens, "steps": l.steps }));
    let payload = serde_json::json!({
        "child_trace_id": sub_run_id,
        "goal": goal,
        "tool_grant": Vec::<String>::new(),
        "budget_usd": 0.0,
        "role": delegation.role.as_str(),
        "projection": delegation.projection,
        "brief_digest": delegation.brief_digest,
        "withheld": delegation.withheld,
        "depth": delegation.depth,
        "lease": lease_json,
    });
    for _ in 0..5 {
        let r = sqlx::query(
            "INSERT INTO agent_effect_log (run_id, seq, direction, kind, payload_json, harness_version)
             SELECT $1, COALESCE(MAX(seq), 0) + 1, 'state', 'Delegation', $2, $3
               FROM agent_effect_log WHERE run_id = $1",
        )
        .bind(parent_run_id)
        .bind(&payload)
        .bind(super::local_driver::HARNESS_VERSION)
        .execute(pg)
        .await;
        match r {
            Ok(_) => break,
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {}
            Err(e) => {
                tracing::warn!(%parent_run_id, error = %e, "the parent's Delegation row was not recorded");
                break;
            }
        }
    }
    let mut child = payload;
    if let Some(o) = child.as_object_mut() {
        o.insert("parent_run_id".into(), serde_json::json!(parent_run_id));
    }
    log_effect(pg, sub_run_id, "state", "Delegation", &child).await;
}

/// What a worker sub-run was leased, read back from its own row.
///
/// The in-memory [`BudgetLease`] is spent by the process that claimed the
/// position, which is the campaign driver; the worker runs in a different job, so
/// the lease reaches it through the row (migration 0171). `None` on an axis means
/// the claim recorded no lease for it -- the parent had no recorded cap to split
/// from -- and is **not** a zero: the worker is then bounded by the profile alone,
/// which is the weaker guarantee and is stated, not hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistedLease {
    pub tokens: Option<u64>,
    pub steps: Option<u32>,
}

/// Read a sub-run's persisted lease.
///
/// # Errors
/// The read failed.
pub async fn lease_of(pg: &PgPool, sub_run_id: Uuid) -> Result<PersistedLease, sqlx::Error> {
    let row: Option<(Option<i64>, Option<i32>)> = sqlx::query_as(
        "SELECT lease_tokens, lease_steps FROM agent_lifecycle.sub_run WHERE sub_run_id = $1",
    )
    .bind(sub_run_id)
    .fetch_optional(pg)
    .await?;
    let (t, s) = row.unwrap_or((None, None));
    Ok(PersistedLease {
        tokens: t.and_then(|v| u64::try_from(v).ok()),
        steps: s.and_then(|v| u32::try_from(v).ok()),
    })
}

impl PersistedLease {
    /// Whether a worker that has made `attempts_made` model calls and spent
    /// `tokens_spent` may make another. The refusal names the axis, because a
    /// worker stopped by its lease and a worker that failed are different events.
    ///
    /// # Errors
    /// The reason, when the lease is spent on an axis it carries.
    pub fn admits(&self, attempts_made: u32, tokens_spent: u64) -> Result<(), String> {
        if let Some(steps) = self.steps {
            if attempts_made >= steps {
                return Err(format!(
                    "the lease allows {steps} model call(s) and {attempts_made} were made"
                ));
            }
        }
        if let Some(tokens) = self.tokens {
            if tokens_spent >= tokens {
                return Err(format!(
                    "the lease allows {tokens} token(s) and {tokens_spent} were spent"
                ));
            }
        }
        Ok(())
    }

    /// The most the next call may generate: the profile's reserve, narrowed to
    /// what the lease has left. A lease narrows a call and never widens it.
    #[must_use]
    pub fn max_output(&self, reserve: u32, tokens_spent: u64) -> u32 {
        match self.tokens {
            Some(t) => {
                let left = t.saturating_sub(tokens_spent);
                u32::try_from(left).map_or(reserve, |l| l.min(reserve))
            }
            None => reserve,
        }
    }
}

/// Point a claim at the job that is carrying it.
///
/// # Errors
/// The write failed.
pub async fn dispatched(pg: &PgPool, sub_run_id: Uuid, job_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE agent_lifecycle.sub_run SET job_id = $2 WHERE sub_run_id = $1")
        .bind(sub_run_id)
        .bind(job_id)
        .execute(pg)
        .await?;
    Ok(())
}

/// Record what a sub-run produced.
///
/// # Errors
/// The write failed.
pub async fn settle(pg: &PgPool, sub_run_id: Uuid, outcome: &Value) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE agent_lifecycle.sub_run
            SET outcome = $2, settled_at = now()
          WHERE sub_run_id = $1 AND settled_at IS NULL",
    )
    .bind(sub_run_id)
    .bind(outcome)
    .execute(pg)
    .await?;
    Ok(())
}

/// The charter tail a worker sub-run runs under (§9).
///
/// Short on purpose. The constraints that matter are not in this text: what the
/// worker can see is the projection, and what it may answer is the grammar. This
/// says what the job is, and nothing it says is load-bearing — which is the
/// property a charter should have.
pub const WORKER_CHARTER: &str = "\
You are proposing one change to a strategy inside a running search campaign.

You have been given the campaign's evidence — findings, results, diagnostics, \
the current candidate and its neighbours — and nothing about how the parent run \
reasoned its way there. That is deliberate: your value is a look at the evidence \
that is not a continuation of somebody else's argument.

Answer with exactly one move, the hypothesis that makes it worth trying, and \
ranges for any parameter the move introduces or changes. One move, not a plan: a \
generation that changes three things and improves has not shown which change did \
it, and the next generation inherits the confusion.";

/// The depth a campaign's worker sub-run sits at, and the ceiling it is checked
/// against (`orchestration.max_subagent_depth`, 1 on every shipped profile).
///
/// Here rather than in the driver or the route because both need it and neither
/// holds a profile: the driver is a job worker, and the route answers before any
/// run is resolved. Two copies of a depth limit is how one of them stops being
/// the limit.
pub const MAX_SUBAGENT_DEPTH: u32 = 1;

/// Tokens the manifest over the projected store may take.
///
/// The manifest never truncates — its floor is one line per kind — so this is a
/// target the grouping coarsens to meet rather than a cut. Small, because a
/// worker's index is over a store that has already been filtered to evidence.
pub const BRIEF_MANIFEST_TOKENS: u32 = 200;

/// How much of a sub-run's input window the pinned brief may occupy.
///
/// Three quarters, with the remaining quarter reserved for the charter, the
/// generation's own evidence and — on the one retry the grammar allows — the
/// validator's reason. A brief sized to the whole window would leave the retry
/// nowhere to go, and the retry is the mechanism that turns a malformed move
/// into a correct one instead of into a failed generation.
#[must_use]
pub fn brief_budget(profile: &harness::profile::Profile) -> u32 {
    profile.input_budget_tokens().saturating_mul(3) / 4
}

/// The projection a worker is dispatched under, named once so the driver and the
/// worker cannot disagree about it.
#[must_use]
pub fn worker_projection_name() -> &'static str {
    Projection::worker([]).as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness::context::body::Body;
    use harness::context::store::{Draft, RecordKind, Subject};

    fn parent() -> RunStore {
        let mut s = RunStore::new();
        s.insert(
            Draft::new(
                1,
                RecordKind::ToolResult,
                "get_diagnostics",
                Body::text("gate 2 failed: CPCV fold 3 is negative"),
            )
            .with_subject(Subject::instrument("BTC-USD")),
        );
        s.insert(Draft::new(
            2,
            RecordKind::Finding,
            "record_finding",
            Body::text("the edge concentrates in the trending regime"),
        ));
        s.insert(Draft::new(
            3,
            RecordKind::Decision,
            "harness",
            Body::text("I will try a regime gate next"),
        ));
        s
    }

    fn query() -> harness::context::select::Query {
        harness::context::select::Query {
            step: 1,
            goal_kind: harness::state::GoalKind::Research,
            subject: Subject::instrument("BTC-USD"),
            ..Default::default()
        }
    }

    /// The brief a campaign pins excludes the parent's reasoning, and the
    /// exclusion survives the trip through this module rather than being a
    /// property of the harness function alone.
    #[test]
    fn the_assembled_brief_is_a_worker_projection_without_the_reasoning() {
        let policy = harness::context::policy::ContextPolicy::v1();
        let b = assemble(&parent(), &query(), 2_000, &policy, 200);
        assert_eq!(b.projection, worker_projection_name());
        assert_eq!(b.withheld.len(), 1, "the one Decision the parent made");
        assert!(
            !b.text.contains("I will try a regime gate next"),
            "the parent's reasoning reached the worker's brief"
        );
        assert!(b.text.contains("trending regime"), "the evidence did not");
    }

    /// Same bytes in, same digest out — and different bytes, different digest.
    /// Without the second half the digest would be decorative.
    #[test]
    fn the_digest_distinguishes_briefs() {
        assert_eq!(digest("a brief"), digest("a brief"));
        assert_ne!(digest("a brief"), digest("a brief "));
        assert!(digest("x").starts_with("sha256:"));
    }

    /// A worker dispatched under anything but its own projection is refused
    /// before the row is written, not after the model has answered.
    #[test]
    fn a_worker_under_the_wrong_projection_is_refused() {
        let d = Delegation {
            role: Role::Worker,
            projection: Some("full".into()),
            brief_digest: digest("x"),
            withheld: 0,
            depth: 1,
        };
        assert!(d.check(1).is_err());
    }

    /// The persisted lease is enforced on the axes it carries and silent on the
    /// ones it does not: a missing axis is "bounded by the profile", never zero.
    #[test]
    fn a_persisted_lease_refuses_past_its_steps_and_tokens_and_a_missing_axis_is_not_zero() {
        let l = PersistedLease { tokens: Some(1_000), steps: Some(2) };
        assert!(l.admits(0, 0).is_ok());
        assert!(l.admits(1, 999).is_ok());
        assert!(l.admits(2, 10).unwrap_err().contains("2 model call"));
        assert!(l.admits(1, 1_000).unwrap_err().contains("1000 token"));
        let open = PersistedLease { tokens: None, steps: None };
        assert!(open.admits(500, u64::MAX).is_ok(), "no recorded lease is not a zero lease");
        // The lease narrows a call and never widens it.
        assert_eq!(l.max_output(2_000, 0), 1_000);
        assert_eq!(l.max_output(2_000, 900), 100);
        assert_eq!(l.max_output(50, 0), 50);
        assert_eq!(open.max_output(2_000, 7), 2_000);
    }
}
