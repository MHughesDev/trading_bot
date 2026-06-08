use axum::{http::StatusCode, response::IntoResponse};

use crate::auth::BearerToken;

/// POST /api/orders — Phase 2.
pub async fn place_order(_token: BearerToken) -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

/// GET /api/orders/:id — Phase 2.
pub async fn get_order(_token: BearerToken) -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
