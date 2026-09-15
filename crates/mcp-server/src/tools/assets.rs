//! Asset-onboarding tools: `list_initialized_assets`, `init_asset`,
//! `get_asset_init_job`.
//!
//! `init_asset` starts the platform's background bar-seeding job for a symbol
//! (historical backfill + live 1-minute aggregation). It collects data only —
//! it never trades — and is how an agent brings a new crypto pair into
//! backtesting scope.

use serde_json::{json, Value};

use crate::tools::market::urlencode;
use crate::ApiClient;

/// `list_initialized_assets` — symbols with live pipelines + seeded history.
pub async fn list_initialized_assets(api: &ApiClient) -> Value {
    match api.get("/assets/initialized").await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `init_asset` — start (or re-run) bar seeding for a symbol.
pub async fn init_asset(api: &ApiClient, params: &Value) -> Value {
    let symbol = params.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
    if symbol.is_empty() {
        return json!({ "error": "missing_field", "field": "symbol" });
    }
    let lookback_days = params
        .get("lookback_days")
        .and_then(|v| v.as_i64())
        .unwrap_or(90)
        .clamp(1, 3650);
    let mut body = json!({ "lookback_days": lookback_days });
    if let Some(ac) = params.get("asset_class").and_then(|v| v.as_str()) {
        body["asset_class"] = json!(ac);
    }
    match api
        .post(&format!("/assets/init/{}", urlencode(symbol)), body)
        .await
    {
        Ok(v) => {
            let mut out = v;
            if let Some(obj) = out.as_object_mut() {
                obj.insert(
                    "hint".into(),
                    json!("seeding runs in the background — poll get_asset_init_job"),
                );
            }
            out
        }
        Err(e) => e.to_tool_error(),
    }
}

/// `get_asset_init_job` — status of one seeding job.
pub async fn get_asset_init_job(api: &ApiClient, params: &Value) -> Value {
    let job_id = params.get("job_id").and_then(|v| v.as_str()).unwrap_or("");
    if job_id.is_empty() {
        return json!({ "error": "missing_field", "field": "job_id" });
    }
    match api
        .get(&format!("/assets/init/jobs/{}", urlencode(job_id)))
        .await
    {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// Declarations for the three tools this module serves (AGENT-006 §2.3).
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::ToolSpec;
    use crate::taxonomy::Risk;
    vec![
        ToolSpec::new(
            "list_initialized_assets",
            "discovery",
            Risk::Read,
            "List symbols that have been initialized on the platform (seeded history plus live \
             1-minute aggregation).",
            json!({ "type": "object", "properties": {} }),
        )
        .internal(),
        // Onboarding starts live collection and a backfill: it spends real
        // resources and changes what the platform is doing, so it is a write.
        ToolSpec::new(
            "init_asset",
            "data",
            Risk::Write,
            "Onboard a symbol: starts a background job that backfills historical bars and starts \
             live 1-minute aggregation. Returns a job_id; poll get_asset_init_job until the \
             status is terminal.",
            json!({
                "type": "object",
                "required": ["symbol"],
                "properties": {
                    "symbol": { "type": "string", "description": "e.g. SOL-USD" },
                    "lookback_days": { "type": "integer", "description": "Days of history to backfill (default 90, max 3650)" },
                    "asset_class": { "type": "string", "description": "Default resolved from the registry, else crypto heuristic" }
                }
            }),
        )
        .internal()
        .evidence(&["job_id"])
        .max_calls_per_run(5)
        .notes("init_asset"),
        ToolSpec::new(
            "get_asset_init_job",
            "data",
            Risk::Read,
            "Status of an init_asset seeding job (status, bars_collected, error).",
            json!({
                "type": "object",
                "required": ["job_id"],
                "properties": {
                    "job_id": { "type": "string", "description": "UUID from init_asset" }
                }
            }),
        )
        .internal(),
    ]
}
