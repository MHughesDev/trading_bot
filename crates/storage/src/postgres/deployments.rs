//! The deployment record and its checkpoint (Set U U-4.2, U-4.4; migrations
//! 0142, 0143).
//!
//! Starting a deployment writes its row; stopping closes it, once. The row is
//! the writer `mlops.paper_deployment` never had: a running strategy's revision,
//! instrument, starting capital, pinned models and author, fixed at start.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

use super::PgError;

/// What starting a deployment records.
#[derive(Clone, Debug)]
pub struct NewDeployment {
    pub deployment_id: Uuid,
    pub user_id: String,
    pub strategy_name: String,
    pub revision_hash: String,
    /// The graph that runs.
    pub definition_json: serde_json::Value,
    pub instrument_id: String,
    pub starting_capital: Decimal,
    /// `[{node, model, version}]`.
    pub model_pins: serde_json::Value,
    pub started_by: String,
    /// Set T's forward-evidence record, when one sits in front.
    pub forward_deployment_id: Option<Uuid>,
}

/// A deployment as recorded.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct DeploymentRow {
    pub deployment_id: Uuid,
    pub user_id: String,
    pub strategy_name: String,
    pub revision_hash: String,
    pub definition_json: serde_json::Value,
    pub instrument_id: String,
    pub environment: String,
    pub venue: String,
    pub starting_capital: Decimal,
    pub model_pins: serde_json::Value,
    pub started_by: String,
    pub started_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
    pub stop_reason: Option<String>,
    pub forward_deployment_id: Option<Uuid>,
}

const COLUMNS: &str = "deployment_id, user_id, strategy_name, revision_hash, definition_json, \
     instrument_id, environment, venue, starting_capital, model_pins, started_by, started_at, \
     stopped_at, stop_reason, forward_deployment_id";

/// Record a paper deployment on the platform's paper venue.
///
/// # Errors
/// The database's refusal — including a second running deployment of the same
/// strategy on the same instrument for the same user.
pub async fn start(pool: &PgPool, d: &NewDeployment) -> Result<(), PgError> {
    sqlx::query(
        "INSERT INTO strategy_deployment (
             deployment_id, user_id, strategy_name, revision_hash, definition_json,
             instrument_id, environment, venue, starting_capital, model_pins, started_by,
             forward_deployment_id
         ) VALUES ($1,$2,$3,$4,$5,$6,'paper','platform_paper',$7,$8,$9,$10)",
    )
    .bind(d.deployment_id)
    .bind(&d.user_id)
    .bind(&d.strategy_name)
    .bind(&d.revision_hash)
    .bind(&d.definition_json)
    .bind(&d.instrument_id)
    .bind(d.starting_capital)
    .bind(&d.model_pins)
    .bind(&d.started_by)
    .bind(d.forward_deployment_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Close a running deployment. `false` when it was already stopped or unknown.
///
/// # Errors
/// A database error.
pub async fn stop(pool: &PgPool, deployment_id: Uuid, reason: &str) -> Result<bool, PgError> {
    let done = sqlx::query(
        "UPDATE strategy_deployment SET stopped_at = now(), stop_reason = $2 \
         WHERE deployment_id = $1 AND stopped_at IS NULL",
    )
    .bind(deployment_id)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Every deployment still running — what the platform resumes at boot.
///
/// # Errors
/// A database error.
pub async fn running(pool: &PgPool) -> Result<Vec<DeploymentRow>, PgError> {
    let sql = format!(
        "SELECT {COLUMNS} FROM strategy_deployment WHERE stopped_at IS NULL ORDER BY started_at"
    );
    Ok(sqlx::query_as(&sql).fetch_all(pool).await?)
}

/// One deployment.
///
/// # Errors
/// A database error.
pub async fn get(pool: &PgPool, deployment_id: Uuid) -> Result<Option<DeploymentRow>, PgError> {
    let sql = format!("SELECT {COLUMNS} FROM strategy_deployment WHERE deployment_id = $1");
    Ok(sqlx::query_as(&sql)
        .bind(deployment_id)
        .fetch_optional(pool)
        .await?)
}

/// A user's deployments, newest first.
///
/// # Errors
/// A database error.
pub async fn for_user(pool: &PgPool, user_id: &str) -> Result<Vec<DeploymentRow>, PgError> {
    let sql = format!(
        "SELECT {COLUMNS} FROM strategy_deployment WHERE user_id = $1 ORDER BY started_at DESC"
    );
    Ok(sqlx::query_as(&sql).bind(user_id).fetch_all(pool).await?)
}

/// The capital a deployment started with — the base its realised series is
/// measured from (reconciliation reads this rather than the current balance).
///
/// # Errors
/// A database error.
pub async fn starting_capital(
    pool: &PgPool,
    deployment_id: Uuid,
) -> Result<Option<Decimal>, PgError> {
    let row: Option<(Decimal,)> =
        sqlx::query_as("SELECT starting_capital FROM strategy_deployment WHERE deployment_id = $1")
            .bind(deployment_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(c,)| c))
}

// ── checkpoints (0143) ───────────────────────────────────────────────────────

/// A deployment's latest checkpoint.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct CheckpointRow {
    pub revision_hash: String,
    pub tick: i64,
    pub state: serde_json::Value,
    pub written_at: DateTime<Utc>,
}

/// Write a deployment's checkpoint; the latest wins.
///
/// # Errors
/// A database error.
pub async fn save_checkpoint(
    pool: &PgPool,
    deployment_id: Uuid,
    revision_hash: &str,
    tick: i64,
    state: &serde_json::Value,
) -> Result<(), PgError> {
    sqlx::query(
        "INSERT INTO strategy_checkpoint (deployment_id, revision_hash, tick, state, written_at) \
         VALUES ($1,$2,$3,$4, now()) \
         ON CONFLICT (deployment_id) DO UPDATE SET \
           revision_hash = EXCLUDED.revision_hash, tick = EXCLUDED.tick, \
           state = EXCLUDED.state, written_at = EXCLUDED.written_at",
    )
    .bind(deployment_id)
    .bind(revision_hash)
    .bind(tick)
    .bind(state)
    .execute(pool)
    .await?;
    Ok(())
}

/// A deployment's latest checkpoint, if it has one.
///
/// # Errors
/// A database error.
pub async fn load_checkpoint(
    pool: &PgPool,
    deployment_id: Uuid,
) -> Result<Option<CheckpointRow>, PgError> {
    Ok(sqlx::query_as(
        "SELECT revision_hash, tick, state, written_at FROM strategy_checkpoint WHERE deployment_id = $1",
    )
    .bind(deployment_id)
    .fetch_optional(pool)
    .await?)
}
