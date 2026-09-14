//! Apply the Postgres migrations to `DATABASE_URL`, and report where it landed.
//!
//! The platform applies migrations on boot, which is right for the platform and
//! awkward for everything else: a test fixture, a fresh developer database, or a
//! CI service container needs the schema without starting the whole stack and
//! its environment.
//!
//! Run with:
//!
//! ```text
//! DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo run -p storage --example migrate
//! ```
//!
//! It prints the version before and after, because "the migrations ran" and "the
//! migrations were already applied" look identical from the outside and only one
//! of them means anything changed.

use sqlx::{PgPool, Row};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::var("DATABASE_URL").map_err(|_| {
        anyhow::anyhow!(
            "DATABASE_URL is not set. There is no default here on purpose: a migration runner \
             that guesses its target is one that eventually guesses production."
        )
    })?;

    let pool = PgPool::connect(&url).await?;
    let before = current_version(&pool).await;

    storage::postgres::run_migrations(&pool).await?;

    let after = current_version(&pool).await;
    match (before, after) {
        (Some(b), Some(a)) if b == a => println!("already at {a}; nothing applied"),
        (Some(b), Some(a)) => println!("applied {b} -> {a}"),
        (None, Some(a)) => println!("initialised to {a}"),
        (_, None) => println!("applied, but no version table was found afterwards"),
    }
    pool.close().await;
    Ok(())
}

/// The newest applied migration, or `None` on a database that has never had one.
async fn current_version(pool: &PgPool) -> Option<i64> {
    sqlx::query("SELECT max(version) AS v FROM public._sqlx_migrations")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.try_get::<Option<i64>, _>("v").ok().flatten())
}
