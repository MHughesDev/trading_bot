//! The deployment path's database half (Set T T-1.1, T-1.2, T-1.3, T-2.1).
//!
//! ```text
//!   research run / person ──▶ create_proposal ──▶ deployment_proposal   (inert)
//!   a person, web:full    ──▶ decide_proposal ──▶ deployment_decision    (envelope)
//!   a person, web:full    ──▶ authorise_deployment ──▶ deployment_authorisation
//!                                                  └─▶ strategy_deployment (0142)
//! ```
//!
//! Nothing here deploys. The deployments route calls [`authorise_deployment`]
//! **before** it records and registers a deployment, and migration 0180's trigger
//! on `strategy_deployment` is what refuses a deployment that skipped it, once the
//! one-way switch is armed (ADR-T1-01). So the route's call is the polite path and
//! the trigger is the enforcement; neither is optional once armed.
//!
//! This module names no `crate::` item, so it is also compiled into
//! `tests/deployment_path.rs` by path and exercised against a real database.

use std::collections::BTreeSet;

use ledger::deployment_path::{
    check_not_stale, envelope, DivergenceThresholds, GateVerdictEntry, PaperAuthority,
    ProposalDraft, ProposalRefusal,
};
use ledger::fleet_ceiling::{admit_deployment, CeilingError, FleetCeilings, Running};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// The one scope a person's web session carries. A deployment decision is made
/// with exactly this; an agent's service session never holds it
/// (`auth::scopes::NEVER_FOR_AGENTS` lists it).
pub const HUMAN_SCOPE: &str = "web:full";

