//! Session resolution: the one place a presented bearer token becomes a user.
//!
//! # What is stored (Set E 1.1, migration 0150)
//!
//! `sessions.token` holds `session_token_hash(token)` — the hex SHA-256 of the
//! credential — and never the credential. The database computes the digest: a
//! BEFORE INSERT trigger replaces whatever is inserted with its hash, so every
//! writer inserts the credential it minted and no writer can store cleartext.
//! Lookups compare `token = session_token_hash($1)`.
//!
//! [`BearerToken::token`] is therefore the **session key** (the digest), not the
//! credential. Every caller that reads `sessions` by `token = $1` with it gets
//! the right row; nothing downstream of the extractor ever holds the secret.
//!
//! # Expiry
//!
//! * A **web** session slides: each use moves `expires_at` to
//!   `now() + WEB_IDLE_TTL`, capped at `created_at + WEB_ABSOLUTE_TTL`. A
//!   session nobody uses for a week dies; one used every day dies at 30 days.
//!   The write is throttled to once per [`TOUCH_EVERY_SECS`] per session, so a
//!   busy page does not write a row per request.
//! * A **service** session never slides. It was minted with the lifetime it was
//!   meant to have, and an agent that keeps calling must not keep itself alive.
//! * Expired rows are swept (see [`maybe_sweep`]) — lazily, from the extractor,
//!   so the sweep is reached by every authenticated request rather than by a
//!   background task someone has to remember to spawn.

use std::sync::atomic::{AtomicI64, Ordering};

use axum::{
    extract::{FromRef, FromRequestParts},
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::state::AppState;

/// How long an unused web session lives.
pub const WEB_IDLE_TTL_DAYS: i64 = 7;
/// The most a web session can live, however often it is used.
pub const WEB_ABSOLUTE_TTL_DAYS: i64 = 30;
/// Minimum gap between two sliding-expiry writes for one session.
pub const TOUCH_EVERY_SECS: i64 = 60;
/// Minimum gap between two expired-session sweeps in one process.
pub const SWEEP_EVERY_SECS: i64 = 600;

/// The lookup. `session_token_hash` is the migration-0150 SQL function; the
/// token presented is hashed by the database, which is the only definition of
/// the key.
pub const SESSION_LOOKUP_SQL: &str =
    "SELECT token, user_id, scopes, kind, last_seen_at, expires_at \
     FROM sessions WHERE token = session_token_hash($1) AND expires_at > now()";

/// The sliding-expiry write for a web session. Never extends past the absolute
/// cap, and never shortens an expiry (`GREATEST`).
pub const SESSION_TOUCH_SQL: &str = "UPDATE sessions \
        SET last_seen_at = now(), \
            expires_at = GREATEST(expires_at, LEAST( \
                now() + make_interval(days => $2), \
                created_at + make_interval(days => $3))) \
      WHERE token = $1 AND kind = 'web' AND expires_at > now() \
      RETURNING expires_at";

/// The sweep.
pub const SESSION_SWEEP_SQL: &str = "DELETE FROM sessions WHERE expires_at <= now()";

/// A verified bearer token with the owning user resolved from the sessions table.
#[derive(Debug, Clone)]
pub struct BearerToken {
    /// The session **key**: `sessions.token`, the SHA-256 hex of the presented
    /// credential. Not the credential — the credential is never kept after the
    /// lookup. Bind this to `WHERE token = $1` to read this session's row.
    pub token: String,
    pub user_id: Uuid,
    /// The scopes this session carries (AGENT-001 s6, migration 0038).
    ///
    /// A web login holds `web:full` and therefore everything; a service session
    /// holds exactly what it was minted with.
    pub scopes: Vec<String>,
}

impl BearerToken {
    #[must_use]
    pub fn user_id(&self) -> Uuid {
        self.user_id
    }

    /// Whether this token permits `required`.
    #[must_use]
    pub fn permits(&self, required: &str) -> bool {
        super::scopes::permits(&self.scopes, required)
    }
}

/// A session row the lookup found.
#[derive(Debug, Clone)]
pub struct ResolvedSession {
    /// `sessions.token` — the digest.
    pub key: String,
    pub user_id: Uuid,
    pub scopes: Vec<String>,
    pub kind: String,
    pub last_seen_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

type SessionRow = (
    String,
    Uuid,
    Vec<String>,
    String,
    DateTime<Utc>,
    DateTime<Utc>,
);

/// Resolve a presented credential to its live session, sliding a web session's
/// expiry as a side effect. `Ok(None)` is unknown or expired.
///
/// # Errors
/// A database error. Every caller treats it as a refusal.
pub async fn resolve(pg: &PgPool, presented: &str) -> Result<Option<ResolvedSession>, sqlx::Error> {
    if presented.is_empty() {
        return Ok(None);
    }
    let row: Option<SessionRow> = sqlx::query_as(SESSION_LOOKUP_SQL)
        .bind(presented)
        .fetch_optional(pg)
        .await?;
    let Some((key, user_id, scopes, kind, last_seen_at, expires_at)) = row else {
        maybe_sweep(pg);
        return Ok(None);
    };
    let mut session = ResolvedSession {
        key,
        user_id,
        scopes,
        kind,
        last_seen_at,
        expires_at,
    };
    if session.kind == "web"
        && (Utc::now() - session.last_seen_at).num_seconds() >= TOUCH_EVERY_SECS
    {
        // A failed touch does not refuse the request: the session was valid
        // when read. It only means the expiry did not move this time.
        match touch_key(pg, &session.key).await {
            Ok(Some(expires_at)) => {
                session.expires_at = expires_at;
                session.last_seen_at = Utc::now();
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "session touch failed; expiry not extended"),
        }
    }
    maybe_sweep(pg);
    Ok(Some(session))
}

/// Slide one web session's expiry now. `Ok(None)` for a service session or a
/// session that expired in the meantime.
///
/// # Errors
/// A database error.
pub async fn touch_key(pg: &PgPool, key: &str) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    sqlx::query_scalar(SESSION_TOUCH_SQL)
        .bind(key)
        .bind(i32::try_from(WEB_IDLE_TTL_DAYS).unwrap_or(7))
        .bind(i32::try_from(WEB_ABSOLUTE_TTL_DAYS).unwrap_or(30))
        .fetch_optional(pg)
        .await
}

/// Delete every expired session. Returns how many went.
///
/// # Errors
/// A database error.
pub async fn sweep_expired(pg: &PgPool) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(SESSION_SWEEP_SQL)
        .execute(pg)
        .await?
        .rows_affected())
}

