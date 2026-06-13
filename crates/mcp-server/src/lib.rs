//! MCP server library — thin front door to the strategy platform.
//!
//! All tools route through the shared validator and runtime.
//! No privileged path; no order-placement tool.

pub mod tools;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sqlx::PgPool;
use uuid::Uuid;

use backtest::manager::BacktestManager;
use demand_manager::{DemandRegistry, NoopPipelineFactory};
use domain::strategy_def::StrategyDefinition;
use strategy_runtime::InstanceManager;

use tools::builder::StrategyDraft;

/// Shared context injected into every MCP tool call.
#[derive(Clone)]
pub struct McpContext {
    pub strategy_store: Arc<Mutex<HashMap<Uuid, StrategyDefinition>>>,
    pub instance_manager: Arc<Mutex<InstanceManager>>,
    /// Postgres connection pool — `None` when `DATABASE_URL` is not set.
    pub pg: Option<PgPool>,
    /// In-memory draft store for the step-by-step strategy builder.
    pub draft_store: Arc<Mutex<HashMap<Uuid, StrategyDraft>>>,
    /// Backtest manager — `None` when CLICKHOUSE_URL or DATABASE_URL is not set.
    pub backtest_manager: Option<Arc<BacktestManager>>,
}

impl McpContext {
    pub fn new_without_db() -> Self {
        let demand = Arc::new(DemandRegistry::new(Arc::new(NoopPipelineFactory)));
        Self {
            strategy_store: Arc::new(Mutex::new(HashMap::new())),
            instance_manager: Arc::new(Mutex::new(InstanceManager::new(demand))),
            pg: None,
            draft_store: Arc::new(Mutex::new(HashMap::new())),
            backtest_manager: None,
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
                tracing::info!(
                    "MCP: DATABASE_URL not set; discovery will return empty instrument list"
                );
                None
            }
        };

        let backtest_manager = match (std::env::var("CLICKHOUSE_URL"), pg.as_ref()) {
            (Ok(ch_url), Some(pool)) => {
                tracing::info!("MCP: BacktestManager configured");
                Some(BacktestManager::new(ch_url, pool.clone()))
            }
            _ => {
                tracing::info!("MCP: CLICKHOUSE_URL or DATABASE_URL not set; backtest tools will return service_unavailable");
                None
            }
        };

        Self {
            strategy_store: Arc::new(Mutex::new(HashMap::new())),
            instance_manager: Arc::new(Mutex::new(InstanceManager::new(demand))),
            pg,
            draft_store: Arc::new(Mutex::new(HashMap::new())),
            backtest_manager,
        }
    }
}

/// Whether live automations are permitted via the MCP server.
///
/// Off by default; set `MCP_ALLOW_LIVE_AUTOMATIONS=true` (or `=1`) to enable.
pub fn mcp_live_automations_allowed() -> bool {
    std::env::var("MCP_ALLOW_LIVE_AUTOMATIONS")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false)
}

/// Dispatch an MCP JSON-RPC tool call and return the result as a JSON Value.
pub async fn dispatch_tool(
    ctx: &McpContext,
    tool_name: &str,
    params: &serde_json::Value,
) -> serde_json::Value {
    use serde_json::json;

    match tool_name {
        // ── Discovery ──────────────────────────────────────────────────────────
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

        // ── Authoring ──────────────────────────────────────────────────────────
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

        // ── Lifecycle ──────────────────────────────────────────────────────────
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

        // ── Strategy Builder ───────────────────────────────────────────────────
        "new_strategy_draft" => tools::builder::new_strategy_draft(ctx),
        "discard_draft" => tools::builder::discard_draft(ctx, params),
        "set_strategy_meta" => tools::builder::set_strategy_meta(ctx, params),
        "add_strategy_input" => tools::builder::add_strategy_input(ctx, params),
        "add_condition_node" => tools::builder::add_condition_node(ctx, params),
        "add_signal_node" => tools::builder::add_signal_node(ctx, params),
        "add_strategy_action" => tools::builder::add_strategy_action(ctx, params),
        "set_risk_overrides" => tools::builder::set_risk_overrides(ctx, params),
        "get_draft_summary" => tools::builder::get_draft_summary(ctx, params),
        "finalize_strategy" => tools::builder::finalize_strategy(ctx, params),

        // ── Backtests ──────────────────────────────────────────────────────────
        "list_backtests" => tools::backtests::list_backtests(ctx).await,
        "get_backtest" => tools::backtests::get_backtest(ctx, params).await,
        "create_backtest" => tools::backtests::create_backtest(ctx, params).await,

        // ── Automations ────────────────────────────────────────────────────────
        "list_automations" => tools::automations::list_automations_tool(ctx).await,
        "create_automation" => tools::automations::create_automation(ctx, params).await,
        "arm_automation" => tools::automations::arm_automation(ctx, params).await,
        "disarm_automation" => tools::automations::disarm_automation(ctx, params).await,

        unknown => {
            json!({ "error": "unknown_tool", "tool": unknown })
        }
    }
}

