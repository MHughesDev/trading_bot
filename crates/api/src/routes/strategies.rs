use std::sync::Arc;

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
use strategy_validator::validate;

use crate::{auth::BearerToken, state::AppState};

// ── Create ────────────────────────────────────────────────────────────────────

/// POST /api/strategies — validate then persist a strategy definition.
pub async fn create_strategy(
    State(state): State<AppState>,
    _token: BearerToken,
    Json(def): Json<StrategyDefinition>,
) -> impl IntoResponse {
    match validate(&def) {
        Err(errors) => {
            let formatted: Vec<serde_json::Value> = errors
                .iter()
                .map(|e| json!({ "path": e.path, "message": e.message }))
                .collect();
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "validation_failed", "errors": formatted })),
            )
                .into_response();
        }
        Ok(validated) => {
            let id = Uuid::new_v4();
            let mut store = state
                .strategy_store
                .lock()
                .expect("strategy_store lock poisoned");
            store.insert(id, validated.into_inner());
            (
                StatusCode::CREATED,
                Json(json!({ "id": id, "strategy_id": def.strategy_id })),
            )
                .into_response()
        }
    }
}

// ── List ──────────────────────────────────────────────────────────────────────

/// GET /api/strategies — list all persisted strategy definitions.
pub async fn list_strategies(
    State(state): State<AppState>,
    _token: BearerToken,
) -> impl IntoResponse {
    let store = state
        .strategy_store
        .lock()
        .expect("strategy_store lock poisoned");
    let list: Vec<serde_json::Value> = store
        .iter()
        .map(|(id, def)| json!({ "id": id, "strategy_id": def.strategy_id }))
        .collect();
    Json(json!({ "strategies": list }))
}

// ── Get ───────────────────────────────────────────────────────────────────────

/// GET /api/strategies/:id/config — fetch a strategy definition by store ID.
pub async fn get_strategy(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let store = state
        .strategy_store
        .lock()
        .expect("strategy_store lock poisoned");
    match store.get(&id) {
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "id": id })),
        )
            .into_response(),
        Some(def) => Json(json!({ "id": id, "definition": def })).into_response(),
    }
}

// ── Start ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StartRequest {
    pub user_id: String,
    pub instrument_id: String,
}

/// POST /api/strategies/:id/start — initialize a strategy instance on an instrument.
pub async fn start_strategy(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<Uuid>,
    Json(req): Json<StartRequest>,
) -> impl IntoResponse {
    let def = {
        let store = state
            .strategy_store
            .lock()
            .expect("strategy_store lock poisoned");
        match store.get(&id).cloned() {
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "error": "not_found", "id": id })),
                )
                    .into_response()
            }
            Some(d) => d,
        }
    };

    let clock: Arc<dyn strategy_runtime::StrategyClock> = state.clock.clone();
    let mut manager = state
        .instance_manager
        .lock()
        .expect("instance_manager lock poisoned");

    match manager.initialize(&req.user_id, &req.instrument_id, def, &clock) {
        Ok(()) => (
            StatusCode::CREATED,
            Json(json!({
                "strategy_store_id": id,
                "user_id": req.user_id,
                "instrument_id": req.instrument_id,
                "status": "running"
            })),
        )
            .into_response(),
        Err(strategy_runtime::RuntimeError::AlreadyRunning {
            user_id,
            instrument_id,
        }) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "already_running",
                "user_id": user_id,
                "instrument_id": instrument_id
            })),
        )
            .into_response(),
    }
}

// ── Stop ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StopRequest {
    pub user_id: String,
    pub instrument_id: String,
}

/// POST /api/strategies/:id/stop — stop a running strategy instance.
pub async fn stop_strategy(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(_id): Path<Uuid>,
    Json(req): Json<StopRequest>,
) -> impl IntoResponse {
    let mut manager = state
        .instance_manager
        .lock()
        .expect("instance_manager lock poisoned");
    manager.stop(&req.user_id, &req.instrument_id);
    Json(json!({
        "user_id": req.user_id,
        "instrument_id": req.instrument_id,
        "status": "stopped"
    }))
}

// ── Response shape (shared by other handlers) ─────────────────────────────────

#[derive(Debug, Serialize)]
pub struct StrategyListItem {
    pub id: Uuid,
    pub strategy_id: String,
}
