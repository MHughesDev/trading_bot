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
        return crate::tool_error::missing_field("instrument_id");
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
        return crate::tool_error::missing_field("order_id");
    }
    match api.get(&format!("/api/orders/{}", urlencode(id))).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// Declarations for the four read-only `portfolio` tools (AGENT-006 §2.3).
///
/// Every one is `Risk::Read`. There is no order-placement tool anywhere in this
/// catalogue, and `place_order` is gated in `ledger::audit` as a platform action
/// precisely so that stays true by decision rather than by omission.
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::ToolSpec;
    use crate::taxonomy::Risk;
    vec![
        ToolSpec::new(
            "get_dashboard_rollup",
            "portfolio",
            Risk::Read,
            "Account overview: balances, open positions and P&L per asset class.",
            json!({ "type": "object", "properties": {} }),
        )
        .internal(),
        ToolSpec::new(
            "get_paper_activity",
            "portfolio",
            Risk::Read,
            "Paper-trading engine activity for one instrument: positions, fills, P&L.",
            json!({
                "type": "object",
                "required": ["instrument_id"],
                "properties": { "instrument_id": { "type": "string" } }
            }),
        )
        .internal(),
        ToolSpec::new(
            "get_trading_status",
            "portfolio",
            Risk::Read,
            "Platform trading status: whether the kill switch is tripped.",
            json!({ "type": "object", "properties": {} }),
        )
        .internal(),
        ToolSpec::new(
            "get_order",
            "portfolio",
            Risk::Read,
            "Look up one order's status by id.",
            json!({
                "type": "object",
                "required": ["order_id"],
                "properties": {
                    "order_id": { "type": "string", "description": "Order UUID" }
                }
            }),
        )
        .internal()
        .evidence(&["order_id"]),
    ]
}