/// Whether a token's scopes include a person's web session.
///
/// Deliberately **not** `auth::scopes::permits`: that treats `web:full` as
/// satisfying every scope and a named scope as satisfying itself, so a service
/// token holding a scope literally spelled the same would pass. Only the human
/// scope itself decides a deployment.
#[must_use]
pub fn may_decide(held: &[String]) -> bool {
    held.iter().any(|s| s == HUMAN_SCOPE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposerKind {
    Agent,
    Human,
    Scheduler,
}

impl ProposerKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Human => "human",
            Self::Scheduler => "scheduler",
        }
    }

    /// A web session is a person; anything else that holds a token is not.
    #[must_use]
    pub fn from_scopes(held: &[String]) -> Self {
        if may_decide(held) {
            Self::Human
        } else {
            Self::Agent
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GateError {
    #[error("a deployment is decided and started by a person, from a web session; an agent or service token cannot")]
    NotHuman,
    #[error(transparent)]
    Refused(#[from] ProposalRefusal),
    #[error(transparent)]
    Ceiling(#[from] CeilingError),
    #[error("no such proposal")]
    NotFound,
    #[error("this proposal was already decided")]
    AlreadyDecided,
    #[error("this proposal has no approved decision behind it")]
    NotApproved,
    #[error("this proposal belongs to another user")]
    WrongOwner,
    #[error("this proposal already authorised a deployment; a decision is spent once")]
    AlreadyUsed,
    #[error("the monitor refused the stop: {0}")]
    Monitor(String),
    #[error("storage: {0}")]
    Db(#[from] sqlx::Error),
}

impl GateError {
    /// `(http status, machine code)`.
    #[must_use]
    pub fn http(&self) -> (u16, &'static str) {
        match self {
            Self::NotHuman => (403, "not_a_human_session"),
            Self::Refused(ProposalRefusal::StaleAgainstHead { .. } | ProposalRefusal::StaleVerdict { .. }) => {
                (409, "stale_verdict")
            }
            Self::Refused(_) => (422, "invalid_proposal"),
            Self::Ceiling(_) => (409, "ceiling"),
            Self::NotFound => (404, "unknown_proposal"),
            Self::AlreadyDecided => (409, "already_decided"),
            Self::NotApproved => (409, "not_approved"),
            Self::WrongOwner => (404, "unknown_proposal"),
            Self::AlreadyUsed => (409, "already_used"),
            Self::Monitor(_) => (500, "monitor"),
            Self::Db(_) => (500, "storage"),
        }
    }
}

// ── creating a proposal (cheap; nothing changes) ─────────────────────────────

/// Record a proposal. Creation is deliberately not gated: asking costs a row.
/// The structural rules still apply, and so does staleness when the strategy's
/// head is known.
///
/// # Errors
/// A malformed or stale draft, or a database error.
pub async fn create_proposal(
    pg: &PgPool,
    owner_user_id: &str,
    proposed_by: &str,
    kind: ProposerKind,
    draft: ProposalDraft,
    head_revision: Option<&str>,
) -> Result<Uuid, GateError> {
    let valid = draft.validate()?;
    let d = valid.draft();
    if let Some(head) = head_revision {
        check_not_stale(&d.revision_hash, head)?;
    }
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO deployment_proposal
           (proposal_id, owner_user_id, proposed_by, proposer_kind, strategy_name, revision_hash,
            instrument_id, profile_id, verdict, verdict_revision_hash, gates_passed, thresholds,
            starting_capital, review_period_days)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13::numeric,$14)",
    )
    .bind(id)
    .bind(owner_user_id)
    .bind(proposed_by)
    .bind(kind.as_str())
    .bind(&d.strategy_name)
    .bind(&d.revision_hash)
    .bind(&d.instrument_id)
    .bind(&d.profile_id)
    .bind(serde_json::to_value(&d.verdict).unwrap_or(Value::Null))
    .bind(&d.verdict_revision_hash)
    .bind(valid.gates_passed())
    .bind(serde_json::to_value(d.thresholds).unwrap_or(Value::Null))
    .bind(&d.starting_capital)
    .bind(d.review_period_days)
    .execute(pg)
    .await?;
    Ok(id)
}

/// A stored proposal, rebuilt as the draft it was.
///
/// # Errors
/// `NotFound`, or a stored row that no longer parses (which would be a bug).
pub async fn load_proposal(
    pg: &PgPool,
    proposal_id: Uuid,
) -> Result<(String, ProposalDraft), GateError> {
    let row = sqlx::query(
        "SELECT owner_user_id, strategy_name, revision_hash, instrument_id, profile_id, verdict,
                verdict_revision_hash, thresholds, starting_capital::text AS capital, review_period_days
           FROM deployment_proposal WHERE proposal_id = $1",
    )
    .bind(proposal_id)
    .fetch_optional(pg)
    .await?
    .ok_or(GateError::NotFound)?;
    let verdict: Vec<GateVerdictEntry> =
        serde_json::from_value(row.get::<Value, _>("verdict")).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    let thresholds: DivergenceThresholds =
        serde_json::from_value(row.get::<Value, _>("thresholds")).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    let capital: String = row.get("capital");
    Ok((
        row.get("owner_user_id"),
        ProposalDraft {
            strategy_name: row.get("strategy_name"),
            revision_hash: row.get("revision_hash"),
            instrument_id: row.get("instrument_id"),
            profile_id: row.get("profile_id"),
            verdict_revision_hash: row.get("verdict_revision_hash"),
            verdict,
            thresholds,
            starting_capital: trim_decimal(&capital),
            review_period_days: row.get("review_period_days"),
        },
    ))
}

/// `10000.000000000000000000` -> `10000`, so the envelope's hash is of the
/// number the proposer wrote and not of Postgres' padding.
fn trim_decimal(s: &str) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s.to_owned()
    }
}

// ── the fleet's ceilings (T-2.1) ─────────────────────────────────────────────

/// The declared ceilings, or the first one that is not.
///
/// # Errors
/// [`CeilingError::Undeclared`] for a ceiling nobody declared (which refuses
/// every deployment), or a database error.
pub async fn load_ceilings(pg: &PgPool) -> Result<FleetCeilings, GateError> {
    let rows = sqlx::query("SELECT name, value::float8 AS value FROM current_deployment_ceiling")
        .fetch_all(pg)
        .await?;
    let get = |name: &str| {
        rows.iter()
            .find(|r| r.get::<String, _>("name") == name)
            .map(|r| r.get::<f64, _>("value"))
    };
    Ok(FleetCeilings::declared(
        get("max_concurrent"),
        get("max_aggregate_capital"),
        get("max_instrument_exposure"),
    )?)
}

/// Declare a ceiling. A person only; append-only.
///
/// # Errors
/// `NotHuman`, a non-positive value, or a database error.
pub async fn declare_ceiling(
    pg: &PgPool,
    held: &[String],
    declared_by: &str,
    name: &str,
    value: &str,
    note: &str,
) -> Result<(), GateError> {
    if !may_decide(held) {
        return Err(GateError::NotHuman);
    }
    sqlx::query(
        "INSERT INTO deployment_ceiling (name, value, declared_by, note) VALUES ($1, $2::numeric, $3, $4)",
    )
    .bind(name)
    .bind(value)
    .bind(declared_by)
    .bind(note)
    .execute(pg)
    .await?;
    Ok(())
}

/// Every deployment that already holds a place in the fleet: running ones, and
/// approved decisions that have not yet become a deployment (reserved, so two
/// approvals made back to back cannot both claim the last place).
///
/// # Errors
/// A database error.
pub async fn fleet(pg: &PgPool) -> Result<Vec<Running>, GateError> {
    let rows = sqlx::query(
        "SELECT revision_hash, instrument_id, starting_capital::float8 AS cap
           FROM strategy_deployment
          WHERE stopped_at IS NULL AND parent_deployment_id IS NULL
         UNION ALL
         SELECT p.revision_hash, p.instrument_id, p.starting_capital::float8
           FROM deployment_decision d
           JOIN deployment_proposal p USING (proposal_id)
          WHERE d.decision = 'approved'
            AND NOT EXISTS (SELECT 1 FROM deployment_authorisation a WHERE a.proposal_id = d.proposal_id)",
    )
    .fetch_all(pg)
    .await?;
    Ok(rows
        .iter()
        .map(|r| Running {
            revision_hash: r.get("revision_hash"),
            instrument_id: r.get("instrument_id"),
            capital_usd: r.get("cap"),
        })
        .collect())
}

// ── deciding ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct Decided {
    pub proposal_id: Uuid,
    pub approved: bool,
    pub envelope: Value,
}

