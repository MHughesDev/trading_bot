//! sqlx pool + transaction helpers.
pub mod instruments;
pub mod models;
pub mod orders;
pub mod strategies;
pub mod users;

use sqlx::postgres::PgPoolOptions;
use thiserror::Error;

pub type PgPool = sqlx::PgPool;

#[derive(Debug, Error)]
pub enum PgError {
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
}

pub async fn connect(database_url: &str) -> Result<PgPool, PgError> {
    Ok(PgPoolOptions::new()
        .max_connections(20)
        .connect(database_url)
        .await?)
}

/// Applies all pending SQL migrations from the repo `migrations/` directory.
///
/// Embedded at compile time and run on startup so a fresh database always has
/// the current schema (e.g. `backtest_runs`) without a manual migration step
/// (#20).  Idempotent: already-applied migrations are skipped.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("../../migrations").run(pool).await
}
