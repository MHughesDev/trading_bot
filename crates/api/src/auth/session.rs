use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};

/// A bearer token extracted from `Authorization: Bearer <token>`.
///
/// Phase 1: presence of any non-empty token is accepted.  Phase 2 will verify
/// against the users table and populate a real user identity.
#[derive(Debug, Clone)]
pub struct BearerToken(pub String);

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
