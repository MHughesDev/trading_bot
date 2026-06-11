use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use domain::{
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side},
    RiskRejection,
};

use crate::{auth::BearerToken, state::AppState};

#[derive(Debug, Deserialize)]
pub struct PlaceOrderRequest {
    pub instrument_id: String,
    pub side: String,
    pub order_type: String,
    pub size: String,
    pub limit_price: Option<String>,
    pub idempotency_key: Option<uuid::Uuid>,
}

/// POST /api/orders — submit a manual order through the risk gate.
pub async fn place_order(
    _token: BearerToken,
    State(_state): State<AppState>,
    Json(req): Json<PlaceOrderRequest>,
) -> impl IntoResponse {
    // Parse side.
    let side = match req.side.as_str() {
        "buy" => Side::Buy,
        "sell" => Side::Sell,
        _ => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "side must be 'buy' or 'sell'" })),
            )
                .into_response();
        }
    };

    // Parse order type.
    let order_type = match req.order_type.as_str() {
        "market" => OrderType::Market,
        "limit" => OrderType::Limit,
        "stop_limit" => OrderType::StopLimit,
        _ => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "order_type must be 'market', 'limit', or 'stop_limit'" })),
            )
                .into_response();
        }
    };

    // Parse size.
    let size = match req.size.parse::<Size>() {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "invalid size" })),
            )
                .into_response();
        }
    };

    // Parse optional limit price.
    let limit_price = if let Some(lp_str) = req.limit_price {
        match lp_str.parse::<Price>() {
            Ok(p) => Some(p),
            Err(_) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({ "error": "invalid limit_price" })),
                )
                    .into_response();
            }
        }
    } else {
        None
    };

    // Build OrderIntent.
    let mut intent = OrderIntent::new(
        req.instrument_id.clone(),
        side,
        order_type,
        size,
        limit_price,
        None, // manual order — no strategy
    );

    // Allow caller-supplied idempotency key (for retries).
    if let Some(key) = req.idempotency_key {
        intent.idempotency_key = key;
    }

    // Position data is not yet wired (Phase 2).  Returning position=0 would let
    // risk checks pass even when the account already holds a position — a
    // false-permissive failure mode that could allow over-leverage.  Reject
    // manual orders until real position, mark price, and P&L data are available.
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": "manual order placement not yet available",
            "detail": "position and mark-price data are not yet wired (Phase 2); \
                       accepting orders with zero-position defaults would bypass \
                       position-limit checks"
        })),
    )
        .into_response()
}

/// GET /api/orders/:id — look up an order by idempotency key.
pub async fn get_order(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    let row: Option<(String, Option<String>, String, String, String)> = sqlx::query_as(
        "SELECT instrument_id, broker_order_id, side, order_type, state \
         FROM orders WHERE idempotency_key = $1",
    )
    .bind(id)
    .fetch_optional(&state.pg)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    .ok()
    .flatten();

    match row {
        Some((instrument_id, broker_order_id, side, order_type, order_state)) => (
            StatusCode::OK,
            Json(json!({
                "idempotency_key": id,
                "instrument_id": instrument_id,
                "broker_order_id": broker_order_id,
                "side": side,
                "order_type": order_type,
                "state": order_state,
            })),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[allow(dead_code)]
fn risk_rejection_response(r: &RiskRejection) -> (StatusCode, &'static str) {
    match r {
        RiskRejection::KillSwitchActive | RiskRejection::TradingDisabled => (
            StatusCode::SERVICE_UNAVAILABLE,
            "trading is currently halted",
        ),
        RiskRejection::PositionLimitExceeded { .. } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "position limit would be exceeded",
        ),
        RiskRejection::RateLimitExceeded { .. } => {
            (StatusCode::TOO_MANY_REQUESTS, "order rate limit exceeded")
        }
        RiskRejection::InstrumentInactive { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, "instrument is not active")
        }
        RiskRejection::PriceSanityFailed { .. } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "price failed sanity check",
        ),
        RiskRejection::InvalidLotSize { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, "invalid lot size")
        }
        RiskRejection::DailyLossLimitExceeded { .. } => {
            (StatusCode::SERVICE_UNAVAILABLE, "daily loss limit exceeded")
        }
        RiskRejection::TrustTierInsufficient { .. } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "data trust tier insufficient",
        ),
        RiskRejection::OutsideTradingHours { .. } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "instrument is outside its trading session",
        ),
        RiskRejection::InstrumentHalted { .. } => (
            StatusCode::SERVICE_UNAVAILABLE,
            "instrument is currently halted",
        ),
        RiskRejection::RateLimitPerSecondExceeded { .. } => (
            StatusCode::TOO_MANY_REQUESTS,
            "order rate limit (per second) exceeded",
        ),
        RiskRejection::InvalidTickSize { .. } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "limit price is not on a valid tick",
        ),
    }
}
