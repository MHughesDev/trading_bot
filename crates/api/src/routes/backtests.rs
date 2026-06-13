//! Backtesting HTTP API.
//!
//! Endpoints for the "Back Testing" UI: create runs, list/poll them as tiles,
//! and drive the per-tile quick actions (stop, rerun, delete).  All heavy
//! lifting lives in the `backtest` crate; these handlers only resolve the
//! strategy definition and translate manager results into HTTP responses.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use backtest::{BacktestRequest, ResolvedSpec, TimeframeExt};
use domain::payloads::bar::Timeframe;
use domain::strategy_def::StrategyDefinition;
use strategy_validator::validate;

use crate::{auth::BearerToken, state::AppState};

/// Default venue name per asset class for display + collection routing.
fn default_venue(asset_class: &str) -> &'static str {
    match asset_class {
        "equity" | "etf" => "alpaca",
        "fx" => "oanda",
        "futures_expiring" => "cme",
        "option" => "opra",
        "prediction_market" => "kalshi",
        _ => "binance",
    }
}

fn timeframe_from_key(key: &str) -> Option<Timeframe> {
    <Timeframe as TimeframeExt>::from_key(key)
}

/// POST /api/backtests — create and start a backtest run.
pub async fn create_backtest(
    State(state): State<AppState>,
    _token: BearerToken,
    Json(req): Json<BacktestRequest>,
) -> impl IntoResponse {
    // Resolve the strategy definition: inline body or stored UUID.
    let definition: StrategyDefinition = match (&req.definition, req.strategy_ref) {
        (Some(def), _) => def.clone(),
        (None, Some(id)) => {
            let store = state
                .strategy_store
                .lock()
                .expect("strategy_store lock poisoned");
            match store.get(&id) {
                Some(def) => def.clone(),
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({ "error": "strategy_not_found", "strategy_ref": id })),
                    )
                        .into_response();
                }
            }
        }
        (None, None) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "missing_strategy",
                    "message": "provide either `definition` or `strategy_ref`" })),
            )
                .into_response();
        }
    };

    if let Err(errors) = validate(&definition) {
        let formatted: Vec<serde_json::Value> = errors
            .iter()
            .map(|e| json!({ "path": e.path, "message": e.message }))
            .collect();
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "invalid_strategy", "errors": formatted })),
        )
            .into_response();
    }

    let Some(timeframe) = timeframe_from_key(&req.timeframe) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "invalid_timeframe",
                "message": "timeframe must be one of 1s,1m,5m,15m,1h,4h,1d" })),
        )
            .into_response();
    };

    let venue_id = req
        .venue_id
        .clone()
        .unwrap_or_else(|| default_venue(&req.asset_class).to_string());
    let name = req.name.clone().unwrap_or_else(|| {
        format!(
            "{} · {} · {}",
            definition.strategy_id, req.instrument_id, req.timeframe
        )
    });

    let spec = ResolvedSpec {
        name,
        definition,
        instrument_id: req.instrument_id.clone(),
        venue_id,
        asset_class: req.asset_class.clone(),
        timeframe,
        start: req.start,
        end: req.end,
        initial_balance: req.initial_balance.clone(),
        quote_currency: req.quote_currency.clone(),
        auto_collect: req.auto_collect,
    };

    match state.backtest.create(spec).await {
        Ok(id) => (StatusCode::CREATED, Json(json!({ "id": id }))).into_response(),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "invalid_request", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /api/backtests — list all runs as tiles (newest first).
pub async fn list_backtests(State(state): State<AppState>, _token: BearerToken) -> impl IntoResponse {
    let runs = state.backtest.list().await;
    Json(json!({ "backtests": runs }))
}

/// GET /api/backtests/:id — full snapshot for one run (progress, result, error).
pub async fn get_backtest(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.backtest.get(id).await {
        Some(snap) => Json(snap).into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response(),
    }
}

/// POST /api/backtests/:id/stop — request cancellation of a running job.
pub async fn stop_backtest(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    map_action(state.backtest.stop(id).await)
}

/// POST /api/backtests/:id/rerun — start a fresh run with the same spec.
pub async fn rerun_backtest(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.backtest.rerun(id).await {
        Ok(new_id) => (StatusCode::CREATED, Json(json!({ "id": new_id }))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "rerun_failed", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// DELETE /api/backtests/:id — remove a finished run.
pub async fn delete_backtest(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    map_action(state.backtest.delete(id).await)
}

fn map_action<E: std::fmt::Display>(result: Result<(), E>) -> axum::response::Response {
    match result {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(e) => {
            let msg = e.to_string();
            let code = if msg.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::CONFLICT
            };
            (code, Json(json!({ "error": "action_failed", "message": msg }))).into_response()
        }
    }
}
