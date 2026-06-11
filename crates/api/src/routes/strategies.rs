use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use domain::instrument::AssetClass;
use domain::strategy_def::StrategyDefinition;
use strategy_runtime::compatibility::{
    default_provided_lanes, is_compatible, InstrumentCapabilities,
};
use strategy_runtime::manifest::compile_manifest;
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
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "validation_failed", "errors": formatted })),
            )
                .into_response()
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

// ── Apply-list (P3-T03) ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ApplyListParams {
    /// Instrument identifier (e.g. `"BTC-USD"`).
    pub instrument: Option<String>,
    /// Asset class slug (e.g. `"crypto_spot_cex"`).  When supplied the provided
    /// lane set is derived from the asset class; otherwise all lanes are assumed.
    pub asset_class: Option<String>,
}

/// GET /api/strategies/apply-list — returns strategies compatible with the
/// given instrument/asset-class combination.
///
/// Incompatible strategies are **omitted** from the response, never returned
/// with a flag (C-113/C-117).
pub async fn apply_list(
    State(state): State<AppState>,
    _token: BearerToken,
    Query(params): Query<ApplyListParams>,
) -> impl IntoResponse {
    let caps = match params.asset_class.as_deref() {
        Some(slug) => {
            // Parse the slug to an AssetClass; fall back to full lane set on
            // an unknown slug so new asset classes degrade gracefully.
            let ac = slug_to_asset_class(slug);
            InstrumentCapabilities {
                provided_lanes: match ac {
                    Some(ac) => default_provided_lanes(ac),
                    None => all_lanes(),
                },
            }
        }
        None => InstrumentCapabilities {
            provided_lanes: all_lanes(),
        },
    };

    let store = state
        .strategy_store
        .lock()
        .expect("strategy_store lock poisoned");

    let compatible: Vec<serde_json::Value> = store
        .iter()
        .filter_map(|(id, def)| {
            let manifest = compile_manifest(def);
            if is_compatible(&manifest, &caps) {
                Some(json!({
                    "id": id,
                    "strategy_id": def.strategy_id,
                    "strategy_kind": manifest.strategy_kind.as_str(),
                    "evaluation_trigger": manifest.evaluation_trigger.as_str(),
                    "required_lanes": manifest.required_lanes
                        .iter()
                        .map(|dt| dt.as_key())
                        .collect::<Vec<_>>(),
                }))
            } else {
                None
            }
        })
        .collect();

    Json(json!({ "strategies": compatible }))
}

fn slug_to_asset_class(slug: &str) -> Option<AssetClass> {
    match slug {
        "crypto_spot_cex" => Some(AssetClass::CryptoSpotCex),
        "equity" => Some(AssetClass::Equity),
        "fx" => Some(AssetClass::Fx),
        "prediction_market" => Some(AssetClass::PredictionMarket),
        "option" => Some(AssetClass::Option),
        "crypto_spot_dex" => Some(AssetClass::CryptoSpotDex),
        "perpetual_swap" => Some(AssetClass::PerpetualSwap),
        "futures_expiring" => Some(AssetClass::FuturesExpiring),
        _ => None,
    }
}

fn all_lanes() -> Vec<domain::data_type::DataType> {
    use domain::data_type::DataType;
    vec![
        DataType::MarketOhlcv,
        DataType::MarketTrade,
        DataType::MarketQuote,
        DataType::MarketFundingRate,
        DataType::MarketOpenInterest,
        DataType::PredictionMarketPrice,
        DataType::DexQuote,
        DataType::SocialPost,
        DataType::WebPageSnapshot,
        DataType::NewsArticle,
    ]
}

// ── Response shape (shared by other handlers) ─────────────────────────────────

#[derive(Debug, Serialize)]
pub struct StrategyListItem {
    pub id: Uuid,
    pub strategy_id: String,
}
