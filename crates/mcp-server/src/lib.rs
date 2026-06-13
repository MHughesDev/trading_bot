//! MCP server library — thin front door to the strategy platform.
//!
//! All seven tools route through the shared validator and runtime.
//! No privileged path; no order-placement tool.

pub mod tools;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sqlx::PgPool;
use uuid::Uuid;

use demand_manager::{DemandRegistry, NoopPipelineFactory};
use domain::strategy_def::StrategyDefinition;
use strategy_runtime::InstanceManager;

/// Shared context injected into every MCP tool call.
///
/// Mirrors the same fields as `AppState` in the API crate so all three doors
/// operate on the same logical store.
#[derive(Clone)]
pub struct McpContext {
    pub strategy_store: Arc<Mutex<HashMap<Uuid, StrategyDefinition>>>,
    pub instance_manager: Arc<Mutex<InstanceManager>>,
    /// Postgres connection pool — `None` when `DATABASE_URL` is not set.
    pub pg: Option<PgPool>,
}

impl McpContext {
    pub fn new_without_db() -> Self {
        let demand = Arc::new(DemandRegistry::new(Arc::new(NoopPipelineFactory)));
        Self {
            strategy_store: Arc::new(Mutex::new(HashMap::new())),
            instance_manager: Arc::new(Mutex::new(InstanceManager::new(demand))),
            pg: None,
        }
    }

    pub async fn new() -> Self {
        let demand = Arc::new(DemandRegistry::new(Arc::new(NoopPipelineFactory)));
        let pg = match std::env::var("DATABASE_URL") {
            Ok(url) => match PgPool::connect(&url).await {
                Ok(pool) => Some(pool),
                Err(e) => {
                    tracing::warn!(error = %e, "MCP: DATABASE_URL set but connection failed; discovery will return empty instrument list");
                    None
                }
            },
            Err(_) => {
                tracing::info!("MCP: DATABASE_URL not set; discovery will return empty instrument list");
                None
            }
        };
        Self {
            strategy_store: Arc::new(Mutex::new(HashMap::new())),
            instance_manager: Arc::new(Mutex::new(InstanceManager::new(demand))),
            pg,
        }
    }
}

/// Dispatch an MCP JSON-RPC tool call and return the result as a JSON Value.
///
/// This is the single entry point used by the transport layer.
pub async fn dispatch_tool(
    ctx: &McpContext,
    tool_name: &str,
    params: &serde_json::Value,
) -> serde_json::Value {
    use serde_json::json;

    match tool_name {
        "list_lanes" => {
            let lanes = tools::discovery::list_lanes();
            json!({ "lanes": lanes })
        }
        "list_instruments" => {
            let asset_class = params.get("asset_class").and_then(|v| v.as_str());
            let instruments = match &ctx.pg {
                Some(pg) => tools::discovery::list_instruments(pg, asset_class).await,
                None => vec![],
            };
            json!({ "instruments": instruments })
        }
        "validate_strategy" => {
            let definition_json = params
                .get("definition_json")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let result = tools::authoring::validate_strategy(definition_json);
            serde_json::to_value(result).unwrap_or_else(|_| json!({"error": "serialization_error"}))
        }
        "create_strategy" => {
            let definition_json = params
                .get("definition_json")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match tools::authoring::create_strategy(ctx, definition_json) {
                Ok(r) => serde_json::to_value(r)
                    .unwrap_or_else(|_| json!({"error": "serialization_error"})),
                Err(e) => serde_json::to_value(e)
                    .unwrap_or_else(|_| json!({"error": "serialization_error"})),
            }
        }
        "apply_strategy" => {
            let store_id = params
                .get("store_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let user_id = params.get("user_id").and_then(|v| v.as_str()).unwrap_or("");
            let instrument_id = params
                .get("instrument_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match tools::lifecycle::apply_strategy(ctx, store_id, user_id, instrument_id) {
                Ok(r) => serde_json::to_value(r)
                    .unwrap_or_else(|_| json!({"error": "serialization_error"})),
                Err(e) => serde_json::to_value(e)
                    .unwrap_or_else(|_| json!({"error": "serialization_error"})),
            }
        }
        "stop_strategy" => {
            let user_id = params.get("user_id").and_then(|v| v.as_str()).unwrap_or("");
            let instrument_id = params
                .get("instrument_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let result = tools::lifecycle::stop_strategy(ctx, user_id, instrument_id);
            serde_json::to_value(result).unwrap_or_else(|_| json!({"error": "serialization_error"}))
        }
        "list_strategies" => {
            let list = tools::lifecycle::list_strategies(ctx);
            json!({ "strategies": list })
        }
        unknown => {
            json!({ "error": "unknown_tool", "tool": unknown })
        }
    }
}

/// The complete list of tools exposed by this MCP server.
pub fn tool_definitions() -> serde_json::Value {
    use serde_json::json;
    json!([
        {
            "name": "list_lanes",
            "description": "Return available data lanes and which instruments publish them",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "list_instruments",
            "description": "Return instruments with asset class and metadata",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "asset_class": { "type": "string", "description": "Filter by asset class (optional)" }
                }
            }
        },
        {
            "name": "validate_strategy",
            "description": "Validate a strategy definition JSON without persisting it; returns structured errors the agent can act on",
            "inputSchema": {
                "type": "object",
                "required": ["definition_json"],
                "properties": {
                    "definition_json": { "type": "string", "description": "Strategy definition as a JSON string" }
                }
            }
        },
        {
            "name": "create_strategy",
            "description": "Persist a validated strategy definition to the strategy library",
            "inputSchema": {
                "type": "object",
                "required": ["definition_json"],
                "properties": {
                    "definition_json": { "type": "string", "description": "Strategy definition as a JSON string" }
                }
            }
        },
        {
            "name": "apply_strategy",
            "description": "Start a strategy instance on a specific instrument",
            "inputSchema": {
                "type": "object",
                "required": ["store_id", "user_id", "instrument_id"],
                "properties": {
                    "store_id": { "type": "string", "description": "UUID from create_strategy" },
                    "user_id": { "type": "string" },
                    "instrument_id": { "type": "string" }
                }
            }
        },
        {
            "name": "stop_strategy",
            "description": "Stop a running strategy instance",
            "inputSchema": {
                "type": "object",
                "required": ["user_id", "instrument_id"],
                "properties": {
                    "user_id": { "type": "string" },
                    "instrument_id": { "type": "string" }
                }
            }
        },
        {
            "name": "list_strategies",
            "description": "List all defined strategies",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}
