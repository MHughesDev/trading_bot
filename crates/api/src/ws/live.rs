use axum::{http::StatusCode, response::IntoResponse};

use crate::auth::BearerToken;

/// GET /ws/live — WebSocket upgrade for live data streaming.  Phase 3.
pub async fn ws_live(_token: BearerToken) -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
