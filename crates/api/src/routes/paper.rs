//! Paper-trading data + admin endpoints, served straight from the internal
//! [`PaperTradingEngine`] (no venue, no Postgres).
//!
//! - `GET  /api/paper/instrument/{instrument_id}` — orders + current position
//!   for one instrument, for the trading-terminal working-orders/fills lists.
//! - `POST /api/paper/reset` — reset every paper account to its opening balance.
//! - `POST /api/paper/accounts/{asset_class}/reset` — reset one asset class.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use domain::instrument::AssetClass;
use execution::paper::ALL_ASSET_CLASSES;
use serde_json::json;

use crate::{auth::BearerToken, state::AppState};

/// Resolve an asset-class URL segment (e.g. `crypto_spot_cex`) to its enum.
fn parse_asset_class(s: &str) -> Option<AssetClass> {
    ALL_ASSET_CLASSES.into_iter().find(|ac| ac.as_str() == s)
}

/// GET /api/paper/instrument/{instrument_id}
///
/// Returns the orders the paper engine retains for this instrument (working +
/// filled + terminal) and the current net position, if any.
pub async fn get_instrument_activity(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(instrument_id): Path<String>,
) -> impl IntoResponse {
    let orders = state.paper_engine.orders_for_instrument(&instrument_id);

    // Position (if the instrument's asset class is registered and holds one).
    let position = state
        .paper_engine
        .asset_class_of(&instrument_id)
        .and_then(|ac| {
            state
                .paper_engine
                .positions(ac)
                .into_iter()
                .find(|p| p.instrument_id == instrument_id)
        })
        .map(|p| {
            json!({
                "instrument_id": p.instrument_id,
                "quantity": p.quantity,
                "average_entry_price": p.avg_entry_price.inner(),
            })
        });

    Json(json!({
        "instrument_id": instrument_id,
        "orders": orders,
        "position": position,
    }))
    .into_response()
}

/// POST /api/paper/reset — reset all paper accounts to their opening balances.
pub async fn reset_all(_token: BearerToken, State(state): State<AppState>) -> impl IntoResponse {
    state.paper_engine.reset_all();
    Json(json!({ "ok": true, "scope": "all" })).into_response()
}

/// POST /api/paper/accounts/{asset_class}/reset — reset one asset-class account.
pub async fn reset_account(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(asset_class): Path<String>,
) -> impl IntoResponse {
    match parse_asset_class(&asset_class) {
        Some(ac) => {
            state.paper_engine.reset_account(ac);
            Json(json!({ "ok": true, "scope": ac.as_str() })).into_response()
        }
        None => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": format!("unknown asset class '{asset_class}'") })),
        )
            .into_response(),
    }
}
