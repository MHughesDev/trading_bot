//! `GET /api/backtest/runs/{run_id}/trades/{idx}/explain` (Set M M-2.7; FEAT-005 §8).
//!
//! Answers "why did this trade happen?" from the rich outputs (`backtest_run_trade_detail`,
//! `backtest_run_fills`, `backtest_run_decisions`; `clickhouse/11_backtest_outputs_v2.sql`): the
//! slot values and signals on the decision bar that led to the entry fill, the regime and path,
//! the excursions, the exit reason and the cost legs.
//!
//! # Ownership
//!
//! The outputs tables are keyed by `run_id`, a content hash that two tenants running the same
//! config share, and they carry no tenant column. So the route first asks the ledger, under the
//! session tenant's row-level security, whether this tenant has a trial for that run; a run it
//! has not looked at is `404`, the same as one that does not exist.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use clickhouse::Row;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{auth::BearerToken, state::AppState};

#[derive(Row, Deserialize)]
struct DetailRow {
    trade_idx: u32,
    symbol: String,
    side: String,
    entry_ts_ns: i64,
    exit_ts_ns: i64,
    mae: f64,
    mfe: f64,
    exit_reason: String,
    entry_order_type: String,
    fee_str: String,
    spread_cost_str: String,
    impact_cost_str: String,
    borrow_cost_str: String,
}

#[derive(Row, Deserialize)]
struct FillRow {
    decided_ns: i64,
    filled_ns: i64,
    side: String,
    qty_str: String,
    price_str: String,
}

#[derive(Row, Deserialize)]
struct DecisionRow {
    ts_ns: i64,
    slots: String,
    signals: String,
    intents: String,
    regime: String,
    path: String,
}

fn err(status: StatusCode, code: &str, message: String, extra: Value) -> axum::response::Response {
    (status, Json(json!({ "error": code, "message": message, "detail": extra }))).into_response()
}

fn parse(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.to_string()))
}

async fn owns_run(state: &AppState, tenant: &str, run_id: &str) -> Result<bool, sqlx::Error> {
    let mut tx = ledger::pg::tenant_tx(&state.pg, tenant).await?;
    let found: Option<i32> = sqlx::query_scalar("SELECT 1 FROM mlops.trial WHERE tenant_id = $1 AND config_hash = $2 LIMIT 1")
        .bind(tenant)
        .bind(run_id)
        .fetch_optional(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(found.is_some())
}

pub async fn explain_trade(
    State(state): State<AppState>,
    token: BearerToken,
    Path((run_id, idx)): Path<(String, u32)>,
) -> impl IntoResponse {
    let tenant = token.user_id().to_string();
    match owns_run(&state, &tenant, &run_id).await {
        Ok(true) => {}
        Ok(false) => return err(StatusCode::NOT_FOUND, "run_not_found", format!("no run {run_id} for this tenant"), json!({})),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string(), json!({})),
    }
    let ch = storage::clickhouse::connect(&state.clickhouse_url);
    let fail = |e: clickhouse::error::Error| err(StatusCode::INTERNAL_SERVER_ERROR, "outputs_unavailable", e.to_string(), json!({}));

    let count: u64 = match ch
        .query("SELECT count() FROM backtest_run_trade_detail FINAL WHERE run_id = ?")
        .bind(&run_id)
        .fetch_one()
        .await
    {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    if count == 0 {
        return err(
            StatusCode::NOT_FOUND,
            "outputs_not_written",
            format!("run {run_id} has no rich outputs: it predates them, or its writer failed"),
            json!({ "run_id": run_id }),
        );
    }
    if u64::from(idx) >= count {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "trade_index_out_of_range",
            format!("trade {idx} is out of range: the run has {count} trades (0..{})", count - 1),
            json!({ "trades": count }),
        );
    }
    let detail: DetailRow = match ch
        .query(
            "SELECT trade_idx, symbol, side, entry_ts_ns, exit_ts_ns, mae, mfe, exit_reason, \
             entry_order_type, fee_str, spread_cost_str, impact_cost_str, borrow_cost_str \
             FROM backtest_run_trade_detail FINAL WHERE run_id = ? AND trade_idx = ?",
        )
        .bind(&run_id)
        .bind(idx)
        .fetch_one()
        .await
    {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    let fill: Option<FillRow> = match ch
        .query(
            "SELECT decided_ns, filled_ns, side, qty_str, price_str FROM backtest_run_fills FINAL \
             WHERE run_id = ? AND filled_ns = ? ORDER BY seq LIMIT 1",
        )
        .bind(&run_id)
        .bind(detail.entry_ts_ns)
        .fetch_optional()
        .await
    {
        Ok(f) => f,
        Err(e) => return fail(e),
    };
    let decision: Option<DecisionRow> = match &fill {
        Some(f) => match ch
            .query(
                "SELECT ts_ns, slots, signals, intents, regime, path FROM backtest_run_decisions FINAL \
                 WHERE run_id = ? AND ts_ns = ?",
            )
            .bind(&run_id)
            .bind(f.decided_ns)
            .fetch_optional()
            .await
        {
            Ok(d) => d,
            Err(e) => return fail(e),
        },
        None => None,
    };
    Json(json!({
        "run_id": run_id,
        "trade_idx": detail.trade_idx,
        "trades": count,
        "symbol": detail.symbol,
        "side": detail.side,
        "entry_ts_ns": detail.entry_ts_ns,
        "exit_ts_ns": detail.exit_ts_ns,
        "mae": detail.mae,
        "mfe": detail.mfe,
        "exit_reason": detail.exit_reason,
        "entry_order_type": detail.entry_order_type,
        "cost_legs": {
            "fee": detail.fee_str, "spread": detail.spread_cost_str,
            "impact": detail.impact_cost_str, "borrow": detail.borrow_cost_str,
        },
        "entry_fill": fill.as_ref().map(|f| json!({"decided_ns": f.decided_ns, "filled_ns": f.filled_ns, "side": f.side, "qty": f.qty_str, "price": f.price_str})),
        "entry_decision": decision.as_ref().map(|d| json!({
            "ts_ns": d.ts_ns, "slots": parse(&d.slots), "signals": parse(&d.signals),
            "intents": parse(&d.intents), "regime": d.regime, "path": d.path,
        })),
    }))
    .into_response()
}
