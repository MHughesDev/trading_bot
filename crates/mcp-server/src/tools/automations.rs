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

/// Declarations for the four `automation` tools (AGENT-006 §2.3).
///
/// None is `internal()`: no standing trading configuration from the in-app
/// agent. They are still declared, because the same catalogue serves the human
/// MCP client, and a tool that is not declared is a tool nothing classifies.
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::{Envelope, ToolSpec};
    use crate::taxonomy::Risk;
    let by_id = |name: &'static str, risk, description| {
        ToolSpec::new(
            name,
            "automation",
            risk,
            description,
            json!({
                "type": "object",
                "required": ["automation_id"],
                "properties": { "automation_id": { "type": "string" } }
            }),
        )
    };
    vec![
        ToolSpec::new(
            "list_automations",
            "automation",
            Risk::Read,
            "List all automation plans (strategy-to-instrument bindings that run when armed).",
            json!({ "type": "object", "properties": {} }),
        ),
        ToolSpec::new(
            "create_automation",
            "automation",
            Risk::Write,
            "Create a SingleInstrument automation tying a stored strategy to an instrument. Live \
             mode is blocked unless the operator sets MCP_ALLOW_LIVE_AUTOMATIONS.",
            json!({
                "type": "object",
                "required": ["execution_strategy_id", "instrument_id", "asset_class", "account_mode"],
                "properties": {
                    "execution_strategy_id": { "type": "string", "description": "strategy_id slug of the stored strategy" },
                    "instrument_id": { "type": "string" },
                    "asset_class": { "type": "string", "description": "e.g. crypto_spot_cex" },
                    "account_mode": { "type": "string", "enum": ["paper", "live"] },
                    "armed": { "type": "boolean", "description": "Start armed (default false)" },
                    "time_window_start": { "type": "string", "description": "HH:MM trading window open (omit for 24/7)" },
                    "time_window_end": { "type": "string", "description": "HH:MM trading window close (omit for 24/7)" },
                    "time_window_tz": { "type": "string", "description": "IANA timezone (default UTC)" }
                }
            }),
        )
        .evidence(&["automation_id"]),
        // Arming changes what the platform will do with money, with no further
        // human step. That is the definition of destructive here, and it is why
        // it is also the one catalogue tool under an approval envelope.
        by_id("arm_automation", Risk::Destructive, "Arm an automation by ID.")
            .envelope(Envelope::Spend),
        by_id("disarm_automation", Risk::Write, "Disarm an automation by ID."),
    ]
}
