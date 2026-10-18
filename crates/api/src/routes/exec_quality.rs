//! Execution quality, per strategy and instrument (FB-RX-003).
//!
//! `GET /api/execution/quality?days=30` — what the order journal says about how
//! orders actually executed: how many fills, what fraction of the ordered
//! quantity filled, and how long a first fill took.
//!
//! # What this will not say
//!
//! **Slippage against the fill model is not served, and says so.** The monitor's
//! `slippage_bps_p50` / `slippage_bps_p95` (`crates/monitor/src/collect.rs`) are
//! computed from `OrderObservation::slippage_vs_model_bps`, and
//! `MonitorStore::order_observations` returns `None` for every row because
//! nothing in `orders` or `fills` records what the fill model *predicted*. So
//! there is no producer for the statistic. It is returned as `unmeasured` with
//! that reason — never `0`, which a reader takes as "the paper engine fills at
//! the touch" — until a producer exists. Fabricating a number from the fill
//! price against some other reference would be a different quantity wearing the
//! same name.
//!
//! Every statistic is a [`Measured`] value: either `{status: "measured", value}`
//! or `{status: "unmeasured", reason}`. A strategy with orders and no fills has
//! a fill ratio of `0` (measured: nothing filled) and an *unmeasured* latency
//! (there was no fill to time), and the two must not be confused.
//!
//! Venue health is not duplicated here: the page joins `/api/monitor/venues`
//! (the monitor's own per-venue reconciliation verdict), which is a fact the
//! monitor already owns.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::BearerToken, state::AppState};

/// The reason the slippage statistics are unmeasured. One place, so the response
/// and the test agree on the words.
pub const NO_FILL_MODEL_PREDICTION: &str =
    "nothing records what the fill model predicted for an order, so realised-versus-model \
     slippage has no producer (monitor::store::order_observations returns None for it)";

#[derive(Debug, Deserialize)]
pub struct QualityQuery {
    /// Window length in days; clamped to `1..=365`.
    pub days: Option<i32>,
}

/// One aggregated row: strategy, instrument, orders, fills, ordered qty,
/// filled qty, first-fill latency p50 and p95 in milliseconds.
type Row = (
    String,
    String,
    i64,
    i64,
    f64,
    f64,
    Option<f64>,
    Option<f64>,
);

fn measured(value: f64) -> Value {
    json!({ "status": "measured", "value": value })
}

fn unmeasured(reason: &str) -> Value {
    json!({ "status": "unmeasured", "reason": reason })
}

/// The wire shape of one row. Pure, so the measured/unmeasured rules are tested
/// without a database.
#[must_use]
pub fn quality_row_json(row: &Row) -> Value {
    let (strategy_id, instrument_id, orders, fills, ordered, filled, p50, p95) = row;
    let fill_ratio = if *ordered > 0.0 {
        measured(filled / ordered)
    } else {
        unmeasured("the orders carry no quantity to divide by")
    };
    let latency = |v: &Option<f64>| match (v, *fills) {
        (Some(ms), n) if n > 0 => measured(*ms),
        _ => unmeasured("no fill was recorded, so there is no submit-to-fill time"),
    };
    json!({
        "strategy_id": strategy_id,
        "instrument_id": instrument_id,
        "orders": orders,
        "fill_count": fills,
        "fill_ratio": fill_ratio,
        "slippage_bps_p50": unmeasured(NO_FILL_MODEL_PREDICTION),
        "slippage_bps_p95": unmeasured(NO_FILL_MODEL_PREDICTION),
        "submit_to_fill_ms_p50": latency(p50),
        "submit_to_fill_ms_p95": latency(p95),
    })
}

