//! The decision journal (Set U U-4.5; ADR-0047; migration 0144).
//!
//! One record per decision per deployment, with enough to replay the step.
//! Append-only; rows leave only by age, through [`prune_older_than`].

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::PgError;

/// Days a journal record is kept unless the data budget says otherwise.
pub const DEFAULT_RETENTION_DAYS: i64 = 90;

/// One decision, as journaled.
#[derive(Clone, Debug, PartialEq, sqlx::FromRow)]
pub struct JournalRecord {
    pub deployment_id: Uuid,
    /// 1, 2, 3… per deployment.
    pub seq: i64,
    pub tick: i64,
    pub decided_at: DateTime<Utc>,
    /// `bar_close`, `fill`, `order_closed`, `schedule`.
    pub cause: String,
    /// The trace's digest.
    pub digest: String,
    /// sha256 of `inputs`.
    pub input_digest: String,
    /// The event and the account the engine saw.
    pub inputs: serde_json::Value,
    pub trace: serde_json::Value,
    pub gate_verdicts: serde_json::Value,
    pub missing_reasons: serde_json::Value,
}

/// Append one record.
///
/// # Errors
/// A database error — which the caller must surface: a decision that was not
/// journaled cannot be replayed.
pub async fn append(pool: &PgPool, r: &JournalRecord) -> Result<(), PgError> {
    sqlx::query(
        "INSERT INTO strategy_decision_journal (
             deployment_id, seq, tick, decided_at, cause, digest, input_digest,
             inputs, trace, gate_verdicts, missing_reasons
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
    )
    .bind(r.deployment_id)
    .bind(r.seq)
    .bind(r.tick)
    .bind(r.decided_at)
    .bind(&r.cause)
    .bind(&r.digest)
    .bind(&r.input_digest)
    .bind(&r.inputs)
    .bind(&r.trace)
    .bind(&r.gate_verdicts)
    .bind(&r.missing_reasons)
    .execute(pool)
    .await?;
    Ok(())
}

/// The last sequence number written for a deployment (0 when none) — where a
/// restarted runner continues numbering.
///
/// # Errors
/// A database error.
pub async fn last_seq(pool: &PgPool, deployment_id: Uuid) -> Result<i64, PgError> {
    let (seq,): (Option<i64>,) =
        sqlx::query_as("SELECT max(seq) FROM strategy_decision_journal WHERE deployment_id = $1")
            .bind(deployment_id)
            .fetch_one(pool)
            .await?;
    Ok(seq.unwrap_or(0))
}

/// A deployment's records, in order.
///
/// # Errors
/// A database error.
pub async fn for_deployment(
    pool: &PgPool,
    deployment_id: Uuid,
) -> Result<Vec<JournalRecord>, PgError> {
    Ok(sqlx::query_as(
        "SELECT deployment_id, seq, tick, decided_at, cause, digest, input_digest, inputs, trace, \
                gate_verdicts, missing_reasons \
         FROM strategy_decision_journal WHERE deployment_id = $1 ORDER BY seq",
    )
    .bind(deployment_id)
    .fetch_all(pool)
    .await?)
}

/// Delete records decided before `cutoff`. Returns how many went.
///
/// # Errors
/// A database error.
pub async fn prune_older_than(pool: &PgPool, cutoff: DateTime<Utc>) -> Result<u64, PgError> {
    let done = sqlx::query("DELETE FROM strategy_decision_journal WHERE decided_at < $1")
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(done.rows_affected())
}

/// Deployments with a decision journaled in `[from, to)` — the ones a nightly
/// parity check replays (U-8.2).
///
/// # Errors
/// A database error.
pub async fn decided_between(
    pool: &PgPool,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<Uuid>, PgError> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT DISTINCT deployment_id FROM strategy_decision_journal \
         WHERE decided_at >= $1 AND decided_at < $2 ORDER BY deployment_id",
    )
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(d,)| d).collect())
}

/// One replayed segment's verdict (migration 0149).
#[derive(Clone, Debug, PartialEq)]
pub struct ParityCheck {
    pub deployment_id: Uuid,
    pub revision: String,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub first_seq: i64,
    pub last_seq: i64,
    /// `identical`, `diverged` or `unreplayable`.
    pub verdict: String,
    pub decisions: Option<i64>,
    pub diverged_seq: Option<i64>,
    pub field: Option<String>,
    pub recorded: Option<serde_json::Value>,
    pub replayed: Option<serde_json::Value>,
    pub reason: Option<String>,
}

