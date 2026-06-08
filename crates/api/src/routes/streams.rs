use axum::{response::IntoResponse, Json};
use serde_json::json;

use crate::auth::BearerToken;

/// GET /api/streams/available — list all available NATS data streams.
pub async fn list_available(_token: BearerToken) -> impl IntoResponse {
    let streams = [
        "market.trades",
        "market.quotes",
        "market.orderbook.l2",
        "market.bars.1s",
        "market.bars.1m",
        "market.bars.1m.revised",
        "features.technical",
        "strategy.signals",
        "orders.commands",
        "orders.events",
        "positions.events",
    ];
    Json(json!({ "streams": streams }))
}
