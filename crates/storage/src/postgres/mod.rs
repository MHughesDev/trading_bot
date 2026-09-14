//! sqlx pool + transaction helpers. (0032 seed complex strategies)
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
    #[error("configuration: {0}")]
    Config(String),
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
/// The login role the platform runs as after migrations (migration 0044).
pub const APP_LOGIN_ROLE: &str = "platform_app";

/// Enable the restricted runtime login and return a pool connected as it.
///
/// Migrations run as the owner; everything after them runs as `platform_app`, a
/// member of `app_role` with no UPDATE/DELETE on the ledger and no BYPASSRLS.
/// There is deliberately no fallback to the owner connection: a runtime that
/// could quietly run as superuser would make every grant and RLS policy optional.
///
/// # Errors
/// Missing password, a failed role update, or a failed connection.
pub async fn connect_app(owner: &PgPool, owner_url: &str, app_password: &str) -> Result<PgPool, PgError> {
    if app_password.trim().is_empty() {
        return Err(PgError::Config(
            "PLATFORM_DB_APP_PASSWORD is required: the platform refuses to run as the table owner".into(),
        ));
    }
    // ALTER ROLE cannot take a bind parameter; quote_literal does the escaping.
    let quoted: String = sqlx::query_scalar("SELECT quote_literal($1)").bind(app_password).fetch_one(owner).await?;
    sqlx::query(&format!("ALTER ROLE {APP_LOGIN_ROLE} WITH LOGIN PASSWORD {quoted}")).execute(owner).await?;
    let url = app_url(owner_url, APP_LOGIN_ROLE, app_password)?;
    let pool = PgPoolOptions::new().max_connections(20).connect(&url).await?;
    let (is_super, bypass): (bool, bool) =
        sqlx::query_as("SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user").fetch_one(&pool).await?;
    if is_super || bypass {
        return Err(PgError::Config(format!("{APP_LOGIN_ROLE} must not be superuser or BYPASSRLS")));
    }
    Ok(pool)
}

/// Replace the credentials in a Postgres URL.
///
/// # Errors
/// A URL without a scheme or host.
pub fn app_url(owner_url: &str, user: &str, password: &str) -> Result<String, PgError> {
    let (scheme, rest) = owner_url.split_once("://").ok_or_else(|| PgError::Config("database url has no scheme".into()))?;
    let host = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
    let enc = |v: &str| {
        v.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect::<String>()
    };
    Ok(format!("{scheme}://{}:{}@{host}", enc(user), enc(password)))
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    // migrations dir is embedded at compile time via sqlx::migrate!
    sqlx::migrate!("../../migrations").run(pool).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_url_swaps_credentials_and_escapes() {
        assert_eq!(
            app_url("postgres://trading:trading@localhost:5432/trading", "platform_app", "p@ss/w:rd").unwrap(),
            "postgres://platform_app:p%40ss%2Fw%3Ard@localhost:5432/trading"
        );
        assert_eq!(app_url("postgres://h:5432/db", "u", "p").unwrap(), "postgres://u:p@h:5432/db");
    }
}
