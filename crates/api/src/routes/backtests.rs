//! Backtesting HTTP API.
//!
//! Endpoints for the "Back Testing" UI: create runs, list/poll them as tiles,
//! and drive the per-tile quick actions (stop, rerun, delete).  All heavy
//! lifting lives in the `backtest` crate; these handlers only resolve the
//! strategy definition and translate manager results into HTTP responses.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use backtest::{BacktestRequest, BarCoverage, CollectorPlan, ResolvedSpec, TimeframeExt};
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
        "crypto_spot_cex" | "perpetual_swap" => "coinbase",
        _ => "coinbase",
    }
}

fn timeframe_from_key(key: &str) -> Option<Timeframe> {
    <Timeframe as TimeframeExt>::from_key(key)
}

/// POST /api/backtests — create and start a backtest run.
pub async fn create_backtest(
    State(state): State<AppState>,
    token: BearerToken,
    Json(req): Json<BacktestRequest>,
) -> impl IntoResponse {
    // Resolve the strategy definition: inline body or stored slug.
    let definition: StrategyDefinition = match (&req.definition, &req.strategy_ref) {
        (Some(def), _) => def.clone(),
        (None, Some(slug)) => {
            let row: Option<(serde_json::Value,)> = sqlx::query_as(
                "SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1",
            )
            .bind(slug)
            .fetch_optional(&state.pg)
            .await
            .ok()
            .flatten();

            match row {
                Some((def_json,)) => match serde_json::from_value::<StrategyDefinition>(def_json) {
                    Ok(def) => def,
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({ "error": "invalid_definition", "detail": e.to_string() })),
                        )
                            .into_response();
                    }
                },
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({ "error": "strategy_not_found", "strategy_ref": slug })),
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

    // When auto-collect is on, reject asset-class/timeframe combinations that
    // have no collector up front (422) rather than letting the job fail later
    // in the CollectingData phase (#15).
    if req.auto_collect {
        if let Err(message) = CollectorPlan::auto_collect_support(&req.asset_class, timeframe) {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "unsupported_auto_collect", "message": message })),
            )
                .into_response();
        }
    }

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

    match state.backtest.create(token.user_id(), spec).await {
        Ok(id) => (StatusCode::CREATED, Json(json!({ "id": id }))).into_response(),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "invalid_request", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// Pagination for the list endpoint.
#[derive(Debug, Deserialize)]
pub struct ListParams {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

/// Largest page the list endpoint will return in one response.
const MAX_LIST_LIMIT: usize = 100;
const DEFAULT_LIST_LIMIT: usize = 50;

/// GET /api/backtests — list this user's runs as tiles (newest first), paged.
pub async fn list_backtests(
    State(state): State<AppState>,
    token: BearerToken,
    Query(params): Query<ListParams>,
) -> impl IntoResponse {
    let limit = params
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);
    let offset = params.offset.unwrap_or(0);

    let all = state.backtest.list(token.user_id()).await;
    let total = all.len();
    let page: Vec<_> = all.into_iter().skip(offset).take(limit).collect();
    Json(json!({
        "backtests": page,
        "total": total,
        "limit": limit,
        "offset": offset,
    }))
}

/// GET /api/backtests/:id — full snapshot for one run (progress, result, error).
pub async fn get_backtest(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.backtest.get(token.user_id(), id).await {
        Some(snap) => Json(snap).into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response(),
    }
}

/// POST /api/backtests/:id/stop — request cancellation of a running job.
pub async fn stop_backtest(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    map_action(state.backtest.stop(token.user_id(), id).await)
}

/// POST /api/backtests/:id/rerun — start a fresh run with the same spec.
pub async fn rerun_backtest(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.backtest.rerun(token.user_id(), id).await {
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
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    map_action(state.backtest.delete(token.user_id(), id).await)
}

/// GET /api/backtests/coverage — instruments that have bar data in ClickHouse.
///
/// Powers the create-backtest guardrail: the UI fetches this once and warns
/// when the user types an instrument with no stored history.
pub async fn coverage(State(state): State<AppState>) -> impl IntoResponse {
    match state.backtest.available_instruments().await {
        Ok(entries) => {
            let items: Vec<serde_json::Value> = entries
                .iter()
                .map(|c: &BarCoverage| {
                    json!({
                        "instrument_id": c.instrument_id,
                        "timeframe":     c.timeframe,
                        "bars":          c.bars,
                        "first_ns":      c.first_ns,
                        "last_ns":       c.last_ns,
                    })
                })
                .collect();
            Json(json!({ "coverage": items })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "coverage_unavailable", "message": e.to_string() })),
        )
            .into_response(),
    }
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
            (
                code,
                Json(json!({ "error": "action_failed", "message": msg })),
            )
                .into_response()
        }
    }
}
