//! Backtest-suite HTTP API (Set J, J-5.4).
//!
//! REST surface over the honest-evaluation core: experiments, their research
//! studies, the null picker, the staged-gate funnel, the one-shot vault, and the
//! reconciliation/suite-calibration read-outs. All rows are user-scoped by the
//! bearer-token-derived id (MASTER §8); the heavy lifting lives in
//! `backtest::suite::SuiteManager`, and these handlers only translate manager
//! results into HTTP responses. Progress streams over the `/ws/backtest-suite`
//! lane.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use backtest::nulls::NullKind;
use backtest::suite::{CreateExperimentSpec, RunStudySpec, SuiteError};

use crate::{auth::BearerToken, state::AppState};

/// Translate a [`SuiteError`] into an HTTP response with a stable error code.
fn map_err(e: SuiteError) -> axum::response::Response {
    use backtest::experiment::ExperimentError;
    let (code, kind) = match &e {
        SuiteError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
        SuiteError::AlreadyExists => (StatusCode::CONFLICT, "already_exists"),
        SuiteError::Experiment(ExperimentError::VaultSpent) => {
            (StatusCode::CONFLICT, "vault_spent")
        }
        SuiteError::Experiment(_) => (StatusCode::CONFLICT, "operation_not_allowed"),
        SuiteError::Gate(_) => (StatusCode::UNPROCESSABLE_ENTITY, "gate_error"),
        SuiteError::Study(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_study"),
        SuiteError::Null(_) => (StatusCode::UNPROCESSABLE_ENTITY, "null_error"),
        SuiteError::Invalid(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"),
    };
    (
        code,
        Json(json!({ "error": kind, "message": e.to_string() })),
    )
        .into_response()
}

// ── experiments ───────────────────────────────────────────────────────────────

/// POST /api/backtest/experiments — create a candidate Experiment.
pub async fn create_experiment(
    State(state): State<AppState>,
    token: BearerToken,
    Json(spec): Json<CreateExperimentSpec>,
) -> impl IntoResponse {
    match state.suite.create_experiment(token.user_id(), spec) {
        Ok(view) => (StatusCode::CREATED, Json(view)).into_response(),
        Err(e) => map_err(e),
    }
}

/// GET /api/backtest/experiments — list this user's Experiments (newest first).
pub async fn list_experiments(
    State(state): State<AppState>,
    token: BearerToken,
) -> impl IntoResponse {
    let views = state.suite.list_experiments(token.user_id());
    Json(json!({ "experiments": views }))
}

/// GET /api/backtest/experiments/:id — one Experiment header.
pub async fn get_experiment(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.get_experiment(token.user_id(), id) {
        Some(view) => Json(view).into_response(),
        None => map_err(SuiteError::NotFound),
    }
}

/// POST /api/backtest/experiments/:id/promote — validated → live.
pub async fn promote_experiment(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.promote_to_live(token.user_id(), id) {
        Ok(view) => Json(view).into_response(),
        Err(e) => map_err(e),
    }
}

/// POST /api/backtest/experiments/:id/retire — terminal.
pub async fn retire_experiment(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.retire(token.user_id(), id) {
        Ok(view) => Json(view).into_response(),
        Err(e) => map_err(e),
    }
}

// ── studies ────────────────────────────────────────────────────────────────

/// GET /api/backtest/experiments/:id/studies — sealed Study distributions.
pub async fn list_studies(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.list_studies(token.user_id(), id) {
        Some(studies) => Json(json!({ "studies": studies })).into_response(),
        None => map_err(SuiteError::NotFound),
    }
}

/// POST /api/backtest/experiments/:id/studies — run a research Study.
pub async fn run_study(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(spec): Json<RunStudySpec>,
) -> impl IntoResponse {
    match state.suite.run_study(token.user_id(), id, spec) {
        Ok(view) => (StatusCode::CREATED, Json(view)).into_response(),
        Err(e) => map_err(e),
    }
}

// ── nulls ──────────────────────────────────────────────────────────────────

/// GET /api/backtest/experiments/:id/nulls — recommendation + catalog + choice.
pub async fn null_picker(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.null_picker(token.user_id(), id) {
        Some(view) => Json(view).into_response(),
        None => map_err(SuiteError::NotFound),
    }
}

#[derive(Debug, Deserialize)]
pub struct ChooseNullBody {
    pub kind: NullKind,
    #[serde(default)]
    pub override_reason: Option<String>,
}

/// POST /api/backtest/experiments/:id/nulls — choose the significance null.
pub async fn choose_null(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(body): Json<ChooseNullBody>,
) -> impl IntoResponse {
    match state
        .suite
        .choose_null(token.user_id(), id, body.kind, body.override_reason)
    {
        Ok(choice) => Json(choice).into_response(),
        Err(e) => map_err(e),
    }
}

// ── gate funnel ──────────────────────────────────────────────────────────────

/// GET /api/backtest/experiments/:id/funnel — the gate-funnel board.
pub async fn get_funnel(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.funnel(token.user_id(), id) {
        Some(view) => Json(view).into_response(),
        None => map_err(SuiteError::NotFound),
    }
}

/// POST /api/backtest/experiments/:id/funnel/advance — drive Gates 0→3.
pub async fn advance_funnel(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.advance_funnel(token.user_id(), id) {
        Ok(view) => Json(view).into_response(),
        Err(e) => map_err(e),
    }
}

// ── vault ──────────────────────────────────────────────────────────────────

/// GET /api/backtest/experiments/:id/vault — one-shot state + access log.
pub async fn get_vault(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.vault(token.user_id(), id) {
        Some(view) => Json(view).into_response(),
        None => map_err(SuiteError::NotFound),
    }
}

/// POST /api/backtest/experiments/:id/vault — spend the holdout vault (once).
pub async fn run_vault(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    // The vault access log records who touched it; the bearer-derived id is the
    // stable per-user actor (M-17 placeholder auth).
    let by = token.user_id().to_string();
    match state.suite.run_vault(token.user_id(), id, by) {
        Ok(view) => Json(view).into_response(),
        Err(e) => map_err(e),
    }
}

// ── reconciliation ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ReconcileBody {
    /// Realized per-period performance series (live ledger input).
    pub realized: Vec<f64>,
    /// Share of periods below the planned worst-5% that flags decay (default 0.10).
    #[serde(default = "default_drift")]
    pub drift_threshold: f64,
}

fn default_drift() -> f64 {
    0.10
}

/// POST /api/backtest/experiments/:id/reconcile — live vs backtest distribution.
pub async fn reconcile(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(body): Json<ReconcileBody>,
) -> impl IntoResponse {
    match state
        .suite
        .reconcile(token.user_id(), id, &body.realized, body.drift_threshold)
    {
        Ok(view) => Json(view).into_response(),
        Err(e) => map_err(e),
    }
}

/// GET /api/backtest/calibration — the suite-calibration meta-view.
pub async fn suite_calibration(
    State(state): State<AppState>,
    token: BearerToken,
) -> impl IntoResponse {
    Json(state.suite.suite_calibration(token.user_id()))
}
