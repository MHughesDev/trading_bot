//! Backtest tools: `list_backtests`, `get_backtest`, `create_backtest`.
//!
//! Uses `BacktestManager` exclusively (ADR-0014). If the manager is not
//! configured (missing CLICKHOUSE_URL or DATABASE_URL), tools return a clear
//! `service_unavailable` error rather than panicking.

use serde_json::{json, Value};
use uuid::Uuid;

use backtest::types::{ResolvedSpec, TimeframeExt};
use domain::payloads::bar::Timeframe;

use crate::McpContext;

const DEV_USER: Uuid = Uuid::nil();

const VALID_TIMEFRAMES: &[&str] = &["1s", "1m", "5m", "15m", "1h", "4h", "1d"];

fn service_unavailable() -> Value {
    json!({
        "error": "service_unavailable",
        "reason": "backtest service not configured"
    })
}

/// `list_backtests` — list all backtest runs for the current user.
pub async fn list_backtests(ctx: &McpContext) -> Value {
    let Some(mgr) = &ctx.backtest_manager else {
        return json!({ "backtests": [] });
    };
    let snapshots = mgr.list(DEV_USER).await;
    let list: Vec<Value> = snapshots
        .iter()
        .map(|s| {
            json!({
                "id": s.id.to_string(),
                "name": s.name,
                "status": format!("{:?}", s.status),
                "progress": s.progress,
                "instrument_id": s.instrument_id,
                "timeframe": s.timeframe,
                "start": s.start.to_rfc3339(),
                "end": s.end.to_rfc3339(),
                "created_at": s.created_at.to_rfc3339(),
                "finished_at": s.finished_at.map(|t| t.to_rfc3339()),
            })
        })
        .collect();
    json!({ "backtests": list })
}

/// `get_backtest` — get full snapshot for one backtest run.
pub async fn get_backtest(ctx: &McpContext, params: &Value) -> Value {
    let Some(mgr) = &ctx.backtest_manager else {
        return service_unavailable();
    };
    let id_str = params.get("backtest_id").and_then(|v| v.as_str()).unwrap_or("");
    let Ok(id) = Uuid::parse_str(id_str) else {
        return json!({ "error": "invalid_uuid", "backtest_id": id_str });
    };
    match mgr.get(DEV_USER, id).await {
        Some(s) => serde_json::to_value(&s).unwrap_or_else(|_| json!({"error": "serialization_error"})),
        None => json!({ "error": "not_found" }),
    }
}

/// `create_backtest` — trigger a new backtest run.
pub async fn create_backtest(ctx: &McpContext, params: &Value) -> Value {
    let Some(mgr) = &ctx.backtest_manager else {
        return service_unavailable();
    };

    // Resolve strategy from store.
    let store_id_str = params.get("store_id").and_then(|v| v.as_str()).unwrap_or("");
    let Ok(store_id) = Uuid::parse_str(store_id_str) else {
        return json!({ "error": "invalid_uuid", "field": "store_id" });
    };
    let def = {
        let store = ctx.strategy_store.lock().expect("strategy_store lock poisoned");
        store.get(&store_id).cloned()
    };
    let Some(definition) = def else {
        return json!({ "error": "strategy_not_found" });
    };

    let instrument_id = params.get("instrument_id").and_then(|v| v.as_str()).unwrap_or("");
    if instrument_id.is_empty() {
        return json!({ "error": "invalid_instrument_id" });
    }

    let asset_class = params.get("asset_class").and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let timeframe_key = params.get("timeframe").and_then(|v| v.as_str()).unwrap_or("");
    let Some(timeframe) = <Timeframe as TimeframeExt>::from_key(timeframe_key) else {
        return json!({
            "error": "invalid_timeframe",
            "valid_values": VALID_TIMEFRAMES,
        });
    };

    let start_str = params.get("start").and_then(|v| v.as_str()).unwrap_or("");
    let end_str = params.get("end").and_then(|v| v.as_str()).unwrap_or("");

    let start = match chrono::DateTime::parse_from_rfc3339(start_str) {
        Ok(dt) => dt.with_timezone(&chrono::Utc),
        Err(_) => return json!({ "error": "invalid_start_date" }),
    };
    let end = match chrono::DateTime::parse_from_rfc3339(end_str) {
        Ok(dt) => dt.with_timezone(&chrono::Utc),
        Err(_) => return json!({ "error": "invalid_end_date" }),
    };
    if end <= start {
        return json!({ "error": "invalid_date_range" });
    }

    let strategy_id = definition.strategy_id.clone();
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned())
        .unwrap_or_else(|| {
            format!("{strategy_id} · {instrument_id} · {timeframe_key}")
        });

    let initial_balance = params
        .get("initial_balance")
        .and_then(|v| v.as_str())
        .unwrap_or("100000")
        .to_owned();
    let quote_currency = params
        .get("quote_currency")
        .and_then(|v| v.as_str())
        .unwrap_or("USD")
        .to_owned();
    let auto_collect = params
        .get("auto_collect")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let venue_id = params
        .get("venue_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_owned();

    let spec = ResolvedSpec {
        name,
        definition,
        instrument_id: instrument_id.to_owned(),
        venue_id,
        asset_class,
        timeframe,
        start,
        end,
        initial_balance,
        quote_currency,
        auto_collect,
    };

    match mgr.create(DEV_USER, spec).await {
        Ok(backtest_id) => json!({
            "backtest_id": backtest_id.to_string(),
            "status": "Queued",
        }),
        Err(e) => json!({ "error": "create_failed", "reason": e.to_string() }),
    }
}