/// A person decides. Approval re-checks, at the moment that matters:
///
/// * the gates (`PaperAuthority`: 1..=15 passed — `paper_v1` or `strict_v1`, a
///   paper venue either way);
/// * staleness against the strategy's head, when it is stored;
/// * the fleet's ceilings, with the binding one named.
///
/// A rejection records itself and checks nothing.
///
/// # Errors
/// `NotHuman`, `NotFound`, `AlreadyDecided`, a refusal of any rule above, or a
/// database error.
pub async fn decide_proposal(
    pg: &PgPool,
    held: &[String],
    decided_by: &str,
    proposal_id: Uuid,
    approve: bool,
    note: &str,
    head_revision: Option<&str>,
) -> Result<Decided, GateError> {
    if !may_decide(held) {
        return Err(GateError::NotHuman);
    }
    let (_, draft) = load_proposal(pg, proposal_id).await?;
    let mut tx = pg.begin().await?;
    // One decision at a time: the ceilings are a read-then-write.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('deployment_gate'))")
        .execute(&mut *tx)
        .await?;
    let already: Option<(String,)> =
        sqlx::query_as("SELECT decision FROM deployment_decision WHERE proposal_id = $1")
            .bind(proposal_id)
            .fetch_optional(&mut *tx)
            .await?;
    if already.is_some() {
        return Err(GateError::AlreadyDecided);
    }
    if approve {
        PaperAuthority::from_pass(&draft.profile_id, &draft.verdict)?;
        if let Some(head) = head_revision {
            check_not_stale(&draft.revision_hash, head)?;
        }
        let ceilings = load_ceilings(pg).await?;
        let running = fleet(pg).await?;
        let capital: f64 = draft.starting_capital.parse().unwrap_or(f64::NAN);
        admit_deployment(
            &ceilings,
            &running,
            &Running {
                revision_hash: draft.revision_hash.clone(),
                instrument_id: draft.instrument_id.clone(),
                capital_usd: capital,
            },
        )?;
    }
    let decided_at = chrono::Utc::now().to_rfc3339();
    let env = envelope(&draft, decided_by, &decided_at);
    sqlx::query(
        "INSERT INTO deployment_decision (proposal_id, decision, decided_by, note, envelope)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(proposal_id)
    .bind(if approve { "approved" } else { "rejected" })
    .bind(decided_by)
    .bind(note)
    .bind(&env)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Decided {
        proposal_id,
        approved: approve,
        envelope: env,
    })
}

// ── authorising a deployment ─────────────────────────────────────────────────

/// Spend an approved decision on one deployment id. Called by the deployments
/// route **before** it records the deployment; the trigger on
/// `strategy_deployment` then checks the revision, owner, capital and instrument
/// against the proposal.
///
/// # Errors
/// `NotHuman`, `NotFound`/`WrongOwner` (indistinguishable to the caller),
/// `NotApproved`, `AlreadyUsed`, or a database error.
pub async fn authorise_deployment(
    pg: &PgPool,
    held: &[String],
    caller_user_id: &str,
    deployment_id: Uuid,
    proposal_id: Uuid,
) -> Result<(), GateError> {
    if !may_decide(held) {
        return Err(GateError::NotHuman);
    }
    let (owner, _) = load_proposal(pg, proposal_id).await?;
    if owner != caller_user_id {
        return Err(GateError::WrongOwner);
    }
    let res = sqlx::query(
        "INSERT INTO deployment_authorisation (deployment_id, proposal_id) VALUES ($1, $2)",
    )
    .bind(deployment_id)
    .bind(proposal_id)
    .execute(pg)
    .await;
    match res {
        Ok(_) => Ok(()),
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23505") => {
            Err(GateError::AlreadyUsed)
        }
        // 23503: the composite foreign key — no APPROVED decision to name.
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("23503") => {
            Err(GateError::NotApproved)
        }
        Err(e) => Err(e.into()),
    }
}

