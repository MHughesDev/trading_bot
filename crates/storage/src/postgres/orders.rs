//! Postgres queries for the orders table.
use sqlx::PgPool;

use super::PgError;

pub async fn count(pool: &PgPool) -> Result<i64, PgError> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*)::BIGINT FROM orders")
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}
