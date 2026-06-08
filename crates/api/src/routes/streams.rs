use axum::{extract::State, response::IntoResponse, Json};
use serde::Deserialize;
use serde_json::json;

use crate::auth::BearerToken;
use crate::state::AppState;

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
        // Virtual UI lanes
        "ui.orderbook.snapshot",
    ];
    Json(json!({ "streams": streams }))
}

#[derive(Deserialize)]
pub struct SubSpecBody {
    pub lane: String,
    pub instrument: String,
    pub depth: Option<u32>,
    pub max_fps: Option<u32>,
}

#[derive(Deserialize)]
pub struct CreateSubscriptionRequest {
    pub panel_id: String,
    pub subscribe: Vec<SubSpecBody>,
}

/// POST /api/ui/subscriptions — register panel subscriptions via REST.
pub async fn create_ui_subscriptions(
    State(state): State<AppState>,
    token: BearerToken,
    Json(req): Json<CreateSubscriptionRequest>,
) -> impl IntoResponse {
    let user_id = token.0;
    let mut registered = Vec::new();
    let mut errors = Vec::new();

    for spec in &req.subscribe {
        match state.gateway.subscribe(
            req.panel_id.clone(),
            user_id.clone(),
            &spec.lane,
            spec.instrument.clone(),
            &user_id,
            spec.depth,
            spec.max_fps,
        ) {
            Ok(sub) => registered.push(json!({
                "sub_id": sub.id,
                "panel_id": sub.panel_id,
                "lane": sub.lane,
                "instrument": sub.instrument,
            })),
            Err(e) => errors.push(json!({
                "lane": spec.lane,
                "instrument": spec.instrument,
                "error": e.to_string(),
            })),
        }
    }

    Json(json!({
        "subscriptions": registered,
        "errors": errors,
    }))
}
