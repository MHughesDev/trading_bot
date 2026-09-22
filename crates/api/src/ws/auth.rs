//! Token verification for the WebSocket surface.
//!
//! # Why this is not just `BearerToken`
//!
//! A browser's `WebSocket` constructor cannot set request headers, so both
//! sockets receive their bearer token as a `?token=` query parameter, and
//! [`crate::auth::BearerToken`] — which reads `Authorization` and nothing else —
//! cannot be used on them.
//!
//! That constraint is about *where the token arrives*. It says nothing about
//! whether the token is checked, and for a long time the two were confused:
//! `ws_live` used the raw query string as the user id, and `ws_backtest_suite`
//! ran it through a UUIDv5 derivation of the same string. Neither read the
//! `sessions` table, so **any non-empty string was a user** on both — one of
//! which carries the live and paper data feed.
//!
//! [`WsBearerToken`] is `BearerToken`'s sibling for that one difference: it
//! takes the token from the query string and then resolves it against
//! `sessions` with the same predicate the REST surface uses. Everything else —
//! the table, the expiry check, the refusal — is the same.
//!
//! # Why an extractor and not a call in the handler
//!
//! Because the argument is the proof. A socket added without a verified
//! identity has to be a socket whose signature is missing this type, which is
//! the one thing `every_mounted_route_refuses_an_anonymous_caller` reads. A
//! resolver called from inside the body can be forgotten by the next handler
//! and nothing says so.
//!
//! # Refusal is the only failure mode
//!
//! There is no fallback path, no dev-mode bypass and no flag. A token that is
//! absent, empty, unknown or expired is refused, and so is a token the database
//! could not be asked about: an outage must not open the socket.

use axum::{
    extract::{FromRef, FromRequestParts, Query},
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use sqlx::PgPool;
use tracing::warn;
use uuid::Uuid;

use crate::state::AppState;

/// The query string a WebSocket upgrade carries its credential in.
#[derive(Debug, Deserialize)]
pub struct WsQuery {
    /// Bearer token passed as a query parameter (the browser WS API cannot set
    /// headers). Absent and empty are the same thing here, and both are refused.
    pub token: Option<String>,
}

/// A `?token=` value resolved to the session that owns it.
///
/// Holding one of these means the `sessions` table was read and answered. There
/// is no constructor that skips that.
#[derive(Debug, Clone)]
pub struct WsBearerToken {
    pub token: String,
    pub user_id: Uuid,
    /// The scopes this session carries (AGENT-001 s6, migration 0038).
    pub scopes: Vec<String>,
}

impl WsBearerToken {
    #[must_use]
    pub fn user_id(&self) -> Uuid {
        self.user_id
    }

    /// Whether this token permits `required`.
    #[must_use]
    pub fn permits(&self, required: &str) -> bool {
        crate::auth::scopes::permits(&self.scopes, required)
    }
}

/// Why an upgrade was refused. The body is deliberately coarse: a caller
/// learning *which* of absent, unknown or expired applies learns whether a
/// guessed token exists.
#[derive(Debug)]
pub struct WsUnauthorized;

impl IntoResponse for WsUnauthorized {
    fn into_response(self) -> Response {
        (StatusCode::UNAUTHORIZED, "missing or invalid token query param").into_response()
    }
}

/// Resolve a `?token=` value against the `sessions` table.
///
/// Split out from the extractor so it can be exercised against a real database
/// without standing up an `AppState`; the extractor is a thin wrapper over it.
///
/// Every `Err` is a refusal — including the database error branch.
pub async fn resolve_ws_session(
    pg: &PgPool,
    token: Option<&str>,
) -> Result<WsBearerToken, WsUnauthorized> {
    let token = token.filter(|t| !t.is_empty()).ok_or(WsUnauthorized)?;

    // The same predicate `BearerToken` uses. `sessions.expires_at` is
    // `TIMESTAMPTZ NOT NULL` (migration 0013), so there is no null-expiry row
    // for this to miss.
    let row: Option<(Uuid, Vec<String>)> = sqlx::query_as(
        "SELECT user_id, scopes FROM sessions WHERE token = $1 AND expires_at > now()",
    )
    .bind(token)
    .fetch_optional(pg)
    .await
    .map_err(|e| {
        // An unreachable database is a refusal, not an exemption.
        warn!(error = %e, "ws: session lookup failed; refusing the upgrade");
        WsUnauthorized
    })?;

    let (user_id, scopes) = row.ok_or(WsUnauthorized)?;

    Ok(WsBearerToken {
        token: token.to_owned(),
        user_id,
        scopes,
    })
}

impl<S> FromRequestParts<S> for WsBearerToken
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = WsUnauthorized;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(query) = Query::<WsQuery>::from_request_parts(parts, state)
            .await
            .map_err(|_| WsUnauthorized)?;

        let app = AppState::from_ref(state);
        resolve_ws_session(&app.pg, query.token.as_deref()).await
    }
}
