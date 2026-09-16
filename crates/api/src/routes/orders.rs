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


/// Parse the inbound `side` string.
///
/// Exactly two spellings are accepted.  The sibling of this function inside
/// `execution` used to read `if s == "buy" { Buy } else { Sell }`, so anything
/// unexpected became a **sell** — a position-reducing or short-opening order
/// built from an input the platform could not read.  There is no safe default
/// for a direction: a wrong side is not a smaller trade, it is the opposite
/// trade.
#[must_use]
pub fn parse_side(s: &str) -> Option<Side> {
    match s {
        "buy" => Some(Side::Buy),
        "sell" => Some(Side::Sell),
        _ => None,
    }
}

/// Parse the inbound `order_type` string.  No default: a market order fills
/// immediately at whatever the book offers, which is the last thing an
/// unreadable input should become.
#[must_use]
pub fn parse_order_type(s: &str) -> Option<OrderType> {
    match s {
        "market" => Some(OrderType::Market),
        "limit" => Some(OrderType::Limit),
        "stop_limit" => Some(OrderType::StopLimit),
        _ => None,
    }
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

    // Parse side.  An unrecognised side is a refusal, never a default.
    let Some(side) = parse_side(&req.side) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "side must be 'buy' or 'sell'",
                "received": req.side,
            })),
        )
            .into_response();
    };

    // Parse order type.  An unrecognised type must never fall through to
    // `market`, the one type that executes immediately at any price.
    let Some(order_type) = parse_order_type(&req.order_type) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "order_type must be 'market', 'limit', or 'stop_limit'",
                "received": req.order_type,
            })),
        )
            .into_response();
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

    // The risk context.  One provider serves this door and the hot path's
    // stage 4, so the two cannot drift: the position, the mark, the venue's
    // published tick and lot, today's loss and the order-rate counts are all
    // measured from live state, and anything that cannot be measured refuses
    // here instead of defaulting.
    //
    // What this replaced, and why it mattered:
    //
    //     GateContext::for_manual_order(pos, mark, ZERO, ZERO, ZERO, true, 0, 0)
    //
    // A zero tick and lot disable the tick and lot checks.  A zero daily loss
    // cannot breach a daily-loss limit.  `0, 0` order counts cannot breach a
    // rate limit.  Three of the ten checks in COMP-002 3.2 reported a pass they
    // had never evaluated, and nothing downstream could tell the difference.
    let ctx = match risk::context::context_for(&intent) {
        Ok(ctx) => ctx,
        Err(rejection) => {
            let (status, message) = risk_rejection_response(&rejection);
            return (
                status,
                Json(json!({ "error": message, "detail": rejection.to_string() })),
            )
                .into_response();
        }
    };

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

/// GET /api/orders/:id — look up an order by idempotency key, with its fills.
///
/// This endpoint could not succeed before migration 0121 and
/// `execution::journal`: nothing in the repository wrote to `orders`, so every
/// lookup was a 404.  The fills are returned alongside because "did it fill,
/// and at what?" is the question the endpoint is asked, and because
/// reconciliation and Gate 15 read exactly these rows.
pub async fn get_order(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    #[allow(clippy::type_complexity)]
    let row: Option<(
        String,
        Option<String>,
        String,
        String,
        String,
        rust_decimal::Decimal,
        Option<String>,
        String,
    )> = sqlx::query_as(
        "SELECT instrument_id, broker_order_id, side, order_type, state, size, \
                strategy_id, execution_mode \
         FROM orders WHERE idempotency_key = $1",
    )
    .bind(id)
    .fetch_optional(&state.pg)
    .await
    .ok()
    .flatten();

    let Some((
        instrument_id,
        broker_order_id,
        side,
        order_type,
        order_state,
        size,
        strategy_id,
        execution_mode,
    )) = row
    else {
        return StatusCode::NOT_FOUND.into_response();
    };

    #[allow(clippy::type_complexity)]
    let fills: Vec<(
        String,
        rust_decimal::Decimal,
        rust_decimal::Decimal,
        Option<rust_decimal::Decimal>,
        chrono::DateTime<chrono::Utc>,
    )> =
        sqlx::query_as(
            "SELECT broker_order_id, filled_size, fill_price, commission, filled_at \
             FROM fills WHERE idempotency_key = $1 ORDER BY filled_at ASC",
        )
        .bind(id)
        .fetch_all(&state.pg)
        .await
        .unwrap_or_default();

    (
        StatusCode::OK,
        Json(json!({
            "idempotency_key": id,
            "instrument_id": instrument_id,
            "broker_order_id": broker_order_id,
            "side": side,
            "order_type": order_type,
            "state": order_state,
            "size": size.to_string(),
            "strategy_id": strategy_id,
            "execution_mode": execution_mode,
            "fills": fills.iter().map(|(boid, qty, price, commission, at)| json!({
                "broker_order_id": boid,
                "filled_size": qty.to_string(),
                "fill_price": price.to_string(),
                // `null`, not "0", when the venue did not report a fee.
                "commission": commission.map(|c| c.to_string()),
                "filled_at": at,
            })).collect::<Vec<_>>(),
        })),
    )
        .into_response()
}

