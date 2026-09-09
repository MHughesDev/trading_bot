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
