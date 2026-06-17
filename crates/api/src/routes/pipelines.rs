//! Pipeline & monitoring REST routes (I-5.12, Phase 5).
//!
//! POST   /api/pipelines                          — create
//! GET    /api/pipelines                          — list
//! GET    /api/pipelines/{id}                     — get
//! DELETE /api/pipelines/{id}                     — delete
//! POST   /api/pipelines/{id}/run                 — run (may fan-out)
//! POST   /api/pipelines/{id}/runs/{run_id}/cancel — cancel
//! GET    /api/pipelines/{id}/runs                — list runs
//! GET    /api/pipelines/runs/{run_id}            — get run
//! GET    /api/pipelines/runs/{run_id}/nodes      — get node runs
//! GET    /api/models/{id}/quality                — rolling quality series + alerts

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use model_registry::pipeline_manager::{CreatePipelineRequest, RunPipelineRequest};

use crate::auth::BearerToken;
use crate::state::AppState;

fn bad(msg: impl ToString) -> impl IntoResponse {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg.to_string() })),
    )
        .into_response()
}

fn not_found(msg: impl ToString) -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg.to_string() })),
    )
        .into_response()
}

fn internal(msg: impl ToString) -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": msg.to_string() })),
    )
        .into_response()
}

// ── Create ────────────────────────────────────────────────────────────────────

pub async fn create_pipeline(
    State(state): State<AppState>,
    token: BearerToken,
    Json(req): Json<CreatePipelineRequest>,
) -> impl IntoResponse {
    let uid = token.user_id().to_string();
    match state.pipelines.clone().create_pipeline(req, &uid).await {
        Ok(rec) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(rec).unwrap()),
        )
            .into_response(),
        Err(e) => bad(e).into_response(),
    }
}

// ── List ──────────────────────────────────────────────────────────────────────

pub async fn list_pipelines(
    State(state): State<AppState>,
    token: BearerToken,
) -> impl IntoResponse {
    let uid = token.user_id().to_string();
    match state.pipelines.clone().list_pipelines(&uid).await {
        Ok(recs) => Json(serde_json::to_value(recs).unwrap()).into_response(),
        Err(e) => internal(e).into_response(),
    }
}

// ── Get ───────────────────────────────────────────────────────────────────────

pub async fn get_pipeline(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uid = token.user_id().to_string();
    match state.pipelines.clone().get_pipeline(&id, &uid).await {
        Ok(rec) => Json(serde_json::to_value(rec).unwrap()).into_response(),
        Err(e) => not_found(e).into_response(),
    }
}

// ── Delete ────────────────────────────────────────────────────────────────────

pub async fn delete_pipeline(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uid = token.user_id().to_string();
    match state.pipelines.clone().delete_pipeline(&id, &uid).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => not_found(e).into_response(),
    }
}

// ── Run ───────────────────────────────────────────────────────────────────────

pub async fn run_pipeline(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
    Json(req): Json<RunPipelineRequest>,
) -> impl IntoResponse {
    let uid = token.user_id().to_string();
    match state.pipelines.clone().run_pipeline(&id, &uid, req).await {
        Ok(result) => (
            StatusCode::ACCEPTED,
            Json(serde_json::to_value(result).unwrap()),
        )
            .into_response(),
        Err(e) => bad(e).into_response(),
    }
}

// ── Cancel run ────────────────────────────────────────────────────────────────

pub async fn cancel_run(
    State(state): State<AppState>,
    _token: BearerToken,
    Path((_id, run_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match state.pipelines.clone().cancel_run(&run_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => internal(e).into_response(),
    }
}

// ── List runs ─────────────────────────────────────────────────────────────────

pub async fn list_runs(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uid = token.user_id().to_string();
    match state.pipelines.clone().list_runs(&id, &uid).await {
        Ok(runs) => Json(serde_json::to_value(runs).unwrap()).into_response(),
        Err(e) => not_found(e).into_response(),
    }
}

// ── Get run ───────────────────────────────────────────────────────────────────

pub async fn get_run(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    match state.pipelines.clone().get_run(&run_id).await {
        Ok(run) => Json(serde_json::to_value(run).unwrap()).into_response(),
        Err(e) => not_found(e).into_response(),
    }
}

// ── Node runs ─────────────────────────────────────────────────────────────────

pub async fn list_node_runs(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    match state.pipelines.clone().list_node_runs(&run_id).await {
        Ok(nodes) => Json(serde_json::to_value(nodes).unwrap()).into_response(),
        Err(e) => not_found(e).into_response(),
    }
}

// ── Quality series (I-5.9/I-5.10) ────────────────────────────────────────────

pub async fn get_model_quality(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(model_id): Path<String>,
) -> impl IntoResponse {
    let series = state
        .quality_monitor
        .get_quality_series(&model_id, 200)
        .await
        .unwrap_or_default();
    let alerts = state
        .quality_monitor
        .get_alerts(&model_id, 50)
        .await
        .unwrap_or_default();

    Json(serde_json::json!({
        "model_id": model_id,
        "series": series,
        "alerts": alerts,
    }))
    .into_response()
}
