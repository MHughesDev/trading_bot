use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use chrono::Utc;
use domain::{
    instrument::{HaltPolicy, TradingSchedule},
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side},
    RiskRejection,
};
use reconciliation::freshness::is_within_trading_hours;
use risk::GateContext;
use rust_decimal::Decimal;

use crate::{auth::BearerToken, state::AppState};

#[derive(Debug, Deserialize)]
pub struct PlaceOrderRequest {
    pub instrument_id: String,
    pub side: String,
    pub order_type: String,
    /// Order quantity.  The frontend order ticket sends this as `qty`; both
    /// names are accepted.
    #[serde(alias = "qty")]
    pub size: String,
    pub limit_price: Option<String>,
    pub idempotency_key: Option<uuid::Uuid>,
    /// `"paper"` (default) or `"live"`.  Only paper is wired in-house; live is
    /// rejected until per-user broker adapters are connected.
    #[serde(default)]
    pub execution_mode: Option<String>,
}

/// POST /api/orders — submit a manual order through the risk gate.
pub async fn place_order(
    _token: BearerToken,
    State(state): State<AppState>,
    Json(req): Json<PlaceOrderRequest>,
) -> impl IntoResponse {
    // Only paper mode is wired in-house; live needs per-user broker adapters.
    let execution_mode = req.execution_mode.as_deref().unwrap_or("paper");
    if execution_mode == "live" {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "live execution is not yet available",
                "detail": "connect live venue credentials in Settings; only paper mode is wired"
            })),
        )
            .into_response();
    }

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

    // Resolve the instrument's asset class from the paper engine's registry.
    // An unregistered instrument has no in-process pipeline feeding it marks, so
    // it cannot be paper-traded — reject with a clear message rather than a mark
    // error deeper in the engine.
    let Some(asset_class) = state.paper_engine.asset_class_of(&req.instrument_id) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "instrument not initialized for trading",
                "detail": format!(
                    "no live pipeline is feeding marks for '{}'; initialize the asset first",
                    req.instrument_id
                ),
            })),
        )
            .into_response();
    };

    // A mark is required: the paper engine fills against the latest observed
    // price and rejects stale/absent marks.  Surface a precise error if the
    // feed has not produced one yet (e.g. just after startup).
    let Some(market_price) = state.paper_engine.mark(&req.instrument_id) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "market data not ready",
                "detail": format!(
                    "no mark price for '{}' yet — wait for the live feed to tick",
                    req.instrument_id
                ),
            })),
        )
            .into_response();
    };

    // Current net position for this instrument, so the risk gate's
    // position-limit check sees real exposure rather than a zero default.
    let current_position = state
        .paper_engine
        .positions(asset_class)
        .into_iter()
        .find(|p| p.instrument_id == req.instrument_id)
        .map(|p| p.quantity)
        .unwrap_or(Decimal::ZERO);

    // Resolve session / halt state from instrument metadata (P6-T03).
    // For crypto (24/7, non-haltable) the defaults remain; for equities the
    // NYSE session window and haltable policy are applied — matching the spec
    // invariant "session/halt checks live in metadata, not in core code."
    let (is_in_session, halt_policy) = {
        let row: Option<(serde_json::Value, String)> = sqlx::query_as(
            "SELECT trading_hours_json, halt_policy FROM instruments WHERE instrument_id = $1",
        )
        .bind(&req.instrument_id)
        .fetch_optional(&state.pg)
        .await
        .ok()
        .flatten();

        match row {
            Some((hours_json, hp_str)) => {
                let schedule: TradingSchedule =
                    serde_json::from_value(hours_json).unwrap_or_else(|_| TradingSchedule::always_open());
                let in_session = is_within_trading_hours(Utc::now(), &schedule);
                let hp = if hp_str == "haltable" {
                    HaltPolicy::Haltable
                } else {
                    HaltPolicy::NonHaltable
                };
                (in_session, hp)
            }
            // Instrument not in metadata table — default to 24/7 non-haltable (crypto behaviour).
            None => (true, HaltPolicy::NonHaltable),
        }
    };

    // Manual-order risk context.  tick_size and lot_size are 0 — which disables
    // those two checks (a nonzero lot would wrongly reject fractional crypto like
    // 0.01 BTC).  Daily-loss is 0 until P&L wiring lands.
    let mut ctx = GateContext::for_manual_order(
        current_position,
        Some(market_price),
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        true,
        0,
        0,
    );
    ctx.is_in_session = is_in_session;
    ctx.halt_policy = halt_policy;
    // is_halted defaults to false — we do not yet subscribe to exchange halt events,
    // so the gate catches session violations but not intra-session halts at this phase.

    match state.risk_gate.check(intent, &ctx) {
        Ok(approved) => {
            let idempotency_key = approved.intent.idempotency_key;
            match state.execution.submit(approved).await {
                Ok(result) => (
                    StatusCode::OK,
                    Json(json!({
                        "status": "submitted",
                        "broker_order_id": result.broker_order_id,
                        "idempotency_key": idempotency_key,
                        "execution_mode": "paper",
                    })),
                )
                    .into_response(),
                Err(e) => (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({ "error": "order submission failed", "detail": e.to_string() })),
                )
                    .into_response(),
            }
        }
        Err(rejection) => {
            let (status, message) = risk_rejection_response(&rejection);
            (status, Json(json!({ "error": message }))).into_response()
        }
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
