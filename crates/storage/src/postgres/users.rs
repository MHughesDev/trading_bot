//! Postgres queries for the users table.
//! placeholder — to be filled in Phase 2.
use sqlx::PgPool;

use super::PgError;

pub async fn count(pool: &PgPool) -> Result<i64, PgError> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*)::BIGINT FROM users")
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}
