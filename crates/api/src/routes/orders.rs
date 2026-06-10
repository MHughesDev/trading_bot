use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;

use domain::{
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side},
    RiskRejection,
};
use risk::GateContext;

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
    State(state): State<AppState>,
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

    // Build a minimal gate context.
    // In production, current_position and market_price would be fetched from
    // Postgres / Redis.  For Phase 2, we use conservative defaults.
    let ctx = GateContext::for_manual_order(
        Decimal::ZERO,      // TODO(P2+): fetch from DB
        None,               // TODO(P2+): fetch from Redis
        Decimal::new(1, 2), // 0.01 default tick
        Decimal::new(1, 3), // 0.001 default lot
        Decimal::ZERO,      // TODO(P2+): fetch realized P&L for today
        true,               // TODO(P2+): check instrument active in DB
        0,
        0,
    );

    // Run through the risk gate.
    let approved = match state.risk_gate.check(intent, &ctx) {
        Ok(a) => a,
        Err(rejection) => {
            let (status, msg) = risk_rejection_response(&rejection);
            return (
                status,
                Json(json!({ "error": msg, "reason": rejection.to_string() })),
            )
                .into_response();
        }
    };

    let idempotency_key = approved.intent.idempotency_key;

    // Submit to execution engine.
    match state.execution.submit(approved).await {
        Ok(result) => (
            StatusCode::CREATED,
            Json(json!({
                "idempotency_key": idempotency_key,
                "broker_order_id": result.broker_order_id,
                "state": "submitted",
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": "execution failed", "detail": e.to_string() })),
        )
            .into_response(),
    }
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
