use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
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
) -> Response {
    // The columns are `active` and there is no `symbol` at all. The previous query
    // asked for `symbol` and `is_active`, so every call to this endpoint returned 500
    // — and returned it silently, because the error was mapped away.
    //
    // Found by an agent run rather than by a test: the model routed correctly, called
    // this, and got back an opaque `http_500` it could do nothing with.
    let row = sqlx::query_as::<_, (String, String, String, bool, i32)>(
        "SELECT instrument_id, venue_id, asset_class, active, watermark_secs \
         FROM instruments WHERE instrument_id = $1",
    )
    .bind(&id)
    .fetch_optional(&state.pg)
    .await
    .map_err(|e| {
        // Logged, not swallowed. A 500 with no line anywhere is how a schema drift
        // survives this long.
        tracing::error!(error = %e, instrument_id = %id, "get_instrument query failed");
        e
    });

    let row = match row {
        Ok(r) => r,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "instrument_lookup_failed" })),
            )
                .into_response()
        }
    };

    match row {
        Some((instrument_id, venue_id, asset_class, active, watermark_secs)) => (
            StatusCode::OK,
            Json(json!({
                "instrument_id": instrument_id,
                "venue_id": venue_id,
                "asset_class": asset_class,
                "active": active,
                "watermark_secs": watermark_secs,
            })),
        )
            .into_response(),
        // A bare 404 here is technically true and operationally a trap, because the
        // discovery namespace holds two tools that answer different questions and
        // look like they answer the same one. `list_instruments` reports BAR
        // COVERAGE, read from the bar store; this reports the INSTRUMENT REGISTRY,
        // read from Postgres. An instrument can be in either without being in the
        // other — BTC-USD has years of bars and no registry row; AAPL has a registry
        // row and no bars.
        //
        // Found in an agent trace: the model listed instruments, picked ETH-USD from
        // the answer it had just been given, called this, got `http_404`, and spent
        // three steps retrying the same call because nothing in the reply suggested
        // anything else to do. An error a caller cannot act on costs more than the
        // call it refuses.
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "instrument_not_registered",
                "instrument_id": id,
                "detail": format!(
                    "{id} has no entry in the instrument registry. Having stored bars \
                     does not put an instrument in the registry: `list_instruments` \
                     reports bar coverage and this endpoint reports the registry, and \
                     the two populations differ."
                ),
                "fix": "Use list_instruments for coverage and get_bars for the data itself; \
                        do not retry this call with the same id.",
            })),
        )
            .into_response(),
    }
}