fn risk_rejection_response(r: &RiskRejection) -> (StatusCode, &'static str) {
    match r {
        RiskRejection::KillSwitchActive | RiskRejection::TradingDisabled => (
            StatusCode::SERVICE_UNAVAILABLE,
            "trading is currently halted",
        ),
        // COMP-007 §4. Worded distinctly from a halt on purpose: a halt is a
        // decision somebody recorded, this is the monitor being unreachable, and
        // an operator who cannot tell them apart will go looking for the wrong
        // thing to reset.
        RiskRejection::MonitorUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "monitor_unavailable — no live monitor lease; new orders are refused",
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

        // The fail-closed refusals.  Each one is the gate saying "I could not
        // evaluate this", which is a 503-class answer — the request was not
        // wrong, the platform was not ready to judge it.  Returning 200 here,
        // or skipping the check, is the defect these variants exist to prevent.
        RiskRejection::MarketDataUnavailable { .. } => (
            StatusCode::SERVICE_UNAVAILABLE,
            "no current mark for this instrument — wait for the live feed to tick",
        ),
        RiskRejection::InstrumentMetadataUnavailable { .. } => (
            StatusCode::SERVICE_UNAVAILABLE,
            "the venue's tick and lot for this instrument are not known — initialize the asset",
        ),
        RiskRejection::AccountStateUnavailable { .. } => (
            StatusCode::SERVICE_UNAVAILABLE,
            "account state could not be read — the daily-loss limit cannot be evaluated",
        ),
        // A divergence halt is a refusal about one strategy, not about the
        // platform: the venue is up and every other strategy still trades. 409
        // rather than 503 says exactly that, and says it is not worth retrying
        // until a human clears the halt.
        RiskRejection::StrategyHalted { .. } => (
            StatusCode::CONFLICT,
            "this strategy is halted after a divergence; a human must clear it",
        ),
        RiskRejection::GateUnavailable { .. } => (
            StatusCode::SERVICE_UNAVAILABLE,
            "the risk gate cannot evaluate orders right now",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unparseable_side_produces_no_order() {
        // Every one of these used to become a SELL in `execution::side_from_str`.
        for bad in [
            "", " ", "BUY", "Buy", "b", "sel", "sell ", "long", "short", "0", "null",
        ] {
            assert!(
                parse_side(bad).is_none(),
                "side '{bad}' must be refused, not read as a sell"
            );
        }
        assert_eq!(parse_side("buy"), Some(Side::Buy));
        assert_eq!(parse_side("sell"), Some(Side::Sell));
    }

    #[test]
    fn an_unparseable_order_type_produces_no_order() {
        for bad in ["", "MARKET", "mkt", "stop", "stoplimit", "trailing"] {
            assert!(
                parse_order_type(bad).is_none(),
                "order_type '{bad}' must be refused, not read as a market order"
            );
        }
        assert_eq!(parse_order_type("market"), Some(OrderType::Market));
        assert_eq!(parse_order_type("limit"), Some(OrderType::Limit));
        assert_eq!(parse_order_type("stop_limit"), Some(OrderType::StopLimit));
    }

    /// Every fail-closed refusal maps to a status that says "not evaluated",
    /// never to a success and never to a client error blaming the order.
    #[test]
    fn unmeasurable_inputs_map_to_service_unavailable() {
        for r in [
            RiskRejection::MarketDataUnavailable {
                instrument_id: "BTC-USD".into(),
            },
            RiskRejection::InstrumentMetadataUnavailable {
                instrument_id: "BTC-USD".into(),
            },
            RiskRejection::AccountStateUnavailable {
                detail: "x".into(),
            },
            RiskRejection::GateUnavailable {
                detail: "x".into(),
            },
        ] {
            let (status, _) = risk_rejection_response(&r);
            assert_eq!(
                status,
                StatusCode::SERVICE_UNAVAILABLE,
                "{r} must read as 'the gate could not evaluate this'"
            );
        }
    }
}
