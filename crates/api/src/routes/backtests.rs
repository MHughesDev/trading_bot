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

use backtest::{
    BacktestRequest, BarCoverage, CollectorPlan, JobStrategy, ResolvedSpec, TimeframeExt,
};
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
    // A v3 graph — inline, or the stored document a slug names — runs on the
    // one strategy engine (Set U U-3.4). Everything v1 keeps its path below.
    let stored_graph = match (&req.graph, &req.definition, &req.strategy_ref) {
        (None, None, Some(slug)) => match stored_v3(&state, slug).await {
            Ok(g) => g,
            Err(resp) => return resp,
        },
        _ => None,
    };
    if let Some(graph) = req.graph.clone().or(stored_graph) {
        return create_v3(&state, &token, &req, graph).await;
    }

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
    // Typed parameters (v1.2) run on their declared defaults here; sweeps go
    // through Experiments/Studies where `RunConfig.params` overrides them.
    let definition = match domain::strategy_def::params::materialize(
        &definition,
        &domain::strategy_def::ParamValues::new(),
    ) {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "invalid_parameters", "message": e.to_string() })),
            )
                .into_response();
        }
    };

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
        definition: JobStrategy::V1(definition),
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

/// The stored definition `slug` names, when it is a v3 graph. `Ok(None)` for a
/// v1 definition or an unknown slug, which the v1 path reports as it always has.
async fn stored_v3(
    state: &AppState,
    slug: &str,
) -> Result<Option<domain::strategy_graph::StrategyGraph>, axum::response::Response> {
    let row: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1")
            .bind(slug)
            .fetch_optional(&state.pg)
            .await
            .ok()
            .flatten();
    match row {
        Some((json,)) if backtest::runner::is_v3(&json) => {
            serde_json::from_value(json).map(Some).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "invalid_definition", "detail": e.to_string() })),
                )
                    .into_response()
            })
        }
        _ => Ok(None),
    }
}

/// POST /api/backtests with a v3 graph: compile it — the one typechecker, the
/// same errors an author sees anywhere else — then hand it to the manager.
async fn create_v3(
    state: &AppState,
    token: &BearerToken,
    req: &BacktestRequest,
    graph: domain::strategy_graph::StrategyGraph,
) -> axum::response::Response {
    if let Err(message) = backtest::runner::compile_for_run(&graph, &backtest::run::ParamMap::new())
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "invalid_strategy", "message": message })),
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
    if graph.timeframe.timeframe() != timeframe {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "timeframe_mismatch",
                "message": format!("the strategy decides on {} bars; the backtest asked for {}",
                    graph.timeframe.key(), req.timeframe) })),
        )
            .into_response();
    }
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
    let name = req
        .name
        .clone()
        .unwrap_or_else(|| format!("{} · {} · {}", graph.name, req.instrument_id, req.timeframe));
    let spec = ResolvedSpec {
        name,
        definition: JobStrategy::V3(Box::new(graph)),
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
        Ok(id) => (
            StatusCode::CREATED,
            Json(json!({ "id": id, "engine": "strategy-engine/v3" })),
        )
            .into_response(),
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
///
/// # Why the token is here, though this handler does not read it
///
/// It did not take one until 2026-09-21, and it answered **200 with the
/// platform's entire data inventory to an anonymous caller** — every
/// instrument, timeframe, bar count and first/last timestamp held. Auth in this
/// crate is per-handler through the [`BearerToken`] extractor rather than
/// middleware, so omitting the argument is all it takes to publish a route, and
/// nothing about the signature looks wrong. Every neighbouring route answers
/// 401; this one answered everybody.
///
/// The body is not user-scoped, so the token is extracted and unused. That is
/// deliberate: the inventory is a platform fact rather than a tenant's, and the
/// requirement is *authenticated*, not *filtered*. Naming it `_token` would be
/// the same mistake one step quieter.
pub async fn coverage(State(state): State<AppState>, token: BearerToken) -> impl IntoResponse {
    let _ = &token;
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
