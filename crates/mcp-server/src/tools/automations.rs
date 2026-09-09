//! Automation tools: `list_automations`, `create_automation`,
//! `arm_automation`, `disarm_automation`.
//!
//! All calls go through the platform API (`/api/automations`) so plans carry
//! the caller's real user identity. `create_automation` with
//! `account_mode: "live"` is blocked unless `MCP_ALLOW_LIVE_AUTOMATIONS=true`.

use serde_json::{json, Value};

use crate::ApiClient;

/// `list_automations` — list all automation plans.
pub async fn list_automations_tool(api: &ApiClient) -> Value {
    match api.get("/api/automations").await {
        Ok(resp) => resp,
        Err(e) => e.to_tool_error(),
    }
}

/// `create_automation` — create a SingleInstrument automation.
pub async fn create_automation(api: &ApiClient, params: &Value) -> Value {
    let strategy_id = params
        .get("execution_strategy_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if strategy_id.is_empty() {
        return json!({ "error": "missing_field", "field": "execution_strategy_id" });
    }
    let instrument_id = params
        .get("instrument_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let asset_class = params
        .get("asset_class")
        .and_then(|v| v.as_str())
        .unwrap_or("crypto_spot_cex");
    let account_mode = params
        .get("account_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("paper");

    if account_mode == "live" && !crate::mcp_live_automations_allowed() {
        return json!({
            "error": "live_automations_disabled",
            "hint": "Set MCP_ALLOW_LIVE_AUTOMATIONS=true to enable live automations"
        });
    }

    let armed = params
        .get("armed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let body = json!({
        "kind": "single_instrument",
        "account_mode": account_mode,
        "armed": armed,
        "spec": {
            "asset_class": asset_class,
            "instrument_id": instrument_id,
            "execution_strategy_id": strategy_id,
            "time_window": {
                "start": params.get("time_window_start").and_then(|v| v.as_str()),
                "end": params.get("time_window_end").and_then(|v| v.as_str()),
                "timezone": params
                    .get("time_window_tz")
                    .and_then(|v| v.as_str())
                    .unwrap_or("UTC"),
            }
        }
    });

    match api.post("/api/automations", body).await {
        Ok(resp) => resp,
        Err(e) => e.to_tool_error(),
    }
}

/// `arm_automation` — set a specific automation to armed = true.
pub async fn arm_automation(api: &ApiClient, params: &Value) -> Value {
    toggle_armed(api, params, true).await
}

/// `disarm_automation` — set a specific automation to armed = false.
pub async fn disarm_automation(api: &ApiClient, params: &Value) -> Value {
    toggle_armed(api, params, false).await
}

async fn toggle_armed(api: &ApiClient, params: &Value, armed: bool) -> Value {
    let id = params
        .get("automation_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id.is_empty() {
        return json!({ "error": "missing_field", "field": "automation_id" });
    }
    let action = if armed { "arm" } else { "disarm" };
    match api
        .post(&format!("/api/automations/{id}/{action}"), json!({}))
        .await
    {
        Ok(resp) => resp,
        Err(e) => e.to_tool_error(),
    }
}