/// Record a verdict. Returns its id.
///
/// # Errors
/// A database error, or a row the table's checks refuse.
pub async fn record_parity(pool: &PgPool, c: &ParityCheck) -> Result<i64, PgError> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO strategy_parity_check (
             deployment_id, revision, window_start, window_end, first_seq, last_seq,
             verdict, decisions, diverged_seq, field, recorded, replayed, reason
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
         RETURNING check_id",
    )
    .bind(c.deployment_id)
    .bind(&c.revision)
    .bind(c.window_start)
    .bind(c.window_end)
    .bind(c.first_seq)
    .bind(c.last_seq)
    .bind(&c.verdict)
    .bind(c.decisions)
    .bind(c.diverged_seq)
    .bind(&c.field)
    .bind(&c.recorded)
    .bind(&c.replayed)
    .bind(&c.reason)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

// ── forward evidence: what a recorder resumes from (Set T T-0.4; 0186) ─────────

/// A bar a forward recorder opened, closed or re-anchored at, and its mark.
#[derive(Clone, Debug, PartialEq, sqlx::FromRow)]
pub struct ForwardMark {
    pub forward_deployment_id: Uuid,
    pub tenant_id: String,
    pub bar: DateTime<Utc>,
    pub mark: rust_decimal::Decimal,
    /// `open`, `close` or `reanchor`.
    pub kind: String,
}

/// One fill of a forward deployment, with what resuming its series needs.
#[derive(Clone, Debug, PartialEq, sqlx::FromRow)]
pub struct ForwardFill {
    pub forward_deployment_id: Uuid,
    pub tenant_id: String,
    /// 1, 2, 3… per forward deployment.
    pub seq: i64,
    /// `buy` or `sell`.
    pub side: String,
    /// This fill's quantity, never the cumulative figure.
    pub filled: rust_decimal::Decimal,
    pub fill_price: rust_decimal::Decimal,
    /// `None` when the venue did not report a fee — never recorded as zero.
    pub charge: Option<rust_decimal::Decimal>,
    pub decision_price: rust_decimal::Decimal,
    pub filled_at: DateTime<Utc>,
}

/// Keep a mark.
///
/// # Errors
/// A database error, or a second mark for the same bar.
pub async fn record_forward_mark(pool: &PgPool, m: &ForwardMark) -> Result<(), PgError> {
    sqlx::query(
        "INSERT INTO strategy_forward_mark (forward_deployment_id, tenant_id, bar, mark, kind) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(m.forward_deployment_id)
    .bind(&m.tenant_id)
    .bind(m.bar)
    .bind(m.mark)
    .bind(&m.kind)
    .execute(pool)
    .await?;
    Ok(())
}

/// Keep a fill.
///
/// # Errors
/// A database error, or a sequence number already used.
pub async fn record_forward_fill(pool: &PgPool, f: &ForwardFill) -> Result<(), PgError> {
    sqlx::query(
        "INSERT INTO strategy_forward_fill (forward_deployment_id, tenant_id, seq, side, filled, \
           fill_price, charge, decision_price, filled_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(f.forward_deployment_id)
    .bind(&f.tenant_id)
    .bind(f.seq)
    .bind(&f.side)
    .bind(f.filled)
    .bind(f.fill_price)
    .bind(f.charge)
    .bind(f.decision_price)
    .bind(f.filled_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// A forward deployment's marks, by bar.
///
/// # Errors
/// A database error.
pub async fn forward_marks(pool: &PgPool, forward_deployment_id: Uuid) -> Result<Vec<ForwardMark>, PgError> {
    Ok(sqlx::query_as(
        "SELECT forward_deployment_id, tenant_id, bar, mark, kind FROM strategy_forward_mark \
         WHERE forward_deployment_id = $1 ORDER BY bar",
    )
    .bind(forward_deployment_id)
    .fetch_all(pool)
    .await?)
}

/// A forward deployment's fills, in the order they were applied.
///
/// # Errors
/// A database error.
pub async fn forward_fills(pool: &PgPool, forward_deployment_id: Uuid) -> Result<Vec<ForwardFill>, PgError> {
    Ok(sqlx::query_as(
        "SELECT forward_deployment_id, tenant_id, seq, side, filled, fill_price, charge, \
                decision_price, filled_at \
           FROM strategy_forward_fill WHERE forward_deployment_id = $1 ORDER BY seq",
    )
    .bind(forward_deployment_id)
    .fetch_all(pool)
    .await?)
}
