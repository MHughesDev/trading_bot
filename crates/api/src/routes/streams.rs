use axum::{extract::State, response::IntoResponse, Json};
use domain::lanes::ALL_LANES;
use serde::Deserialize;
use serde_json::json;

use crate::auth::BearerToken;
use crate::state::AppState;

/// GET /api/streams/available — list all available NATS data streams.
pub async fn list_available(_token: BearerToken) -> impl IntoResponse {
    Json(json!({ "streams": ALL_LANES }))
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
            &req.panel_id,
            &user_id,
            &spec.lane,
            &spec.instrument,
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
