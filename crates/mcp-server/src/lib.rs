//! MCP server library — thin front door to the strategy platform.
//!
//! Per ADR-0010, every tool that touches platform state goes through the
//! platform's own authenticated HTTP API (`ApiClient`); this crate holds no
//! database pools or managers of its own. Only the incremental draft builder
//! keeps process-local state (a scratchpad that persists nothing).
//!
//! No privileged path; no order-placement tool.

// The tool-definitions json! literal is large enough to exceed the default
// macro recursion limit.
#![recursion_limit = "256"]

pub mod tools;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use uuid::Uuid;

use tools::builder::StrategyDraft;

/// The full authoring protocol document served by `get_authoring_guide`.
pub fn authoring_guide() -> &'static str {
    include_str!("../docs/authoring_guide.md")
}

/// Short server instructions surfaced in the MCP `initialize` response.
pub fn server_instructions() -> &'static str {
    "This server exposes a trading-strategy research platform: design trading \
     algorithms as canonical JSON strategy definitions, backtest them against real \
     historical market data, read the results, and iterate. There is deliberately \
     no order-placement tool — this server never trades.\n\
     \n\
     Workflow: list_instruments (what data exists) → get_bars (inspect the \
     market) → write a definition → validate_strategy → create_strategy \
     (upserts by strategy_id slug; use versioned slugs like my_strat_v2 when \
     iterating) → create_backtest → wait_for_backtest (call repeatedly for \
     long runs; each call blocks up to timeout_seconds and returns progress on \
     timeout) → read results → compare_backtests across variants → iterate.\n\
     \n\
     Backtests run in the background and can take from minutes to over an hour. \
     Expressions use a tiny frozen grammar (feature('ema_7') > feature('ema_21')); \
     backtests support only ema_N and rsi_N features. If an instrument has no \
     stored history, init_asset onboards it (background seeding job).\n\
     \n\
     Call get_authoring_guide before authoring anything."
}

// ── HTTP client to the platform API ──────────────────────────────────────────

/// Error from a platform API call.
#[derive(Debug)]
pub struct ApiError {
    /// HTTP status, `None` for transport-level failures.
    pub status: Option<u16>,
    /// Response body (or a synthesized `{ "error": ... }` object).
    pub body: Value,
}

impl ApiError {
    fn network(e: impl std::fmt::Display) -> Self {
        Self {
            status: None,
            body: json!({ "error": "network_error", "detail": e.to_string() }),
        }
    }

    /// Render as a tool-result error object.
    pub fn to_tool_error(&self) -> Value {
        let mut obj = json!({
            "error": self
                .body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("api_error"),
            "detail": self.body,
        });
        if let (Some(status), Some(map)) = (self.status, obj.as_object_mut()) {
            map.insert("status".into(), json!(status));
        }
        obj
    }
}

/// Thin authenticated HTTP client for the platform API (ADR-0010 front door).
#[derive(Clone)]
pub struct ApiClient {
    base_url: String,
    bearer: String,
    http: reqwest::Client,
}

impl ApiClient {
    pub fn new(base_url: impl Into<String>, bearer: impl Into<String>) -> Self {
        let mut base_url = base_url.into();
        while base_url.ends_with('/') {
            base_url.pop();
        }
        Self {
            base_url,
            bearer: bearer.into(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
        }
    }

    /// Build from `PLATFORM_API_URL` (default `http://127.0.0.1:7080`) and
    /// `PLATFORM_API_TOKEN` (required).
    pub fn from_env() -> Result<Self, String> {
        let base_url = std::env::var("PLATFORM_API_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:7080".to_string());
        let bearer = std::env::var("PLATFORM_API_TOKEN").map_err(|_| {
            "PLATFORM_API_TOKEN is not set — mint a service token via \
             POST /auth/service-tokens (or Settings → Credentials → API Access) \
             and export it before starting the MCP server"
                .to_string()
        })?;
        if bearer.trim().is_empty() {
            return Err("PLATFORM_API_TOKEN is empty".to_string());
        }
        Ok(Self::new(base_url, bearer))
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value, ApiError> {
        let resp = req
            .header("authorization", format!("Bearer {}", self.bearer))
            .send()
            .await
            .map_err(ApiError::network)?;
        let status = resp.status();
        let text = resp.text().await.map_err(ApiError::network)?;
        let body: Value = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }))
        };
        if status.is_success() {
            Ok(body)
        } else {
            Err(ApiError {
                status: Some(status.as_u16()),
                body: if body.is_null() {
                    json!({ "error": format!("http_{}", status.as_u16()) })
                } else {
                    body
                },
            })
        }
    }

    pub async fn get(&self, path: &str) -> Result<Value, ApiError> {
        self.send(self.http.get(format!("{}{path}", self.base_url)))
            .await
    }

    pub async fn post(&self, path: &str, body: Value) -> Result<Value, ApiError> {
        self.send(
            self.http
                .post(format!("{}{path}", self.base_url))
                .json(&body),
        )
        .await
    }

    pub async fn delete(&self, path: &str) -> Result<Value, ApiError> {
        self.send(self.http.delete(format!("{}{path}", self.base_url)))
            .await
    }
}