/// The complete list of tools exposed by this MCP server.
pub fn tool_definitions() -> serde_json::Value {
    use serde_json::json;
    json!([
        // ── Discovery ────────────────────────────────────────────────────────
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
        // ── Authoring ────────────────────────────────────────────────────────
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
        // ── Lifecycle ────────────────────────────────────────────────────────
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
        // ── Strategy Builder ─────────────────────────────────────────────────
        {
            "name": "new_strategy_draft",
            "description": "Create a new empty strategy draft; returns a draft_id to use in subsequent builder calls",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "discard_draft",
            "description": "Discard a strategy draft by draft_id",
            "inputSchema": {
                "type": "object",
                "required": ["draft_id"],
                "properties": {
                    "draft_id": { "type": "string", "description": "UUID of the draft to discard" }
                }
            }
        },
        {
            "name": "set_strategy_meta",
            "description": "Set top-level strategy fields on a draft (strategy_id, asset_class, min_trust_tier)",
            "inputSchema": {
                "type": "object",
                "required": ["draft_id", "strategy_id", "asset_class"],
                "properties": {
                    "draft_id": { "type": "string" },
                    "strategy_id": { "type": "string", "description": "Human-readable slug, e.g. ema_cross_v1" },
                    "asset_class": { "type": "string", "description": "e.g. crypto_spot_cex" },
                    "min_trust_tier": { "type": "string", "description": "Optional trust tier override" }
                }
            }
        },
        {
            "name": "add_strategy_input",
            "description": "Append an input lane subscription to the draft",
            "inputSchema": {
                "type": "object",
                "required": ["draft_id", "lane"],
                "properties": {
                    "draft_id": { "type": "string" },
                    "lane": { "type": "string", "description": "NATS lane, e.g. market.bars.1m" },
                    "instrument": { "type": "string", "description": "Instrument ID or $bound_at_init (default)" },
                    "features": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Feature names (for features.* lanes)"
                    }
                }
            }
        },
        {
            "name": "add_condition_node",
            "description": "Append a Condition node to the draft",
            "inputSchema": {
                "type": "object",
                "required": ["draft_id", "node_id", "expr"],
                "properties": {
                    "draft_id": { "type": "string" },
                    "node_id": { "type": "string", "description": "Unique node ID within this strategy" },
                    "expr": { "type": "string", "description": "Predicate expression, e.g. feature('ema_7') > feature('ema_21')" }
                }
            }
        },
        {
            "name": "add_signal_node",
            "description": "Append a Signal node that emits when a condition is true",
            "inputSchema": {
                "type": "object",
                "required": ["draft_id", "node_id", "when", "emit"],
                "properties": {
                    "draft_id": { "type": "string" },
                    "node_id": { "type": "string" },
                    "when": { "type": "string", "description": "ID of the condition node to watch" },
                    "emit": { "type": "string", "description": "Named signal to emit, e.g. long" }
                }
            }
        },
        {
            "name": "add_strategy_action",
            "description": "Append a PlaceOrder action triggered by a named signal",
            "inputSchema": {
                "type": "object",
                "required": ["draft_id", "on_signal", "side", "size_mode", "size"],
                "properties": {
                    "draft_id": { "type": "string" },
                    "on_signal": { "type": "string", "description": "Signal name that triggers this action" },
                    "side": { "type": "string", "enum": ["buy", "sell"] },
                    "size_mode": { "type": "string", "enum": ["fixed", "percent_of_balance", "risk_unit"] },
                    "size": { "type": "string", "description": "Decimal string quantity" }
                }
            }
        },
        {
            "name": "set_risk_overrides",
            "description": "Set per-strategy risk overrides on the draft (tighten-only)",
            "inputSchema": {
                "type": "object",
                "required": ["draft_id"],
                "properties": {
                    "draft_id": { "type": "string" },
                    "max_position": { "type": "string", "description": "Decimal max position size" },
                    "max_order_rate_per_minute": { "type": "integer" },
                    "max_order_rate_per_second": { "type": "integer" }
                }
            }
        },
        {
            "name": "get_draft_summary",
            "description": "Return the current draft definition as JSON without mutating it",
            "inputSchema": {
                "type": "object",
                "required": ["draft_id"],
                "properties": {
                    "draft_id": { "type": "string" }
                }
            }
        },
        {
            "name": "finalize_strategy",
            "description": "Validate and persist the draft as a strategy; returns store_id on success or validation errors on failure",
            "inputSchema": {
                "type": "object",
                "required": ["draft_id"],
                "properties": {
                    "draft_id": { "type": "string" }
                }
            }
        },
        // ── Backtests ────────────────────────────────────────────────────────
        {
            "name": "list_backtests",
            "description": "List all backtest runs",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_backtest",
            "description": "Get the full snapshot for one backtest run including results",
            "inputSchema": {
                "type": "object",
                "required": ["backtest_id"],
                "properties": {
                    "backtest_id": { "type": "string", "description": "UUID of the backtest run" }
                }
            }
        },
        {
            "name": "create_backtest",
            "description": "Trigger a new backtest run against a stored strategy",
            "inputSchema": {
                "type": "object",
                "required": ["store_id", "instrument_id", "asset_class", "timeframe", "start", "end"],
                "properties": {
                    "store_id": { "type": "string", "description": "UUID from create_strategy or finalize_strategy" },
                    "instrument_id": { "type": "string", "description": "e.g. BTC-USDT" },
                    "asset_class": { "type": "string", "description": "e.g. crypto_spot_cex" },
                    "timeframe": { "type": "string", "enum": ["1s", "1m", "5m", "15m", "1h", "4h", "1d"] },
                    "start": { "type": "string", "description": "RFC3339 start datetime" },
                    "end": { "type": "string", "description": "RFC3339 end datetime" },
                    "name": { "type": "string", "description": "Optional display name" },
                    "initial_balance": { "type": "string", "description": "Decimal starting balance (default 100000)" },
                    "quote_currency": { "type": "string", "description": "Quote currency (default USD)" },
                    "auto_collect": { "type": "boolean", "description": "Backfill missing data (default true)" }
                }
            }
        },
        // ── Automations ──────────────────────────────────────────────────────
        {
            "name": "list_automations",
            "description": "List all automation plans",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "create_automation",
            "description": "Create a SingleInstrument automation that ties a strategy to an instrument",
            "inputSchema": {
                "type": "object",
                "required": ["execution_strategy_id", "instrument_id", "asset_class", "account_mode"],
                "properties": {
                    "execution_strategy_id": { "type": "string", "description": "UUID of the stored strategy" },
                    "instrument_id": { "type": "string" },
                    "asset_class": { "type": "string" },
                    "account_mode": { "type": "string", "enum": ["paper", "live"] },
                    "armed": { "type": "boolean", "description": "Start armed (default false)" },
                    "time_window_start": { "type": "string", "description": "HH:MM trading window open" },
                    "time_window_end": { "type": "string", "description": "HH:MM trading window close" },
                    "time_window_tz": { "type": "string", "description": "IANA timezone (default UTC)" }
                }
            }
        },
        {
            "name": "arm_automation",
            "description": "Arm an automation by ID",
            "inputSchema": {
                "type": "object",
                "required": ["automation_id"],
                "properties": {
                    "automation_id": { "type": "string" }
                }
            }
        },
        {
            "name": "disarm_automation",
            "description": "Disarm an automation by ID",
            "inputSchema": {
                "type": "object",
                "required": ["automation_id"],
                "properties": {
                    "automation_id": { "type": "string" }
                }
            }
        }
    ])
}
