//! Authoring tools: `validate_strategy`, `create_strategy`, `get_strategy`,
//! `list_strategies`.
//!
//! Validation runs locally (the validator is pure and identical to the one the
//! platform uses); persistence goes through the platform API per ADR-0010, so
//! strategies created here are real: visible in the UI, usable by slug in
//! backtests, and durable across restarts.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use domain::strategy_def::StrategyDefinition;
use strategy_validator::validate;

use crate::ApiClient;

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<ValidationErrorItem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidationErrorItem {
    pub path: String,
    pub message: String,
}

/// `validate_strategy` — validate a definition JSON without persisting.
///
/// Returns structured errors the agent can parse and act on.
pub fn validate_strategy(definition_json: &str) -> ValidationResult {
    let def: StrategyDefinition = match serde_json::from_str(definition_json) {
        Ok(d) => d,
        Err(e) => {
            return ValidationResult {
                valid: false,
                errors: vec![ValidationErrorItem {
                    path: "<root>".into(),
                    message: format!("JSON parse error: {e}"),
                }],
            }
        }
    };

    match validate(&def) {
        Ok(_) => ValidationResult {
            valid: true,
            errors: vec![],
        },
        Err(errs) => ValidationResult {
            valid: false,
            errors: errs
                .into_iter()
                .map(|e| ValidationErrorItem {
                    path: e.path,
                    message: e.message,
                })
                .collect(),
        },
    }
}

/// `create_strategy` — validate locally, then persist via the platform API.
pub async fn create_strategy(api: &ApiClient, definition_json: &str) -> Value {
    let vr = validate_strategy(definition_json);
    if !vr.valid {
        return json!({ "error": "validation_failed", "errors": vr.errors });
    }
    let def: StrategyDefinition = serde_json::from_str(definition_json).expect("already validated");
    create_strategy_from_def(api, def).await
}

/// Persist a typed, already-validated `StrategyDefinition` via the API.
///
/// Also used by `finalize_strategy` in the builder flow.
pub async fn create_strategy_from_def(api: &ApiClient, def: StrategyDefinition) -> Value {
    let body = match serde_json::to_value(&def) {
        Ok(v) => v,
        Err(e) => return json!({ "error": "serialization_error", "detail": e.to_string() }),
    };
    match api.post("/api/strategies", body).await {
        Ok(resp) => json!({
            "strategy_id": resp.get("strategy_id").cloned().unwrap_or(json!(def.strategy_id)),
            "created": true,
            "note": "upserted by strategy_id — reusing this slug overwrites the strategy",
        }),
        Err(e) => e.to_tool_error(),
    }
}

/// `get_strategy` — fetch a stored definition by slug.
pub async fn get_strategy(api: &ApiClient, params: &Value) -> Value {
    let slug = params
        .get("strategy_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if slug.is_empty() {
        return json!({ "error": "missing_field", "field": "strategy_id" });
    }
    match api.get(&format!("/api/strategies/{slug}/config")).await {
        Ok(resp) => resp,
        Err(e) => e.to_tool_error(),
    }
}

/// `list_strategies` — list stored strategy slugs.
pub async fn list_strategies(api: &ApiClient) -> Value {
    match api.get("/api/strategies").await {
        Ok(resp) => resp,
        Err(e) => e.to_tool_error(),
    }
}

/// `list_compatible_strategies` — strategies whose data requirements the given
/// instrument/asset-class can satisfy (incompatible ones are omitted).
pub async fn list_compatible_strategies(api: &ApiClient, params: &Value) -> Value {
    let mut query = Vec::new();
    if let Some(i) = params.get("instrument_id").and_then(|v| v.as_str()) {
        query.push(format!("instrument={}", crate::tools::market::urlencode(i)));
    }
    let asset_class = params
        .get("asset_class")
        .and_then(|v| v.as_str())
        .unwrap_or("crypto_spot_cex");
    query.push(format!(
        "asset_class={}",
        crate::tools::market::urlencode(asset_class)
    ));
    match api
        .get(&format!("/api/strategies/apply-list?{}", query.join("&")))
        .await
    {
        Ok(resp) => resp,
        Err(e) => e.to_tool_error(),
    }
}
