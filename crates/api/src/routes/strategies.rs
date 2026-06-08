use axum::{http::StatusCode, response::IntoResponse};

use crate::auth::BearerToken;

/// POST /api/strategies — Phase 2.
pub async fn create_strategy(_token: BearerToken) -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

/// GET /api/strategies — Phase 2.
pub async fn list_strategies(_token: BearerToken) -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
