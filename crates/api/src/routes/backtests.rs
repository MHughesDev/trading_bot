use axum::{http::StatusCode, response::IntoResponse};

use crate::auth::BearerToken;

/// POST /api/backtests — Phase 3.
pub async fn run_backtest(_token: BearerToken) -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
