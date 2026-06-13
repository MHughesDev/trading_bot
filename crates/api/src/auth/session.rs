use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};

/// A bearer token extracted from `Authorization: Bearer <token>`.
///
/// # Security note (M-17)
/// Currently accepts any non-empty token without validation against the user
/// store — this is Phase 1 placeholder behaviour and **must not be deployed on
/// a network-accessible endpoint**.  Phase 2 upgrade: validate the token with
/// constant-time comparison against a session table and populate a real user
/// identity.  Tracked as an open security item.
#[derive(Debug, Clone)]
pub struct BearerToken(pub String);

impl BearerToken {
    /// A stable per-token user identity.
    ///
    /// Until real session validation lands (the M-17 upgrade), the token *is*
    /// the identity: the same token always maps to the same `user_id` and two
    /// different tokens map to two different ids.  That is enough to scope
    /// resources (e.g. backtests) to their creator and stop cross-user
    /// visibility/control.  Derived as a UUIDv5 so it is deterministic and
    /// carries no secret material.
    #[must_use]
    pub fn user_id(&self) -> uuid::Uuid {
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, self.0.as_bytes())
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
{
    type Rejection = Unauthorized;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(Unauthorized)?;

        let token = header
            .strip_prefix("Bearer ")
            .filter(|t| !t.is_empty())
            .ok_or(Unauthorized)?;

        Ok(BearerToken(token.to_owned()))
    }
}