/// `GET /api/execution/quality`
pub async fn get_quality(
    _token: BearerToken,
    State(state): State<AppState>,
    Query(q): Query<QualityQuery>,
) -> Result<Json<Value>, Response> {
    let days = q.days.unwrap_or(30).clamp(1, 365);
    // One row per order first (its fills summed, its first fill timed), then one
    // row per strategy and instrument. Joining fills straight onto orders and
    // then summing `size` would count an order's quantity once per fill.
    let rows = sqlx::query_as::<_, Row>(
        "WITH per_order AS (
            SELECT o.idempotency_key,
                   COALESCE(o.strategy_id, 'manual') AS strategy_id,
                   o.instrument_id,
                   o.size::float8                    AS size,
                   o.created_at,
                   COALESCE(SUM(f.filled_size), 0)::float8 AS filled,
                   COUNT(f.fill_id)                  AS fills,
                   MIN(f.filled_at)                  AS first_fill
              FROM orders o
              LEFT JOIN fills f ON f.idempotency_key = o.idempotency_key
             WHERE o.created_at >= now() - make_interval(days => $1)
             GROUP BY o.idempotency_key, o.strategy_id, o.instrument_id, o.size, o.created_at
         )
         SELECT strategy_id,
                instrument_id,
                COUNT(*)::bigint,
                COALESCE(SUM(fills), 0)::bigint,
                COALESCE(SUM(size), 0)::float8,
                COALESCE(SUM(filled), 0)::float8,
                (percentile_cont(0.5) WITHIN GROUP (ORDER BY
                    GREATEST(0::float8, (EXTRACT(EPOCH FROM (first_fill - created_at)) * 1000)::float8))
                    FILTER (WHERE first_fill IS NOT NULL))::float8,
                (percentile_cont(0.95) WITHIN GROUP (ORDER BY
                    GREATEST(0::float8, (EXTRACT(EPOCH FROM (first_fill - created_at)) * 1000)::float8))
                    FILTER (WHERE first_fill IS NOT NULL))::float8
           FROM per_order
          GROUP BY strategy_id, instrument_id
          ORDER BY strategy_id, instrument_id",
    )
    .bind(days)
    .fetch_all(&state.pg)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "execution quality query failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "execution_quality_unavailable", "detail": e.to_string() })),
        )
            .into_response()
    })?;

    Ok(Json(json!({
        "window_days": days,
        "rows": rows.iter().map(quality_row_json).collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(orders: i64, fills: i64, ordered: f64, filled: f64, p50: Option<f64>) -> Row {
        ("s".into(), "BTC-USD".into(), orders, fills, ordered, filled, p50, p50)
    }

    #[test]
    fn slippage_is_unmeasured_with_the_reason_and_never_zero() {
        let j = quality_row_json(&row(3, 3, 3.0, 3.0, Some(40.0)));
        assert_eq!(j["slippage_bps_p50"]["status"], "unmeasured");
        assert_eq!(j["slippage_bps_p95"]["status"], "unmeasured");
        assert_eq!(j["slippage_bps_p50"]["reason"], NO_FILL_MODEL_PREDICTION);
        assert!(j["slippage_bps_p50"].get("value").is_none());
    }

    #[test]
    fn orders_with_no_fills_have_a_measured_zero_ratio_and_an_unmeasured_latency() {
        let j = quality_row_json(&row(2, 0, 2.0, 0.0, None));
        assert_eq!(j["fill_ratio"]["status"], "measured");
        assert_eq!(j["fill_ratio"]["value"], 0.0);
        assert_eq!(j["submit_to_fill_ms_p50"]["status"], "unmeasured");
    }

    #[test]
    fn a_partial_fill_is_the_filled_over_the_ordered_quantity() {
        let j = quality_row_json(&row(1, 1, 4.0, 1.0, Some(12.0)));
        assert_eq!(j["fill_ratio"]["value"], 0.25);
        assert_eq!(j["submit_to_fill_ms_p50"]["value"], 12.0);
        assert_eq!(j["fill_count"], 1);
    }

    #[test]
    fn a_latency_without_a_fill_is_not_reported() {
        // The percentile would be NULL in SQL; if a producer ever sends a number
        // alongside zero fills, the fill count wins.
        let j = quality_row_json(&row(1, 0, 1.0, 0.0, Some(5.0)));
        assert_eq!(j["submit_to_fill_ms_p95"]["status"], "unmeasured");
    }

    #[test]
    fn the_query_sums_fills_per_order_before_summing_orders() {
        let src = include_str!("exec_quality.rs");
        let from = src.find("WITH per_order AS").unwrap();
        let q = &src[from..];
        assert!(
            q.find("GROUP BY o.idempotency_key").unwrap() < q.find("GROUP BY strategy_id").unwrap(),
            "orders must be collapsed to one row each before the strategy aggregate, or an order's \
             size is counted once per fill"
        );
    }
}
