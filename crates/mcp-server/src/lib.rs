//! MCP server library — thin front door to the strategy platform.
//!
//! All nine tools route through the shared validator and runtime.
//! No privileged path; no order-placement tool.

pub mod tools;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use uuid::Uuid;

use domain::strategy_def::StrategyDefinition;
use market_simulator_adapter::BacktestReport;
use strategy_runtime::InstanceManager;
use demand_manager::{DemandRegistry, NoopPipelineFactory};

/// Shared context injected into every MCP tool call.
///
/// Mirrors the same fields as `AppState` in the API crate so all three doors
/// operate on the same logical store.
#[derive(Clone)]
pub struct McpContext {
    pub strategy_store: Arc<Mutex<HashMap<Uuid, StrategyDefinition>>>,
    pub instance_manager: Arc<Mutex<InstanceManager>>,
    pub backtest_results: Arc<Mutex<HashMap<Uuid, Result<BacktestReport, String>>>>,
}

impl McpContext {
    pub fn new() -> Self {
        let demand = Arc::new(DemandRegistry::new(Arc::new(NoopPipelineFactory)));
        Self {
            strategy_store: Arc::new(Mutex::new(HashMap::new())),
            instance_manager: Arc::new(Mutex::new(InstanceManager::new(demand))),
            backtest_results: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for McpContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Dispatch an MCP JSON-RPC tool call and return the result as a JSON Value.
///
/// This is the single entry point used by the transport layer.
pub fn dispatch_tool(
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
            let instruments = tools::discovery::list_instruments(asset_class);
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
            let store_id = params.get("store_id").and_then(|v| v.as_str()).unwrap_or("");
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
            serde_json::to_value(result)
                .unwrap_or_else(|_| json!({"error": "serialization_error"}))
        }
        "list_strategies" => {
            let list = tools::lifecycle::list_strategies(ctx);
            json!({ "strategies": list })
        }
        "run_backtest" => {
            let store_id = params.get("store_id").and_then(|v| v.as_str()).unwrap_or("");
            let instrument_id = params
                .get("instrument_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let result = tools::backtest::run_backtest(ctx, store_id, instrument_id);
            serde_json::to_value(result)
                .unwrap_or_else(|_| json!({"error": "serialization_error"}))
        }
        "get_backtest_result" => {
            let backtest_id = params
                .get("backtest_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let result = tools::backtest::get_backtest_result(ctx, backtest_id);
            serde_json::to_value(result)
                .unwrap_or_else(|_| json!({"error": "serialization_error"}))
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
        },
        {
            "name": "run_backtest",
            "description": "Submit a strategy for backtesting on a given instrument",
            "inputSchema": {
                "type": "object",
                "required": ["store_id", "instrument_id"],
                "properties": {
                    "store_id": { "type": "string" },
                    "instrument_id": { "type": "string" }
                }
            }
        },
        {
            "name": "get_backtest_result",
            "description": "Fetch metrics and P&L for a completed backtest",
            "inputSchema": {
                "type": "object",
                "required": ["backtest_id"],
                "properties": {
                    "backtest_id": { "type": "string" }
                }
            }
        }
    ])
}
