use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use domain::strategy_def::StrategyDefinition;
use market_simulator_adapter::results::placeholder_report;

use crate::{
    auth::BearerToken,
    state::{AppState, BacktestJobState},
};

/// Request body for `POST /api/backtests`.
#[derive(Debug, Deserialize)]
pub struct BacktestRequest {
    /// Full strategy definition JSON (must be valid v1.0).
    pub strategy_definition: StrategyDefinition,
    pub instrument_id: String,
    pub start_capital: f64,
    /// ISO-8601 start date, e.g. `"2024-01-01T00:00:00Z"`.
    pub range_start: String,
    /// ISO-8601 end date, e.g. `"2024-06-01T00:00:00Z"`.
    pub range_end: String,
}

/// POST /api/backtests — accept a backtest job and return a job ID.
pub async fn run_backtest(
    State(state): State<AppState>,
    _token: BearerToken,
    Json(req): Json<BacktestRequest>,
) -> impl IntoResponse {
    // Validate definition version (fail-closed on unknown versions).
    if req.strategy_definition.definition_version != "1.0" {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "unsupported_version",
                "message": format!(
                    "unknown definition_version '{}'",
                    req.strategy_definition.definition_version
                )
            })),
        )
            .into_response();
    }

    let job_id = Uuid::new_v4();

    // Optimistically mark as pending.
    {
        let mut jobs = state
            .backtest_jobs
            .lock()
            .expect("backtest_jobs lock poisoned");
        jobs.insert(job_id, BacktestJobState::Pending);
    }

    // Run the backtest asynchronously.
    let jobs_ref = state.backtest_jobs.clone();
    let strategy_id = req.strategy_definition.strategy_id.clone();
    let instrument_id = req.instrument_id.clone();

    tokio::spawn(async move {
        // Phase 4 MVP: produce a placeholder report.
        // Production wiring: export bars from storage → call market_simulator adapter.
        let report = placeholder_report(&strategy_id, &instrument_id);

        let mut jobs = jobs_ref.lock().expect("backtest_jobs lock poisoned");
        jobs.insert(job_id, BacktestJobState::Completed(report));
    });

    (
        StatusCode::ACCEPTED,
        Json(json!({ "job_id": job_id })),
    )
        .into_response()
}

/// GET /api/backtests/{id} — fetch a backtest result by job ID.
pub async fn get_backtest(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let jobs = state
        .backtest_jobs
        .lock()
        .expect("backtest_jobs lock poisoned");

    match jobs.get(&id) {
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "job_id": id })),
        )
            .into_response(),
        Some(BacktestJobState::Pending) => {
            Json(json!({ "job_id": id, "status": "pending" })).into_response()
        }
        Some(BacktestJobState::Completed(report)) => {
            Json(json!({ "job_id": id, "status": "completed", "report": report }))
                .into_response()
        }
        Some(BacktestJobState::Failed(err)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "job_id": id, "status": "failed", "error": err })),
        )
            .into_response(),
    }
}

/// Response shape for strategy start/stop endpoints (Phase 4).
#[derive(Debug, Serialize)]
pub struct StrategyInstanceResponse {
    pub instance_id: Uuid,
    pub user_id: String,
    pub instrument_id: String,
    pub status: String,
}
