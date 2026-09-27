//! Research HTTP API (FEAT-003 Phase 1): sweeps, diagnostics, and the one
//! carry-forward read. Thin wrappers over [`crate::research::ResearchManager`]
//! and the suite; every row is user-scoped by the bearer token.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
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
    match state.research.diagnostics(token.user_id(), &run_id).await {
        Some(b) => Json(b).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(
                json!({ "error": "not_found", "message": "run not found in any of your studies" }),
            ),
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

/// GET /api/backtest/experiments/:id/returns — a recorded family's return
/// series, as JSON.
///
/// # Why this route exists
///
/// `SuiteManager::recorded_returns` has been reachable in exactly one way: the
/// template materialiser writes it to a **parquet file** on the cache root for a
/// Python skill to read. That is right for a template and useless for anything
/// else, and it left `ablation_sweep` and `parameter_fragility_map` (Set S
/// S-3.13) with nothing to diff against the null library's confidence interval —
/// the phase file records them as `BLOCKED:UPSTREAM` for exactly this reason:
/// *"no route returns a run's return series as JSON"*.
///
/// The timestamps are carried, not dropped, and that is the point of returning
/// the family rather than a bare matrix. `recorded_family` says why in its own
/// comment: without them a family is an integer-indexed matrix, and an integer
/// index cannot be aligned to anything outside itself — no factor series, no
/// market regime, no second family. Every attribution tool needs the alignment.
///
/// One member per recorded run with at least two returns, deduplicated across
/// studies, in the order the studies recorded them.
pub async fn get_experiment_returns(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let family = match state.suite.recorded_returns(token.user_id(), id) {
        Ok(f) => f,
        // `NotFound` is the only error this call can return today, and it is
        // matched by name rather than caught as a catch-all so it stays that way.
        // Mapping every future error to 404 is how `/api/evidence/trades` came to
        // report "no trades" for a table migration 0026 had renamed: a 404 for a
        // real failure is a lie the caller cannot distinguish from an answer.
        Err(backtest::suite::SuiteError::NotFound) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": "not_found",
                    "message": "no experiment of yours with that id",
                    "experiment_id": id.to_string(),
                })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "suite_error",
                    "message": e.to_string(),
                    "experiment_id": id.to_string(),
                })),
            )
                .into_response();
        }
    };

    // An empty family is 422 and not an empty 200, and the wording is the
    // materialiser's own: nothing was dispatched to find this out, so the caller
    // has not learned that the runs produced no returns — it has learned that no
    // run happened. A caller reading `[]` from a 200 would conclude the first.
    if family.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "empty_family",
                "message": "this experiment has no recorded run with a usable return series. \
                            Nothing was dispatched to find that out — run the family first.",
                "experiment_id": id.to_string(),
            })),
        )
            .into_response();
    }

    let members: Vec<Value> = family
        .into_iter()
        .map(|(run_id, stamps, returns)| {
            json!({
                "run_id": run_id,
                // RFC-3339 so the alignment survives the wire as text rather than
                // as an epoch a reader has to guess the unit of.
                "at": stamps.iter().map(|t| t.to_rfc3339()).collect::<Vec<_>>(),
                "returns": returns,
            })
        })
        .collect();

    Json(json!({
        "experiment_id": id.to_string(),
        "members": members.len(),
        // Named `daily_returns` because that is what `recorded_family` computes
        // (`daily_returns` over the equity curve). A consumer that assumed
        // per-bar returns would misscale every statistic it took.
        "series": "daily_returns",
        "family": members,
    }))
    .into_response()
}
