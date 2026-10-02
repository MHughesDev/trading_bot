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
