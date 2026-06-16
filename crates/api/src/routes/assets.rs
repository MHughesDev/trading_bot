use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use backtest::BarStore;
use domain::instrument::ALL_ASSET_CLASSES;
use serde_json::json;

use crate::{auth::BearerToken, state::AppState};

/// GET /api/assets — list all supported asset classes.
pub async fn list_assets(_token: BearerToken) -> impl IntoResponse {
    Json(json!({ "asset_classes": ALL_ASSET_CLASSES }))
}

/// GET /api/market/instruments — list every (instrument, timeframe) pair that
/// has stored bars in ClickHouse, with coverage stats.  Powers the AI Model
/// Studio data-selection dropdown so users only train on instruments that have
/// real history available.
pub async fn list_market_instruments(
    _token: BearerToken,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let store = BarStore::connect(&state.clickhouse_url);
    match store.list_coverage().await {
        Ok(rows) => {
            let instruments: Vec<_> = rows
                .into_iter()
                .map(|c| {
                    json!({
                        "instrument_id": c.instrument_id,
                        "timeframe": c.timeframe,
                        "bars": c.bars,
                        "first_ms": c.first_ns / 1_000_000,
                        "last_ms": c.last_ns / 1_000_000,
                    })
                })
                .collect();
            Json(json!({ "instruments": instruments })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /api/instruments/:id — fetch one instrument by its ID.
pub async fn get_instrument(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let row: Option<(String, Option<String>, String, String, bool)> = sqlx::query_as(
        "SELECT instrument_id, symbol, venue_id, asset_class, is_active \
         FROM instruments WHERE instrument_id = $1",
    )
    .bind(&id)
    .fetch_optional(&state.pg)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match row {
        Some((instrument_id, symbol, venue_id, asset_class, is_active)) => Ok(Json(json!({
            "instrument_id": instrument_id,
            "symbol": symbol,
            "venue_id": venue_id,
            "asset_class": asset_class,
            "is_active": is_active,
        }))),
        None => Err(StatusCode::NOT_FOUND),
    }
}
