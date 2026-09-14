use axum::{
    extract::{FromRef, FromRequestParts},
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::state::AppState;

/// A verified bearer token with the owning user resolved from the sessions table.
///
/// `FromRequestParts` hits the sessions table on every authenticated request
/// and rejects tokens that are expired or unknown. This replaces the old
/// UUIDv5-from-token derivation (M-17 Phase 1 placeholder) which caused every
/// new login to produce a different user_id even for the same account.
#[derive(Debug, Clone)]
pub struct BearerToken {
    pub token: String,
    pub user_id: Uuid,
    /// The scopes this session carries (AGENT-001 s6, migration 0038).
    ///
    /// A web login holds `web:full` and therefore everything; a service session
    /// holds exactly what it was minted with. Carrying the scopes on the extractor
    /// is what lets a handler ask "may this caller do X" without a second query,
    /// and a handler that forgets to ask is the only remaining way past them.
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

        let token = header
            .strip_prefix("Bearer ")
            .filter(|t| !t.is_empty())
            .ok_or(Unauthorized)?
            .to_owned();

        let app = AppState::from_ref(state);

        let row: Option<(Uuid, Vec<String>)> = sqlx::query_as(
            "SELECT user_id, scopes FROM sessions WHERE token = $1 AND expires_at > now()",
        )
        .bind(&token)
        .fetch_optional(&app.pg)
        .await
        .ok()
        .flatten();

        let (user_id, scopes) = row.ok_or(Unauthorized)?;

        Ok(BearerToken {
            token,
            user_id,
            scopes,
        })
    }
}
