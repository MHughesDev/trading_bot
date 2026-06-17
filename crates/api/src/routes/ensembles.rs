//! Ensemble API routes (I-4.3 / Phase 4).
//!
//! Mirrors the /api/models/** shape for ensembles:
//!   POST   /api/ensembles                    — create
//!   GET    /api/ensembles                    — list
//!   GET    /api/ensembles/{id}               — get
//!   POST   /api/ensembles/{id}/combine       — drive combine (dispatches sidecar)
//!   GET    /api/ensembles/{id}/versions      — list versions
//!   POST   /api/ensembles/{id}/versions/{v}/promote/{alias}  — promote

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::collections::HashMap;

use model_registry::ensemble_manager::CreateEnsembleRequest;

use crate::auth::BearerToken;
use crate::state::AppState;

// ── Create ────────────────────────────────────────────────────────────────────

pub async fn create_ensemble(
    State(state): State<AppState>,
    token: BearerToken,
    Json(req): Json<CreateEnsembleRequest>,
) -> impl IntoResponse {
    let uid = token.user_id().to_string();
    match state.ensembles.clone().create_ensemble(req, &uid).await {
        Ok(rec) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(rec).unwrap()),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── List ──────────────────────────────────────────────────────────────────────

pub async fn list_ensembles(
    State(state): State<AppState>,
    token: BearerToken,
) -> impl IntoResponse {
    let uid = token.user_id().to_string();
    match state.ensembles.clone().list_ensembles(&uid).await {
        Ok(recs) => Json(serde_json::to_value(recs).unwrap()).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── Get ───────────────────────────────────────────────────────────────────────

pub async fn get_ensemble(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uid = token.user_id().to_string();
    match state.ensembles.clone().get_ensemble(&id, &uid).await {
        Ok(rec) => Json(serde_json::to_value(rec).unwrap()).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── Combine ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CombineRequest {
    pub dataset_uri: String,
    pub dataset_hash: String,
    #[serde(default)]
    pub cal_start: usize,
    #[serde(default)]
    pub cal_end: usize,
    #[serde(default)]
    pub member_sigmas: HashMap<String, f64>,
    #[serde(default)]
    pub member_crps: HashMap<String, f64>,
}

pub async fn combine_ensemble(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
    Json(req): Json<CombineRequest>,
) -> impl IntoResponse {
    let uid = token.user_id().to_string();
    match state
        .ensembles
        .clone()
        .drive_combine(
            &id,
            &uid,
            &req.dataset_uri,
            &req.dataset_hash,
            req.cal_start,
            req.cal_end,
            req.member_sigmas,
            req.member_crps,
        )
        .await
    {
        Ok(ver) => Json(serde_json::to_value(ver).unwrap()).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── List versions ─────────────────────────────────────────────────────────────

pub async fn list_ensemble_versions(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let uid = token.user_id().to_string();
    match state
        .ensembles
        .clone()
        .list_ensemble_versions(&id, &uid)
        .await
    {
        Ok(vers) => Json(serde_json::to_value(vers).unwrap()).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── Promote ───────────────────────────────────────────────────────────────────

pub async fn promote_ensemble_version(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, version, alias)): Path<(String, i32, String)>,
) -> impl IntoResponse {
    let uid = token.user_id().to_string();
    match state
        .ensembles
        .clone()
        .promote_ensemble(&id, version, &alias, &uid)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
