//! sqlx pool + transaction helpers.
pub mod instruments;
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