// ── Context ──────────────────────────────────────────────────────────────────

/// Shared context injected into every MCP tool call.
#[derive(Clone)]
pub struct McpContext {
    /// Authenticated client for the platform API.
    pub api: ApiClient,
    /// In-memory draft store for the step-by-step strategy builder (scratchpad;
    /// nothing here persists until `finalize_strategy` posts to the platform).
    pub draft_store: Arc<Mutex<HashMap<Uuid, StrategyDraft>>>,
}

impl McpContext {
    pub fn new(api: ApiClient) -> Self {
        Self {
            api,
            draft_store: Arc::new(Mutex::new(HashMap::new())),
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

// ── Dispatch ─────────────────────────────────────────────────────────────────

/// Progress emitted by long-running tools (`wait_for_backtest`), forwarded to
/// streaming clients as MCP `notifications/progress` frames.
#[derive(Clone, Debug)]
pub struct ProgressUpdate {
    /// 0–100.
    pub progress: f64,
    pub message: String,
}

/// Dispatch an MCP JSON-RPC tool call and return the result as a JSON Value.
///
/// A result carrying a top-level `"error"` key should be surfaced to the MCP
/// client with `isError: true`.
pub async fn dispatch_tool(
    ctx: &McpContext,
    tool_name: &str,
    params: &Value,
    progress: Option<tokio::sync::mpsc::Sender<ProgressUpdate>>,
) -> Value {
    match tool_name {
        // ── Protocol ───────────────────────────────────────────────────────────
        "get_authoring_guide" => json!({ "guide": authoring_guide() }),

        // ── Discovery ──────────────────────────────────────────────────────────
        "list_lanes" => {
            let lanes = tools::discovery::list_lanes();
            json!({ "lanes": lanes })
        }
        "list_instruments" => tools::discovery::list_instruments(&ctx.api).await,

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
            tools::authoring::create_strategy(&ctx.api, definition_json).await
        }
        "get_strategy" => tools::authoring::get_strategy(&ctx.api, params).await,
        "list_strategies" => tools::authoring::list_strategies(&ctx.api).await,
        "list_compatible_strategies" => {
            tools::authoring::list_compatible_strategies(&ctx.api, params).await
        }

        // ── Market data ────────────────────────────────────────────────────────
        "get_bars" => tools::market::get_bars(&ctx.api, params).await,
        "get_instrument" => tools::market::get_instrument(&ctx.api, params).await,
        "list_asset_classes" => tools::market::list_asset_classes(&ctx.api).await,

        // ── Asset onboarding ───────────────────────────────────────────────────
        "list_initialized_assets" => tools::assets::list_initialized_assets(&ctx.api).await,
        "init_asset" => tools::assets::init_asset(&ctx.api, params).await,
        "get_asset_init_job" => tools::assets::get_asset_init_job(&ctx.api, params).await,

        // ── Portfolio observability (read-only) ────────────────────────────────
        "get_dashboard_rollup" => tools::portfolio::get_dashboard_rollup(&ctx.api).await,
        "get_paper_activity" => tools::portfolio::get_paper_activity(&ctx.api, params).await,
        "get_trading_status" => tools::portfolio::get_trading_status(&ctx.api).await,
        "get_order" => tools::portfolio::get_order(&ctx.api, params).await,

        // ── Model registry (read-only) ─────────────────────────────────────────
        "list_models" => tools::models::list_models(&ctx.api).await,
        "get_model" => tools::models::get_model(&ctx.api, params).await,

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
        "finalize_strategy" => tools::builder::finalize_strategy(ctx, params).await,

        // ── Backtests ──────────────────────────────────────────────────────────
        "list_backtests" => tools::backtests::list_backtests(&ctx.api, params).await,
        "get_backtest" => tools::backtests::get_backtest(&ctx.api, params).await,
        "create_backtest" => tools::backtests::create_backtest(&ctx.api, params).await,
        "stop_backtest" => tools::backtests::stop_backtest(&ctx.api, params).await,
        "rerun_backtest" => tools::backtests::rerun_backtest(&ctx.api, params).await,
        "delete_backtest" => tools::backtests::delete_backtest(&ctx.api, params).await,
        "compare_backtests" => tools::backtests::compare_backtests(&ctx.api, params).await,
        "wait_for_backtest" => {
            tools::backtests::wait_for_backtest(&ctx.api, params, progress).await
        }

        // ── Automations ────────────────────────────────────────────────────────
        "list_automations" => tools::automations::list_automations_tool(&ctx.api).await,
        "create_automation" => tools::automations::create_automation(&ctx.api, params).await,
        "arm_automation" => tools::automations::arm_automation(&ctx.api, params).await,
        "disarm_automation" => tools::automations::disarm_automation(&ctx.api, params).await,

        // ── Research (FEAT-003): Experiments, sweeps, gates, diagnostics ───────
        "create_experiment" => tools::research::create_experiment(&ctx.api, params).await,
        "list_experiments" => tools::research::list_experiments(&ctx.api).await,
        "get_experiment" => tools::research::get_experiment(&ctx.api, params).await,
        "run_sweep" => tools::research::run_sweep(&ctx.api, params).await,
        "get_sweep" => tools::research::get_sweep(&ctx.api, params).await,
        "cancel_sweep" => tools::research::cancel_sweep(&ctx.api, params).await,
        "run_study" => tools::research::run_study(&ctx.api, params).await,
        "list_studies" => tools::research::list_studies(&ctx.api, params).await,
        "get_carried_forward" => tools::research::get_carried_forward(&ctx.api, params).await,
        "get_diagnostics" => tools::research::get_diagnostics(&ctx.api, params).await,
        "get_funnel" => tools::research::get_funnel(&ctx.api, params).await,
        "advance_gate" => tools::research::advance_gate(&ctx.api, params).await,
        "get_null_picker" => tools::research::get_null_picker(&ctx.api, params).await,
        "choose_null" => tools::research::choose_null(&ctx.api, params).await,

        unknown => {
            json!({ "error": "unknown_tool", "tool": unknown })
        }
    }
}

// ── Tool definitions ─────────────────────────────────────────────────────────

/// Which tool subset a front door exposes.
///
/// `InternalAgent` is the reduced set used by the in-app agent: no draft
/// builder (the agent writes full JSON definitions) and no automations, which
/// keeps the tool context small enough for local models.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolProfile {
    Mcp,
    InternalAgent,
}

/// Tool names included in the internal-agent profile.
///
/// Everything except the step-by-step draft builder (redundant — the agent
/// writes full definition JSON) and automations (no standing trading config
/// from the in-app agent). Raw backtest *creation* is also excluded
/// (FEAT-003 §11): the agent may only cause Runs through Experiments and
/// Studies, so every result it sees is sealed and counted.
const INTERNAL_AGENT_TOOLS: &[&str] = &[
    "get_authoring_guide",
    "list_lanes",
    "list_instruments",
    "list_asset_classes",
    "get_instrument",
    "get_bars",
    "list_initialized_assets",
    "init_asset",
    "get_asset_init_job",
    "validate_strategy",
    "create_strategy",
    "get_strategy",
    "list_strategies",
    "list_compatible_strategies",
    "get_backtest",
    "wait_for_backtest",
    "list_backtests",
    "compare_backtests",
    "get_dashboard_rollup",
    "get_paper_activity",
    "get_trading_status",
    "get_order",
    "list_models",
    "get_model",
    // research (FEAT-003)
    "create_experiment",
    "list_experiments",
    "get_experiment",
    "run_sweep",
    "get_sweep",
    "cancel_sweep",
    "run_study",
    "list_studies",
    "get_carried_forward",
    "get_diagnostics",
    "get_funnel",
    "advance_gate",
    "get_null_picker",
    "choose_null",
];

/// The complete list of tools exposed by this MCP server (MCP profile).
pub fn tool_definitions() -> Value {
    tool_definitions_for(ToolProfile::Mcp)
}

/// Tool definitions filtered by profile.
pub fn tool_definitions_for(profile: ToolProfile) -> Value {
    let all = all_tool_definitions();
    match profile {
        ToolProfile::Mcp => all,
        ToolProfile::InternalAgent => {
            let filtered: Vec<Value> = all
                .as_array()
                .expect("tool definitions are an array")
                .iter()
                .filter(|t| {
                    t.get("name")
                        .and_then(|n| n.as_str())
                        .is_some_and(|n| INTERNAL_AGENT_TOOLS.contains(&n))
                })
                .cloned()
                .collect();
            Value::Array(filtered)
        }
    }
}

fn all_tool_definitions() -> Value {
    let mut all = json!([
        // ── Protocol ─────────────────────────────────────────────────────────
        {
            "name": "get_authoring_guide",
            "description": "Return the full strategy-authoring protocol: definition JSON format, expression grammar, supported features, backtest workflow, and result interpretation. Call this once before authoring anything.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        // ── Discovery ────────────────────────────────────────────────────────
        {
            "name": "list_lanes",
            "description": "Return the data lanes strategies can subscribe to (e.g. market.bars.1m, features.technical)",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "list_instruments",
            "description": "List instruments that have stored bar history, with per-timeframe coverage (bar count, first/last timestamp). Backtest inside the covered window, or rely on auto_collect to backfill.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "list_asset_classes",
            "description": "List the asset classes the platform supports (crypto_spot_cex is the v1 focus)",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_instrument",
            "description": "Fetch one instrument's registry row (venue, asset class, active flag)",
            "inputSchema": {
                "type": "object",
                "required": ["instrument_id"],
                "properties": {
                    "instrument_id": { "type": "string", "description": "e.g. BTC-USD" }
                }
            }
        },
        {
            "name": "get_bars",
            "description": "Fetch real OHLCV bars for an instrument over a time range, with full-window summary stats (return %, high/low). Use this to inspect market behavior before designing a strategy or to sanity-check a backtest window. Responses are capped: you get summary + the most recent max_bars bars; widen interval_seconds for long ranges.",
            "inputSchema": {
                "type": "object",
                "required": ["instrument_id", "start", "end"],
                "properties": {
                    "instrument_id": { "type": "string", "description": "e.g. BTC-USD" },
                    "start": { "type": "string", "description": "RFC3339 UTC start" },
                    "end": { "type": "string", "description": "RFC3339 UTC end" },
                    "interval_seconds": { "type": "integer", "description": "Bar size in seconds (default 3600; 60=1m, 86400=1d). Aggregated from stored 1m bars." },
                    "max_bars": { "type": "integer", "description": "Max bars returned (default 300, cap 2000); earlier bars are summarized, not returned" }
                }
            }
        },
        {
            "name": "list_initialized_assets",
            "description": "List symbols that have been initialized on the platform (seeded history + live 1-minute aggregation)",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "init_asset",
            "description": "Onboard a symbol: starts a background job that backfills historical bars and starts live 1-minute aggregation. Data collection only — never trades. Returns a job_id; poll get_asset_init_job until status is terminal, then the symbol is backtestable.",
            "inputSchema": {
                "type": "object",
                "required": ["symbol"],
                "properties": {
                    "symbol": { "type": "string", "description": "e.g. SOL-USD" },
                    "lookback_days": { "type": "integer", "description": "Days of history to backfill (default 90, max 3650)" },
                    "asset_class": { "type": "string", "description": "Default resolved from the registry, else crypto heuristic" }
                }
            }
        },
        {
            "name": "get_asset_init_job",
            "description": "Status of an init_asset seeding job (status, bars_collected, error)",
            "inputSchema": {
                "type": "object",
                "required": ["job_id"],
                "properties": {
                    "job_id": { "type": "string", "description": "UUID from init_asset" }
                }
            }
        },
        // ── Authoring ────────────────────────────────────────────────────────
        {
            "name": "validate_strategy",
            "description": "Validate a strategy definition JSON without persisting it; returns structured errors (path + message) you can fix and retry. Free and instant — validate early and often.",
            "inputSchema": {
                "type": "object",
                "required": ["definition_json"],
                "properties": {
                    "definition_json": { "type": "string", "description": "Strategy definition as a JSON string (format: see get_authoring_guide)" }
                }
            }
        },
        {
            "name": "create_strategy",
            "description": "Validate and persist a strategy definition to the platform's strategy library. Upserts by strategy_id slug — reusing a slug overwrites that strategy, so mint versioned slugs (my_strat_v2) when iterating.",
            "inputSchema": {
                "type": "object",
                "required": ["definition_json"],
                "properties": {
                    "definition_json": { "type": "string", "description": "Strategy definition as a JSON string" }
                }
            }
        },
        {
            "name": "get_strategy",
            "description": "Fetch a stored strategy definition by its strategy_id slug",
            "inputSchema": {
                "type": "object",
                "required": ["strategy_id"],
                "properties": {
                    "strategy_id": { "type": "string", "description": "Slug returned by create_strategy / list_strategies" }
                }
            }
        },
        {
            "name": "list_strategies",
            "description": "List all stored strategy slugs in the platform library",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "list_compatible_strategies",
            "description": "List stored strategies whose data requirements the given instrument/asset-class can satisfy (incompatible ones are omitted, with each strategy's kind, trigger, and required lanes)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "instrument_id": { "type": "string", "description": "Optional instrument filter" },
                    "asset_class": { "type": "string", "description": "Default crypto_spot_cex" }
                }
            }
        },
        // ── Strategy Builder (optional step-by-step alternative) ─────────────
        {
            "name": "new_strategy_draft",
            "description": "Create a new empty strategy draft; returns a draft_id for subsequent builder calls. Alternative to writing full definition JSON — drafts live only in this server's memory until finalize_strategy.",
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
            "description": "Set top-level strategy fields on a draft (strategy_id slug, asset_class, min_trust_tier)",
            "inputSchema": {
                "type": "object",
                "required": ["draft_id", "strategy_id", "asset_class"],
                "properties": {
                    "draft_id": { "type": "string" },
                    "strategy_id": { "type": "string", "description": "Human-readable slug, e.g. ema_cross_v1" },
                    "asset_class": { "type": "string", "description": "e.g. crypto_spot_cex (platform v1 focus is crypto)" },
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
                    "lane": { "type": "string", "description": "Lane name, e.g. market.bars.1m" },
                    "instrument": { "type": "string", "description": "Instrument ID or $bound_at_init (default)" },
                    "features": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Feature names for features.technical lanes (backtests support ema_N and rsi_N)"
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
            "description": "Append a Signal node that emits on the rising edge of a condition",
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
                    "size": { "type": "string", "description": "Decimal string quantity (e.g. \"0.02\" = 2% for percent_of_balance)" }
                }
            }
        },
        {
            "name": "set_risk_overrides",
            "description": "Set per-strategy risk overrides on the draft (tighten-only — loosening platform limits fails validation)",
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
            "description": "Validate the draft and persist it to the platform strategy library; returns the strategy_id slug on success or structured validation errors",
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
            "name": "create_backtest",
            "description": "Launch a backtest of a stored strategy (by slug) or an inline definition against real historical bars. Returns a backtest_id immediately; the run proceeds in the background (minutes to 1h+). Follow with wait_for_backtest.",
            "inputSchema": {
                "type": "object",
                "required": ["instrument_id", "timeframe", "start", "end"],
                "properties": {
                    "strategy_id": { "type": "string", "description": "Slug of a stored strategy (from create_strategy). Provide this OR definition_json." },
                    "definition_json": { "type": "string", "description": "Inline strategy definition JSON string (alternative to strategy_id)" },
                    "instrument_id": { "type": "string", "description": "e.g. BTC-USD (see list_instruments for coverage)" },
                    "asset_class": { "type": "string", "description": "Default crypto_spot_cex" },
                    "timeframe": { "type": "string", "enum": ["1s", "1m", "5m", "15m", "1h", "4h", "1d"], "description": "Bar timeframe. Note: a market.bars.<tf> input lane in the definition overrides this." },
                    "start": { "type": "string", "description": "RFC3339 UTC start, e.g. 2026-06-01T00:00:00Z" },
                    "end": { "type": "string", "description": "RFC3339 UTC end" },
                    "name": { "type": "string", "description": "Optional display name" },
                    "initial_balance": { "type": "string", "description": "Decimal starting balance (default 100000)" },
                    "quote_currency": { "type": "string", "description": "Quote currency (default USD)" },
                    "auto_collect": { "type": "boolean", "description": "Backfill missing history before simulating (default true; can dominate runtime on first touch)" }
                }
            }
        },
        {
            "name": "get_backtest",
            "description": "One-shot snapshot of a backtest run: status, progress %, error detail, and (when completed) the result document. detail:'summary' (default) truncates long arrays; use detail:'full' for every trade and the full equity curve.",
            "inputSchema": {
                "type": "object",
                "required": ["backtest_id"],
                "properties": {
                    "backtest_id": { "type": "string", "description": "UUID from create_backtest" },
                    "detail": { "type": "string", "enum": ["summary", "full"], "description": "Default summary" }
                }
            }
        },
        {
            "name": "wait_for_backtest",
            "description": "Block server-side until the backtest reaches a terminal state (completed/failed/cancelled) or timeout_seconds elapses. On timeout it returns {timed_out:true, status, progress} — simply call it again; chain calls to wait out hour-long runs without burning turns on manual polling.",
            "inputSchema": {
                "type": "object",
                "required": ["backtest_id"],
                "properties": {
                    "backtest_id": { "type": "string", "description": "UUID from create_backtest" },
                    "timeout_seconds": { "type": "integer", "description": "Max seconds this call blocks (default 300, clamp 10-600)" },
                    "poll_seconds": { "type": "integer", "description": "Seconds between status polls (default 5, clamp 2-60)" }
                }
            }
        },
        {
            "name": "list_backtests",
            "description": "List recent backtest runs (newest first) with status, progress and identifying fields",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max runs to return (default 20, max 100)" }
                }
            }
        },
        {
            "name": "stop_backtest",
            "description": "Cancel a running backtest",
            "inputSchema": {
                "type": "object",
                "required": ["backtest_id"],
                "properties": {
                    "backtest_id": { "type": "string", "description": "UUID of the run to cancel" }
                }
            }
        },
        {
            "name": "rerun_backtest",
            "description": "Start a fresh run with the same spec as an existing backtest; returns the new backtest_id",
            "inputSchema": {
                "type": "object",
                "required": ["backtest_id"],
                "properties": {
                    "backtest_id": { "type": "string", "description": "UUID of the run to copy" }
                }
            }
        },
        {
            "name": "delete_backtest",
            "description": "Delete a finished backtest run (terminal states only) — housekeeping for failed experiments",
            "inputSchema": {
                "type": "object",
                "required": ["backtest_id"],
                "properties": {
                    "backtest_id": { "type": "string", "description": "UUID of the run to delete" }
                }
            }
        },
        {
            "name": "compare_backtests",
            "description": "Side-by-side comparison of 2-5 backtest runs: identifying fields plus headline result metrics (summary, returns, order counts). Use after iterating to pick the best variant on evidence.",
            "inputSchema": {
                "type": "object",
                "required": ["backtest_ids"],
                "properties": {
                    "backtest_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "2-5 backtest UUIDs"
                    }
                }
            }
        },
        // ── Portfolio observability (read-only) ──────────────────────────────
        {
            "name": "get_dashboard_rollup",
            "description": "Account overview: balances, open positions, and P&L per asset class (read-only)",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_paper_activity",
            "description": "Paper-trading engine activity for one instrument: positions, fills, P&L (read-only)",
            "inputSchema": {
                "type": "object",
                "required": ["instrument_id"],
                "properties": {
                    "instrument_id": { "type": "string" }
                }
            }
        },
        {
            "name": "get_trading_status",
            "description": "Platform trading status: whether the kill switch is tripped (read-only)",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_order",
            "description": "Look up one order's status by id (read-only — there is no order-placement tool)",
            "inputSchema": {
                "type": "object",
                "required": ["order_id"],
                "properties": {
                    "order_id": { "type": "string", "description": "Order UUID" }
                }
            }
        },
        // ── Model registry (read-only) ───────────────────────────────────────
        {
            "name": "list_models",
            "description": "List registered AI models (kind, status). Definition v1.1 strategies can bind model outputs into feature slots via Inference nodes.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_model",
            "description": "One registered model's detail by id (read-only)",
            "inputSchema": {
                "type": "object",
                "required": ["model_id"],
                "properties": {
                    "model_id": { "type": "string" }
                }
            }
        },
        // ── Automations ──────────────────────────────────────────────────────
        {
            "name": "list_automations",
            "description": "List all automation plans (strategy↔instrument bindings that run when armed)",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "create_automation",
            "description": "Create a SingleInstrument automation that ties a stored strategy to an instrument. Live mode is blocked unless the operator sets MCP_ALLOW_LIVE_AUTOMATIONS.",
            "inputSchema": {
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
    ]);
    if let Some(arr) = all.as_array_mut() {
        arr.extend(tools::research::definitions());
    }
    all
}
