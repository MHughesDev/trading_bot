//! Read-only portfolio/observability tools: `get_dashboard_rollup`,
//! `get_paper_activity`, `get_trading_status`, `get_order`.
//!
//! Strictly observational — there is still no order-placement tool anywhere
//! on this server.

use serde_json::{json, Value};

use crate::tools::market::urlencode;
use crate::ApiClient;

/// `get_dashboard_rollup` — account overview (balances, positions, P&L).
pub async fn get_dashboard_rollup(api: &ApiClient) -> Value {
    match api.get("/api/dashboard/rollup").await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `get_paper_activity` — paper-engine positions/fills/PnL for one instrument.
pub async fn get_paper_activity(api: &ApiClient, params: &Value) -> Value {
    let id = params
        .get("instrument_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id.is_empty() {
        return json!({ "error": "missing_field", "field": "instrument_id" });
    }
    match api
        .get(&format!("/api/paper/instrument/{}", urlencode(id)))
        .await
    {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `get_trading_status` — kill-switch / trading-enabled state.
pub async fn get_trading_status(api: &ApiClient) -> Value {
    match api.get("/api/trading/status").await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `get_order` — one order's status by id (read-only).
pub async fn get_order(api: &ApiClient, params: &Value) -> Value {
    let id = params
        .get("order_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id.is_empty() {
        return json!({ "error": "missing_field", "field": "order_id" });
    }
    match api.get(&format!("/api/orders/{}", urlencode(id))).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}
