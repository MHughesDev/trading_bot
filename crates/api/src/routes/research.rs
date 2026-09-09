//! Research HTTP API (FEAT-003 Phase 1): sweeps, diagnostics, and the one
//! carry-forward read. Thin wrappers over [`crate::research::ResearchManager`]
//! and the suite; every row is user-scoped by the bearer token.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::research::{StartError, StartSweepBody};
use crate::{auth::BearerToken, state::AppState};

/// POST /api/research/sweeps — start a sweep on an Experiment (202 + id).
pub async fn start_sweep(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<StartSweepBody>,
) -> impl IntoResponse {
    match state.research.start(token.user_id(), body) {
        Ok(id) => (
            StatusCode::ACCEPTED,
            Json(json!({ "sweep_id": id, "status": "queued" })),
        )
            .into_response(),
        Err(StartError::ExperimentNotFound) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "experiment not found" })),
        )
            .into_response(),
        Err(StartError::Busy) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "busy", "message": StartError::Busy.to_string() })),
        )
            .into_response(),
        Err(e @ StartError::InvalidRequest(_)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "invalid_request", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /api/research/sweeps — this user's sweeps, newest first.
pub async fn list_sweeps(State(state): State<AppState>, token: BearerToken) -> impl IntoResponse {
    Json(json!({ "sweeps": state.research.list(token.user_id()) }))
}

/// GET /api/research/sweeps/:id — status, progress and (when done) the report.
pub async fn get_sweep(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.research.get(token.user_id(), id) {
        Some(s) => Json(s).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "sweep not found" })),
        )
            .into_response(),
    }
}

/// POST /api/research/sweeps/:id/cancel — honoured between batches.
pub async fn cancel_sweep(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if state.research.cancel(token.user_id(), id) {
        Json(json!({ "sweep_id": id, "cancel_requested": true })).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "sweep not found" })),
        )
            .into_response()
    }
}

/// GET /api/research/diagnostics/:run_id — the diagnostic bundle for a Run
/// the user reached through one of their Studies.
pub async fn get_diagnostics(
    State(state): State<AppState>,
    token: BearerToken,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    match state.research.diagnostics(token.user_id(), &run_id) {
        Some(b) => Json(b).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "run not found in any of your studies" })),
        )
            .into_response(),
    }
}

/// GET /api/backtest/experiments/:id/studies/:study_id/carried-forward — the
/// parameter set a Study's pre-declared selection rule carried forward.
pub async fn get_carried_forward(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, study_id)): Path<(Uuid, String)>,
) -> impl IntoResponse {
    match state.suite.study_carried_forward(token.user_id(), id, &study_id) {
        Some(params) => Json(json!({ "study_id": study_id, "carried_forward": params })).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "not_found",
                "message": "no carried-forward parameters for that study (no selection rule, or study not found)"
            })),
        )
            .into_response(),
    }
}
