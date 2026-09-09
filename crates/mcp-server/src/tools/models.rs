//! Model-registry tools (read-only): `list_models`, `get_model`.
//!
//! Strategies (definition v1.1) can reference registered AI models via
//! Inference nodes; these tools let an agent discover what exists. Training
//! and deployment stay UI-only for now.

use serde_json::{json, Value};

use crate::tools::market::urlencode;
use crate::ApiClient;

/// `list_models` — registered AI models with kind/status.
pub async fn list_models(api: &ApiClient) -> Value {
    match api.get("/api/models").await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `get_model` — one model's detail by id.
pub async fn get_model(api: &ApiClient, params: &Value) -> Value {
    let id = params
        .get("model_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id.is_empty() {
        return json!({ "error": "missing_field", "field": "model_id" });
    }
    match api.get(&format!("/api/models/{}", urlencode(id))).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}
