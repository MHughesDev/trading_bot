use axum::{http::StatusCode, response::IntoResponse};

use crate::auth::BearerToken;

/// GET /api/trading/status — Phase 2.
pub async fn trading_status(_token: BearerToken) -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