// ── revoking (T-1.3) ─────────────────────────────────────────────────────────

/// What a stop did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revoked {
    pub withdrawn: bool,
    pub closed: bool,
}

/// Stop one deployment, in the three places that must agree: the live registry
/// (its runner stops at its next decision tick), the monitor's strategy scope
/// (the order gate refuses its orders **now**, not at the tick), and the record.
///
/// Stopping is never the dangerous direction, so this asks for no authority
/// beyond the caller having already established it owns the deployment.
///
/// # Errors
/// A monitor or database failure. The registry is withdrawn first, so a failure
/// later still leaves the runner stopping.
pub async fn revoke_one(
    pg: &PgPool,
    kill: &risk::KillSwitch,
    deployment_id: Uuid,
    actor: &str,
    reason: &str,
) -> Result<Revoked, GateError> {
    let registry = strategy_runtime::live::global();
    let withdrawn = registry.deregister_v3(deployment_id) | registry.deregister_selection(deployment_id);
    let scope = risk::Scope::strategy(deployment_id.to_string());
    trip(pg, kill, scope, monitor::family::Family::Pnl, actor, reason).await?;
    let closed = storage::postgres::deployments::stop(pg, deployment_id, reason)
        .await
        .map_err(|e| GateError::Monitor(e.to_string()))?;
    Ok(Revoked { withdrawn, closed })
}

/// Stop every deployment with **the action the kill switch is**: the global
/// scope, tripped through the monitor's own store and mirrored into the order
/// gate, exactly as `POST /api/monitor/kill-switch/global/trip` does. Resetting
/// it is the monitor's reset, with its typed confirmation. The registry is then
/// emptied and the records closed.
///
/// # Errors
/// A monitor or database failure.
pub async fn revoke_all(
    pg: &PgPool,
    kill: &risk::KillSwitch,
    actor: &str,
    reason: &str,
) -> Result<Vec<Uuid>, GateError> {
    trip(pg, kill, risk::Scope::Global, monitor::family::Family::Platform, actor, reason).await?;
    let running: Vec<(Uuid,)> =
        sqlx::query_as("SELECT deployment_id FROM strategy_deployment WHERE stopped_at IS NULL")
            .fetch_all(pg)
            .await?;
    let registry = strategy_runtime::live::global();
    let mut stopped = Vec::new();
    for (id,) in running {
        registry.deregister_v3(id);
        registry.deregister_selection(id);
        if storage::postgres::deployments::stop(pg, id, reason)
            .await
            .map_err(|e| GateError::Monitor(e.to_string()))?
        {
            stopped.push(id);
        }
    }
    Ok(stopped)
}

/// The one way this module trips a scope: durable record first, mirror second.
async fn trip(
    pg: &PgPool,
    kill: &risk::KillSwitch,
    scope: risk::Scope,
    family: monitor::family::Family,
    actor: &str,
    reason: &str,
) -> Result<(), GateError> {
    let kind = if matches!(scope, risk::Scope::Global) {
        monitor::events::EventKind::Tripped
    } else {
        monitor::events::EventKind::Halted
    };
    let event = monitor::events::MonitorEvent::new(scope.clone(), family, kind, actor)
        .map_err(|e| GateError::Monitor(e.to_string()))?
        .with_reason(reason.to_owned());
    monitor::store::MonitorStore::new(pg.clone())
        .trip(&event, reason)
        .await
        .map_err(|e| GateError::Monitor(e.to_string()))?;
    kill.trip_scope(&scope);
    Ok(())
}

// ── visibility (T-1.3) ───────────────────────────────────────────────────────

