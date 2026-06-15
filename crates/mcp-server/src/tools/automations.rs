//! Automation tools: `list_automations`, `create_automation`,
//! `arm_automation`, `disarm_automation`.
//!
//! Uses `storage::automation` functions directly via the `pg` pool.
//! `create_automation` with `account_mode: "live"` is blocked unless
//! `MCP_ALLOW_LIVE_AUTOMATIONS=true`.

use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;

use storage::automation::{
    insert_automation, list_automations, set_automation_armed, AutomationRow,
};

use crate::McpContext;

const DEV_USER: Uuid = Uuid::nil();

fn db_unavailable() -> Value {
    json!({ "error": "service_unavailable", "reason": "database not configured" })
}

/// `list_automations` — list all automation plans.
pub async fn list_automations_tool(ctx: &McpContext) -> Value {
    let Some(pg) = &ctx.pg else {
        tracing::warn!("list_automations: no pg pool configured");
        return json!({ "automations": [] });
    };
    match list_automations(pg).await {
        Ok(rows) => {
            let list: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id.to_string(),
                        "kind": r.kind,
                        "account_mode": r.account_mode,
                        "armed": r.armed,
                        "spec": r.spec,
                        "created_at": r.created_at.to_rfc3339(),
                    })
                })
                .collect();
            json!({ "automations": list })
        }
        Err(e) => {
            tracing::warn!(error = %e, "list_automations: query failed");
            json!({ "automations": [] })
        }
    }
}

/// `create_automation` — create a SingleInstrument automation.
pub async fn create_automation(ctx: &McpContext, params: &Value) -> Value {
    let Some(pg) = &ctx.pg else {
        return db_unavailable();
    };

    // Validate execution_strategy_id exists.
    let sid_str = params
        .get("execution_strategy_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Ok(sid) = Uuid::parse_str(sid_str) else {
        return json!({ "error": "invalid_uuid", "field": "execution_strategy_id" });
    };
    {
        let store = ctx
            .strategy_store
            .lock()
            .expect("strategy_store lock poisoned");
        if !store.contains_key(&sid) {
            return json!({ "error": "strategy_not_found" });
        }
    }

    let instrument_id = params
        .get("instrument_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let asset_class = params
        .get("asset_class")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let account_mode = params
        .get("account_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("paper")
        .to_owned();

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

    let time_window_start = params
        .get("time_window_start")
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned());
    let time_window_end = params
        .get("time_window_end")
        .and_then(|v| v.as_str())
        .map(|s| s.to_owned());
    let time_window_tz = params
        .get("time_window_tz")
        .and_then(|v| v.as_str())
        .unwrap_or("UTC")
        .to_owned();

    let spec = json!({
        "asset_class": asset_class,
        "instrument_id": instrument_id,
        "execution_strategy_id": sid.to_string(),
        "time_window": {
            "start": time_window_start,
            "end": time_window_end,
            "timezone": time_window_tz,
        }
    });

    let row = AutomationRow {
        id: Uuid::new_v4(),
        user_id: DEV_USER,
        kind: "single_instrument".into(),
        account_mode,
        spec,
        armed,
        created_at: Utc::now(),
    };
    let automation_id = row.id;

    match insert_automation(pg, &row).await {
        Ok(()) => json!({
            "automation_id": automation_id.to_string(),
            "armed": armed,
            "kind": "single_instrument",
        }),
        Err(e) => json!({ "error": "service_unavailable", "reason": e.to_string() }),
    }
}

/// `arm_automation` — set a specific automation to armed = true.
pub async fn arm_automation(ctx: &McpContext, params: &Value) -> Value {
    toggle_armed(ctx, params, true).await
}

/// `disarm_automation` — set a specific automation to armed = false.
pub async fn disarm_automation(ctx: &McpContext, params: &Value) -> Value {
    toggle_armed(ctx, params, false).await
}

async fn toggle_armed(ctx: &McpContext, params: &Value, armed: bool) -> Value {
    let Some(pg) = &ctx.pg else {
        return db_unavailable();
    };
    let id_str = params
        .get("automation_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let Ok(id) = Uuid::parse_str(id_str) else {
        return json!({ "error": "invalid_uuid", "automation_id": id_str });
    };
    match set_automation_armed(pg, id, armed).await {
        Ok(true) => json!({ "automation_id": id.to_string(), "armed": armed }),
        Ok(false) => json!({ "error": "not_found" }),
        Err(e) => json!({ "error": "service_unavailable", "reason": e.to_string() }),
    }
}
