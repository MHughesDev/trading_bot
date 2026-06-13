use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use domain::instrument::ALL_ASSET_CLASSES;
use serde_json::json;

use crate::{auth::BearerToken, state::AppState};

/// GET /api/assets — list all supported asset classes.
pub async fn list_assets(_token: BearerToken) -> impl IntoResponse {
    Json(json!({ "asset_classes": ALL_ASSET_CLASSES }))
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