/// One row of the deployments surface: everything a person needs to decide
/// whether to stop it. Built from the record, the authority behind it, the
/// ramp's size and period, and the forward record where there is one.
///
/// # Errors
/// A database error.
pub async fn overview(pg: &PgPool, user_id: &str) -> Result<Vec<Value>, GateError> {
    let rows = sqlx::query(
        "SELECT d.deployment_id, d.strategy_name, d.revision_hash, d.instrument_id, d.environment,
                d.venue, d.starting_capital::text AS capital, d.started_at, d.stopped_at,
                d.stop_reason, d.forward_deployment_id,
                p.proposal_id, p.profile_id, p.gates_passed, p.verdict,
                (m.deployment_id IS NOT NULL) AS legacy_ungated,
                s.halvings,
                (SELECT max(ends_at) FROM ramp_period r WHERE r.deployment_id = d.deployment_id) AS period_ends_at,
                (SELECT max(period_no) FROM ramp_period r WHERE r.deployment_id = d.deployment_id) AS period_no
           FROM strategy_deployment d
           LEFT JOIN deployment_authorisation a ON a.deployment_id = d.deployment_id
           LEFT JOIN deployment_proposal p ON p.proposal_id = a.proposal_id
           LEFT JOIN deployment_legacy_marker m ON m.deployment_id = d.deployment_id
           LEFT JOIN deployment_size s ON s.deployment_id = d.deployment_id
          WHERE d.user_id = $1 AND d.parent_deployment_id IS NULL
          ORDER BY d.started_at DESC",
    )
    .bind(user_id)
    .fetch_all(pg)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let id: Uuid = r.get("deployment_id");
        let forward: Option<Uuid> = r.get("forward_deployment_id");
        let (halts, series) = match forward {
            Some(f) => forward_record(pg, user_id, f).await.unwrap_or((None, Vec::new())),
            None => (None, Vec::new()),
        };
        let halvings: Option<i16> = r.get("halvings");
        out.push(serde_json::json!({
            "deployment_id": id,
            "strategy_name": r.get::<String, _>("strategy_name"),
            "revision": r.get::<String, _>("revision_hash"),
            "instrument_id": r.get::<String, _>("instrument_id"),
            "environment": r.get::<String, _>("environment"),
            "venue": r.get::<String, _>("venue"),
            "starting_capital": r.get::<String, _>("capital"),
            "started_at": r.get::<chrono::DateTime<chrono::Utc>, _>("started_at"),
            "stopped_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("stopped_at"),
            "stop_reason": r.get::<Option<String>, _>("stop_reason"),
            "running": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("stopped_at").is_none(),
            // The authority behind it, or the marker that says there is none.
            "proposal_id": r.get::<Option<Uuid>, _>("proposal_id"),
            "profile_id": r.get::<Option<String>, _>("profile_id"),
            "gates_passed": r.get::<Option<i32>, _>("gates_passed"),
            "verdict": r.get::<Option<Value>, _>("verdict"),
            "legacy_ungated": r.get::<bool, _>("legacy_ungated"),
            // The ramp: size as halvings (0 full .. 4 zero) and the clock.
            "size_halvings": halvings.unwrap_or(0),
            "review_period_no": r.get::<Option<i32>, _>("period_no"),
            "review_period_ends_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("period_ends_at"),
            // The forward record, where a recorder is feeding one.
            "divergence_halts": halts,
            "realised_vs_expected": series,
        }));
    }
    Ok(out)
}

/// Divergence-halt count and the last observed `(bar, realised, expected)`.
async fn forward_record(
    pg: &PgPool,
    tenant_id: &str,
    forward_deployment_id: Uuid,
) -> Result<(Option<i64>, Vec<Value>), sqlx::Error> {
    let mut tx = pg.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;
    let halts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM mlops.divergence_halt WHERE deployment_id = $1",
    )
    .bind(forward_deployment_id)
    .fetch_one(&mut *tx)
    .await?;
    let rows = sqlx::query(
        "SELECT bar, realised_return, expected_return FROM mlops.forward_observation
          WHERE deployment_id = $1 AND source = 'observed' ORDER BY bar DESC LIMIT 200",
    )
    .bind(forward_deployment_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut series: Vec<Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "bar": r.get::<chrono::DateTime<chrono::Utc>, _>("bar"),
                "realised": r.get::<f64, _>("realised_return"),
                "expected": r.get::<f64, _>("expected_return"),
            })
        })
        .collect();
    series.reverse();
    Ok((Some(halts), series))
}

/// The distinct gate numbers in a stored verdict that did not pass, for a
/// summary line.
#[must_use]
pub fn failed_gates(verdict: &Value) -> BTreeSet<i64> {
    verdict
        .as_array()
        .into_iter()
        .flatten()
        .filter(|g| g.get("passed").and_then(Value::as_bool) == Some(false))
        .filter_map(|g| g.get("gate_no").and_then(Value::as_i64))
        .collect()
}
