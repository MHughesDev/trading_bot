use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

use crate::{auth::BearerToken, state::AppState};

/// GET /api/trading/status — current kill switch / trading state.
pub async fn trading_status(
    _token: BearerToken,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let halted = state.kill_switch.is_active();
    Json(json!({
        "trading_enabled": !halted,
        "kill_switch_active": halted,
    }))
}

/// POST /api/trading/kill — manually trip the kill switch.
pub async fn trip_kill_switch(
    _token: BearerToken,
    State(state): State<AppState>,
) -> impl IntoResponse {
    state.kill_switch.trip();
    (
        StatusCode::OK,
        Json(json!({ "message": "kill switch tripped — all new orders halted" })),
    )
}

/// POST /api/trading/resume — reset the kill switch (use with care).
pub async fn reset_kill_switch(
    _token: BearerToken,
    State(state): State<AppState>,
) -> impl IntoResponse {
    state.kill_switch.reset();
    (
        StatusCode::OK,
        Json(json!({ "message": "kill switch reset — order flow resumed" })),
    )
}