static LAST_SWEEP: AtomicI64 = AtomicI64::new(0);

/// Run [`sweep_expired`] in the background if this process has not swept in the
/// last [`SWEEP_EVERY_SECS`]. Called from every session lookup, so the sweep is
/// reached for as long as anyone is calling the API; the compare-exchange makes
/// it one sweep per window however many requests race.
pub fn maybe_sweep(pg: &PgPool) {
    let now = Utc::now().timestamp();
    let last = LAST_SWEEP.load(Ordering::Relaxed);
    if now - last < SWEEP_EVERY_SECS {
        return;
    }
    if LAST_SWEEP
        .compare_exchange(last, now, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let pg = pg.clone();
    tokio::spawn(async move {
        match sweep_expired(&pg).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(swept = n, "expired sessions removed"),
            Err(e) => tracing::warn!(error = %e, "expired-session sweep failed"),
        }
    });
}

#[derive(Debug)]
pub struct Unauthorized;

impl IntoResponse for Unauthorized {
    fn into_response(self) -> Response {
        (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response()
    }
}

impl<S> FromRequestParts<S> for BearerToken
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = Unauthorized;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(Unauthorized)?;

        let presented = header
            .strip_prefix("Bearer ")
            .filter(|t| !t.is_empty())
            .ok_or(Unauthorized)?;

        let app = AppState::from_ref(state);

        // A database error is a refusal: an outage must not authenticate.
        let session = resolve(&app.pg, presented)
            .await
            .ok()
            .flatten()
            .ok_or(Unauthorized)?;

        Ok(BearerToken {
            token: session.key,
            user_id: session.user_id,
            scopes: session.scopes,
        })
    }
}
