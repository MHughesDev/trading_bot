//! Integration tests for the MCP server tool workflows.
//!
//! The rewired server is a thin HTTP client of the platform API (ADR-0010),
//! so these tests run against a stub axum server on an ephemeral port — no
//! real platform, DB, or ClickHouse required.

use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use mcp_server_lib::{
    dispatch_tool, mcp_live_automations_allowed, tool_definitions, tool_definitions_for, ApiClient,
    McpContext, ToolProfile,
};

/// Shared stub-server state: strategies posted, a per-backtest poll counter so
/// `wait_for_backtest` sees an in-flight run resolve, and the audit trail the
/// platform would have written.
#[derive(Default)]
struct StubState {
    strategies: Vec<Value>,
    backtest_polls: usize,
    /// `(phase, action_or_audit_id)` in the order the platform was asked to
    /// write them. AT-64 is an assertion about this order.
    audit: Vec<(String, String)>,
    /// When true the stub refuses to open an audit record — the adversarial
    /// case. A dispatch must then not happen at all.
    audit_broken: bool,
    /// Every tool-bearing API path the stub was actually asked for, so a test
    /// can prove a refused dispatch touched nothing.
    api_calls: Vec<String>,
}

type Shared = Arc<Mutex<StubState>>;

async fn spawn_stub() -> (String, Shared) {
    let state: Shared = Arc::new(Mutex::new(StubState::default()));

    let app = Router::new()
        // The platform's audit front door (SPEC §15, AT-64). Every dispatch
        // opens a record here first; the id it returns is the proof the tool is
        // allowed to run at all.
        .route(
            "/api/agent/audit",
            post(
                |State(s): State<Shared>, Json(body): Json<Value>| async move {
                    let action = body
                        .get("action")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let mut st = s.lock().unwrap();
                    if st.audit_broken {
                        return (
                            axum::http::StatusCode::SERVICE_UNAVAILABLE,
                            Json(json!({ "error": "audit_unavailable" })),
                        );
                    }
                    let id = uuid::Uuid::new_v4();
                    st.audit.push(("pre".to_string(), action));
                    (
                        axum::http::StatusCode::CREATED,
                        Json(json!({ "audit_id": id })),
                    )
                },
            ),
        )
        .route(
            "/api/agent/audit/{audit_id}",
            post(
                |State(s): State<Shared>, Path(id): Path<String>, Json(_): Json<Value>| async move {
                    s.lock().unwrap().audit.push(("post".to_string(), id));
                    (
                        axum::http::StatusCode::CREATED,
                        Json(json!({ "recorded": true })),
                    )
                },
            ),
        )
        .route(
            "/api/strategies",
            get(|State(s): State<Shared>| async move {
                let list: Vec<Value> = s
                    .lock()
                    .unwrap()
                    .strategies
                    .iter()
                    .map(|d| {
                        let slug = d.get("strategy_id").cloned().unwrap_or_default();
                        json!({ "id": slug, "strategy_id": slug })
                    })
                    .collect();
                Json(json!({ "strategies": list }))
            })
            .post(
                |State(s): State<Shared>, Json(body): Json<Value>| async move {
                    let slug = body.get("strategy_id").cloned().unwrap_or_default();
                    s.lock().unwrap().strategies.push(body);
                    (
                        axum::http::StatusCode::CREATED,
                        Json(json!({ "id": slug, "strategy_id": slug })),
                    )
                },
            ),
        )
        .route(
            "/api/strategies/{id}/config",
            get(
                |State(s): State<Shared>, Path(id): Path<String>| async move {
                    let found = s
                        .lock()
                        .unwrap()
                        .strategies
                        .iter()
                        .find(|d| d.get("strategy_id").and_then(|v| v.as_str()) == Some(&id))
                        .cloned();
                    match found {
                        Some(def) => Json(json!({ "id": id, "definition": def })).into_response(),
                        None => (
                            axum::http::StatusCode::NOT_FOUND,
                            Json(json!({ "error": "not_found" })),
                        )
                            .into_response(),
                    }
                },
            ),
        )
        .route(
            "/api/backtests",
            get(|| async {
                Json(json!({ "backtests": [], "total": 0, "limit": 20, "offset": 0 }))
            })
            .post(|Json(body): Json<Value>| async move {
                // The platform requires a strategy_ref or definition.
                if body.get("strategy_ref").is_none() && body.get("definition").is_none() {
                    return (
                        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                        Json(json!({ "error": "missing_strategy" })),
                    );
                }
                (
                    axum::http::StatusCode::CREATED,
                    Json(json!({ "id": "6f9619ff-8b86-d011-b42d-00c04fc964ff" })),
                )
            }),
        )
        .route(
            "/api/backtests/{id}",
            get(
                |State(s): State<Shared>, Path(id): Path<String>| async move {
                    if id.starts_with("00000000") {
                        return (
                            axum::http::StatusCode::NOT_FOUND,
                            Json(json!({ "error": "not_found" })),
                        );
                    }
                    let polls = {
                        let mut st = s.lock().unwrap();
                        st.backtest_polls += 1;
                        st.backtest_polls
                    };
                    let (status, progress) = if polls < 2 {
                        ("simulating", 60.0)
                    } else {
                        ("completed", 100.0)
                    };
                    (
                        axum::http::StatusCode::OK,
                        Json(json!({
                            "id": id,
                            "status": status,
                            "progress": progress,
                            "result": if status == "completed" {
                                json!({ "summary": { "pnl": "12.5" } })
                            } else {
                                Value::Null
                            },
                        })),
                    )
                },
            ),
        )
        .route(
            "/assets/chart/bars",
            get(|| async {
                // 500 synthetic hourly bars with a rising close.
                let bars: Vec<Value> = (0..500)
                    .map(|i| {
                        json!({
                            "t": 1_750_000_000i64 + i * 3600,
                            "o": format!("{}", 100 + i),
                            "h": format!("{}", 101 + i),
                            "l": format!("{}", 99 + i),
                            "c": format!("{}", 100 + i),
                            "v": "10",
                        })
                    })
                    .collect();
                Json(json!({ "bars": bars }))
            }),
        )
        .route(
            "/api/market/instruments",
            get(|| async {
                Json(json!({ "instruments": [
                    { "instrument_id": "BTC-USD", "timeframe": "1m", "bars": 100000,
                      "first_ms": 1700000000000i64, "last_ms": 1750000000000i64 },
                    { "instrument_id": "BTC-USD", "timeframe": "1h", "bars": 2000,
                      "first_ms": 1700000000000i64, "last_ms": 1750000000000i64 },
                ] }))
            }),
        )
        .route(
            "/api/automations",
            get(|| async { Json(json!({ "automations": [] })) }).post(|| async {
                (
                    axum::http::StatusCode::CREATED,
                    Json(json!({ "id": "a", "armed": false })),
                )
            }),
        )
        .route(
            "/api/automations/{id}/arm",
            post(|| async {
                (
                    axum::http::StatusCode::NOT_FOUND,
                    Json(json!({ "error": "not_found" })),
                )
            }),
        )
        // Records every platform path the dispatch actually touched, so a test
        // can assert that a refused dispatch touched *nothing* — the difference
        // between refusing a call and logging that it happened.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            record_path,
        ))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), state)
}

use axum::response::IntoResponse;

/// Notes every non-audit platform path the stub was asked for.
async fn record_path(
    State(s): State<Shared>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = req.uri().path().to_string();
    if !path.starts_with("/api/agent/") {
        s.lock().unwrap().api_calls.push(path);
    }
    next.run(req).await
}

async fn ctx() -> (McpContext, Shared) {
    let (base_url, state) = spawn_stub().await;
    (
        McpContext::new(ApiClient::new(base_url, "test-token")),
        state,
    )
}

/// The audit phases the stub was asked to write, in order.
fn audit_phases(state: &Shared) -> Vec<String> {
    state
        .lock()
        .unwrap()
        .audit
        .iter()
        .map(|(phase, _)| phase.clone())
        .collect()
}

async fn call(ctx: &McpContext, tool: &str, params: Value) -> Value {
    dispatch_tool(ctx, tool, &params, None).await
}

/// Build a valid draft through the step-by-step tools; returns the draft_id.
async fn build_valid_draft(ctx: &McpContext, slug: &str) -> String {
    let r = call(ctx, "new_strategy_draft", json!({})).await;
    let draft_id = r
        .get("draft_id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_owned();
    call(
        ctx,
        "set_strategy_meta",
        json!({ "draft_id": draft_id, "strategy_id": slug, "asset_class": "crypto_spot_cex" }),
    )
    .await;
    call(
        ctx,
        "add_strategy_input",
        json!({ "draft_id": draft_id, "lane": "market.bars.1m" }),
    )
    .await;
    call(
        ctx,
        "add_strategy_input",
        json!({ "draft_id": draft_id, "lane": "features.technical",
                "features": ["ema_7", "ema_21"] }),
    )
    .await;
    call(
        ctx,
        "add_condition_node",
        json!({ "draft_id": draft_id, "node_id": "n1",
                "expr": "feature('ema_7') > feature('ema_21')" }),
    )
    .await;
    call(
        ctx,
        "add_signal_node",
        json!({ "draft_id": draft_id, "node_id": "n2", "when": "n1", "emit": "long" }),
    )
    .await;
    call(
        ctx,
        "add_strategy_action",
        json!({ "draft_id": draft_id, "on_signal": "long", "side": "buy",
                "size_mode": "fixed", "size": "0.01" }),
    )
    .await;
    draft_id
}

// ─── Workflow A: Draft → Finalize persists via the platform API ──────────────

#[tokio::test]
async fn workflow_a_draft_to_finalize_persists_via_api() {
    let (ctx, state) = ctx().await;
    let draft_id = build_valid_draft(&ctx, "ema_cross_v1").await;

    // Draft summary shows the accumulated definition.
    let r = call(&ctx, "get_draft_summary", json!({ "draft_id": draft_id })).await;
    assert!(r.get("error").is_none(), "get_draft_summary failed: {r}");
    assert_eq!(r.get("inputs").and_then(|v| v.as_array()).unwrap().len(), 2);

    // Finalize → validates locally, POSTs to /api/strategies, returns the slug.
    let r = call(&ctx, "finalize_strategy", json!({ "draft_id": draft_id })).await;
    assert_eq!(r.get("valid").and_then(|v| v.as_bool()), Some(true), "{r}");
    assert_eq!(
        r.get("strategy_id").and_then(|v| v.as_str()),
        Some("ema_cross_v1")
    );
    assert_eq!(state.lock().unwrap().strategies.len(), 1, "must persist");

    // Draft is gone after finalize.
    let r = call(&ctx, "discard_draft", json!({ "draft_id": draft_id })).await;
    assert_eq!(r.get("discarded").and_then(|v| v.as_bool()), Some(false));

    // list_strategies reflects the platform's stored list.
    let r = call(&ctx, "list_strategies", json!({})).await;
    let strategies = r.get("strategies").and_then(|v| v.as_array()).unwrap();
    assert!(strategies
        .iter()
        .any(|s| s.get("strategy_id").and_then(|v| v.as_str()) == Some("ema_cross_v1")));

    // get_strategy round-trips the stored definition.
    let r = call(
        &ctx,
        "get_strategy",
        json!({ "strategy_id": "ema_cross_v1" }),
    )
    .await;
    assert!(r.get("definition").is_some(), "{r}");
}

// ─── Workflow B: invalid draft surfaces validation errors, draft survives ────

#[tokio::test]
async fn workflow_b_invalid_draft_catches_errors() {
    let (ctx, state) = ctx().await;

    let r = call(&ctx, "new_strategy_draft", json!({})).await;
    let draft_id = r
        .get("draft_id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_owned();
    call(
        &ctx,
        "set_strategy_meta",
        json!({ "draft_id": draft_id, "strategy_id": "bad_strategy",
                "asset_class": "crypto_spot_cex" }),
    )
    .await;
    call(
        &ctx,
        "add_condition_node",
        json!({ "draft_id": draft_id, "node_id": "n1", "expr": ">>" }),
    )
    .await;
    call(
        &ctx,
        "add_signal_node",
        json!({ "draft_id": draft_id, "node_id": "n2", "when": "n1", "emit": "long" }),
    )
    .await;
    call(
        &ctx,
        "add_strategy_action",
        json!({ "draft_id": draft_id, "on_signal": "long", "side": "buy",
                "size_mode": "fixed", "size": "0.01" }),
    )
    .await;

    let r = call(&ctx, "finalize_strategy", json!({ "draft_id": draft_id })).await;
    assert_eq!(r.get("valid").and_then(|v| v.as_bool()), Some(false), "{r}");
    assert!(!r
        .get("errors")
        .and_then(|v| v.as_array())
        .unwrap()
        .is_empty());
    // Nothing reached the platform.
    assert_eq!(state.lock().unwrap().strategies.len(), 0);

    // Draft still accessible after a failed finalize; discard works.
    let r = call(&ctx, "get_draft_summary", json!({ "draft_id": draft_id })).await;
    assert!(r.get("error").is_none(), "{r}");
    let r = call(&ctx, "discard_draft", json!({ "draft_id": draft_id })).await;
    assert_eq!(r.get("discarded").and_then(|v| v.as_bool()), Some(true));
}

// ─── Backtests ───────────────────────────────────────────────────────────────

/// The dispatch path itself, not just the catalogue, refuses the bypass.
///
/// These two tests used to assert that `create_backtest` launched a run and that
/// it validated its arguments. Both behaviours were real — and both were the
/// problem: the tool reached `POST /api/backtests`, which runs a real simulation
/// through the legacy job tracker with no Experiment, no trial counter and no
/// ledger row. Removing it from the catalogue is not enough on its own, because a
/// model that remembers the old name can still name it; dispatch has to refuse.
#[tokio::test]
async fn dispatching_create_backtest_by_name_is_refused() {
    let (ctx, _state) = ctx().await;
    let r = call(
        &ctx,
        "create_backtest",
        json!({
            "strategy_id": "ema_cross_v1",
            "instrument_id": "BTC-USD",
            "timeframe": "1h",
            "start": "2026-06-01T00:00:00Z",
            "end": "2026-09-01T00:00:00Z"
        }),
    )
    .await;
    assert_eq!(
        r.get("error").and_then(|v| v.as_str()),
        Some("unknown_tool"),
        "a remembered tool name must not reach the legacy dispatch path: {r}"
    );
    assert!(
        r.get("backtest_id").is_none(),
        "nothing may have been dispatched: {r}"
    );
}

#[tokio::test]
async fn wait_for_backtest_polls_until_terminal() {
    let (ctx, state) = ctx().await;
    let r = call(
        &ctx,
        "wait_for_backtest",
        json!({
            "backtest_id": "6f9619ff-8b86-d011-b42d-00c04fc964ff",
            "timeout_seconds": 30,
            "poll_seconds": 2
        }),
    )
    .await;
    assert_eq!(r.get("status").and_then(|v| v.as_str()), Some("completed"));
    assert!(r.get("timed_out").is_none(), "{r}");
    assert!(
        state.lock().unwrap().backtest_polls >= 2,
        "must have polled"
    );
}

#[tokio::test]
async fn get_backtest_not_found_surfaces_status() {
    let (ctx, _state) = ctx().await;
    let r = call(
        &ctx,
        "get_backtest",
        json!({ "backtest_id": "00000000-0000-0000-0000-000000000001" }),
    )
    .await;
    assert_eq!(r.get("error").and_then(|v| v.as_str()), Some("not_found"));
    assert_eq!(r.get("status").and_then(|v| v.as_u64()), Some(404));
}

// ─── Discovery ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_instruments_groups_coverage_per_instrument() {
    let (ctx, _state) = ctx().await;
    let r = call(&ctx, "list_instruments", json!({})).await;
    let instruments = r.get("instruments").and_then(|v| v.as_array()).unwrap();
    assert_eq!(instruments.len(), 1, "two rows, one instrument: {r}");
    let coverage = instruments[0]
        .get("coverage")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(coverage.len(), 2);
}

#[tokio::test]
async fn get_bars_truncates_but_summarizes_full_window() {
    let (ctx, _state) = ctx().await;
    let r = call(
        &ctx,
        "get_bars",
        json!({
            "instrument_id": "BTC-USD",
            "start": "2026-06-01T00:00:00Z",
            "end": "2026-06-22T00:00:00Z",
            "max_bars": 100
        }),
    )
    .await;
    assert!(r.get("error").is_none(), "{r}");
    assert_eq!(r["bars"].as_array().unwrap().len(), 100);
    assert_eq!(r["omitted_earlier_bars"], 400);
    // Summary spans the full 500-bar window: first close 100, last close 599.
    assert_eq!(r["summary"]["bar_count"], 500);
    assert_eq!(r["summary"]["first_close"], 100.0);
    assert_eq!(r["summary"]["last_close"], 599.0);
}

#[tokio::test]
async fn compare_backtests_returns_side_by_side_rows() {
    let (ctx, _state) = ctx().await;
    let r = call(
        &ctx,
        "compare_backtests",
        json!({ "backtest_ids": [
            "6f9619ff-8b86-d011-b42d-00c04fc964ff",
            "6f9619ff-8b86-d011-b42d-00c04fc964fe"
        ] }),
    )
    .await;
    let rows = r["comparison"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    // The stub's completed snapshots expose result.summary in metrics.
    assert!(rows
        .iter()
        .any(|row| row["metrics"].get("summary").is_some()));

    // Wrong cardinality is rejected locally.
    let r = call(
        &ctx,
        "compare_backtests",
        json!({ "backtest_ids": ["one"] }),
    )
    .await;
    assert_eq!(r["error"]["code"], "invalid_request");
}

// ─── Automations ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn automations_route_through_api_and_live_is_gated() {
    let (ctx, _state) = ctx().await;

    let r = call(&ctx, "list_automations", json!({})).await;
    assert_eq!(
        r.get("automations")
            .and_then(|v| v.as_array())
            .unwrap()
            .len(),
        0
    );

    // Live mode blocked locally (env gate) before any HTTP happens.
    std::env::remove_var("MCP_ALLOW_LIVE_AUTOMATIONS");
    let r = call(
        &ctx,
        "create_automation",
        json!({
            "execution_strategy_id": "ema_cross_v1",
            "instrument_id": "BTC-USD",
            "asset_class": "crypto_spot_cex",
            "account_mode": "live"
        }),
    )
    .await;
    assert_eq!(
        r.pointer("/error/code").and_then(|v| v.as_str()),
        Some("live_automations_disabled")
    );

    // Paper mode goes through to the API.
    let r = call(
        &ctx,
        "create_automation",
        json!({
            "execution_strategy_id": "ema_cross_v1",
            "instrument_id": "BTC-USD",
            "asset_class": "crypto_spot_cex",
            "account_mode": "paper"
        }),
    )
    .await;
    assert!(r.get("error").is_none(), "{r}");

    // Arm on an unknown id surfaces the API's 404.
    let r = call(
        &ctx,
        "arm_automation",
        json!({ "automation_id": "00000000-0000-0000-0000-000000000001" }),
    )
    .await;
    assert_eq!(r.get("error").and_then(|v| v.as_str()), Some("not_found"));
}

// ─── Unit: builder edge cases (no HTTP involved) ─────────────────────────────

#[tokio::test]
async fn duplicate_node_id_returns_error() {
    let (ctx, _state) = ctx().await;
    let r = call(&ctx, "new_strategy_draft", json!({})).await;
    let draft_id = r
        .get("draft_id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_owned();
    let node = json!({ "draft_id": draft_id, "node_id": "n1",
                       "expr": "feature('ema_7') > feature('ema_21')" });
    call(&ctx, "add_condition_node", node.clone()).await;
    let r = call(&ctx, "add_condition_node", node).await;
    assert_eq!(
        r.pointer("/error/code").and_then(|v| v.as_str()),
        Some("duplicate_node_id")
    );
}

#[tokio::test]
async fn discard_unknown_draft_returns_false() {
    let (ctx, _state) = ctx().await;
    let r = call(
        &ctx,
        "discard_draft",
        json!({ "draft_id": "00000000-0000-0000-0000-000000000099" }),
    )
    .await;
    assert_eq!(r.get("discarded").and_then(|v| v.as_bool()), Some(false));
}

// ─── Unit: gating + tool definitions ─────────────────────────────────────────

#[test]
fn live_automations_default_off() {
    std::env::remove_var("MCP_ALLOW_LIVE_AUTOMATIONS");
    assert!(!mcp_live_automations_allowed());
}

#[test]
fn tool_definitions_is_valid_json_array() {
    let defs = tool_definitions();
    let arr = defs.as_array().expect("tool_definitions is a JSON array");
    assert!(
        arr.len() >= 20,
        "expected at least 20 tools, got {}",
        arr.len()
    );
    for tool in arr {
        assert!(tool.get("name").is_some(), "tool missing 'name': {tool}");
        assert!(
            tool.get("description").is_some(),
            "tool missing 'description': {tool}"
        );
        assert!(
            tool.get("inputSchema").is_some(),
            "tool missing 'inputSchema': {tool}"
        );
    }
    // The lifecycle no-ops are gone.
    let names: Vec<&str> = arr
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();
    assert!(!names.contains(&"apply_strategy"));
    assert!(names.contains(&"wait_for_backtest"));
    assert!(names.contains(&"get_authoring_guide"));
}

#[test]
fn internal_agent_profile_is_a_reduced_subset() {
    let mcp = tool_definitions_for(ToolProfile::Mcp);
    let internal = tool_definitions_for(ToolProfile::InternalAgent);
    let mcp_names: Vec<&str> = mcp
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();
    let internal_names: Vec<&str> = internal
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();
    assert!(internal_names.len() < mcp_names.len());
    assert!(internal_names.iter().all(|n| mcp_names.contains(n)));
    // No draft builder or automations for the internal agent.
    assert!(!internal_names.contains(&"new_strategy_draft"));
    assert!(!internal_names.contains(&"create_automation"));
    assert!(internal_names.contains(&"wait_for_backtest"));
}

/// INV-16: no compute is dispatched without a `REGISTERED` trial row, and there
/// is no bypass at any permission level.
///
/// `create_backtest` was that bypass. It called `POST /api/backtests` directly,
/// which runs a real simulation against real bars through the legacy job tracker
/// — no Experiment, no trial counter, no propensity, no ledger row of any kind.
/// An agent could therefore burn compute and read a Sharpe that the platform's
/// own trial accounting never saw, which is precisely the "a side door exists,
/// so an audit will find it was used" failure the invariant exists to prevent.
///
/// The sanctioned path is create_experiment → run_sweep: every member registers
/// before it executes, so every look is counted.
///
/// If this test starts failing, the tool came back. Do not re-add it to make the
/// test pass — the invariant is the point, not the assertion.
#[test]
fn no_tool_profile_can_dispatch_an_unregistered_backtest() {
    for profile in [ToolProfile::Mcp, ToolProfile::InternalAgent] {
        let defs = tool_definitions_for(profile);
        let names: Vec<&str> = defs
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(
            !names.contains(&"create_backtest"),
            "{profile:?} exposes create_backtest — that dispatches compute with no trial row (INV-16)"
        );
        // The sanctioned path must still exist, or the tool was removed without
        // leaving anything that can legitimately run a backtest.
        assert!(
            names.contains(&"create_experiment") && names.contains(&"run_sweep"),
            "{profile:?} must retain the registered path (create_experiment + run_sweep); got {names:?}"
        );
    }
}

// ── AT-64 · the external front door's audit trail ────────────────────────────
//
// These run through exactly the code path `apps/mcp-server` runs: an
// `McpContext` over an `ApiClient`, and `dispatch_tool`. There is nothing the
// external process does that these do not, which is the point — the front door
// used to be exempt from the audit trail, and the exemption was structural
// rather than accidental.

/// AT-64 · every dispatch through the external front door opens a `pre` record
/// before the tool runs and closes it after (SPEC §15, ADR-P2-21, ADR-P2-34).
#[tokio::test]
async fn at64_the_external_front_door_writes_pre_and_post_around_every_dispatch() {
    let (ctx, state) = ctx().await;

    let out = call(&ctx, "list_instruments", json!({})).await;
    assert!(out.get("instruments").is_some(), "the tool ran: {out}");

    let trail = state.lock().unwrap().audit.clone();
    assert_eq!(
        trail.len(),
        2,
        "one dispatch must write exactly one pair, not zero and not two: {trail:?}"
    );
    assert_eq!(trail[0].0, "pre", "the pre record comes first");
    assert_eq!(
        trail[0].1, "list_instruments",
        "the record names the action asked for"
    );
    assert_eq!(trail[1].0, "post");

    // And a second call adds a second pair, so the trail counts looks rather
    // than sessions.
    call(&ctx, "list_lanes", json!({})).await;
    assert_eq!(audit_phases(&state), vec!["pre", "post", "pre", "post"]);
}

/// AT-64 adversarial · a dispatch the platform will not record **does not
/// happen**.
///
/// This is the half that makes the invariant a property rather than a claim. It
/// is not enough that a failure to audit is logged: the tool must not run. The
/// assertion that carries the weight is the last one — the stub was asked for no
/// platform path at all, so the call was refused rather than executed and
/// regretted.
#[tokio::test]
async fn at64_a_dispatch_the_platform_will_not_record_is_refused_not_logged() {
    let (ctx, state) = ctx().await;
    state.lock().unwrap().audit_broken = true;

    let out = call(&ctx, "list_instruments", json!({})).await;

    assert_eq!(
        out.get("error").and_then(|e| e.as_str()),
        Some("audit_unavailable"),
        "the dispatch must be refused, and say why: {out}"
    );
    assert!(
        out.get("instruments").is_none(),
        "no tool output may come back from a call that was never allowed to run: {out}"
    );
    assert!(
        state.lock().unwrap().audit.is_empty(),
        "nothing was recorded, which is exactly why nothing may have run"
    );
    assert!(
        state.lock().unwrap().api_calls.is_empty(),
        "the tool touched the platform despite having no audit record: {:?}",
        state.lock().unwrap().api_calls
    );
}

/// AT-64 adversarial · the refusal covers **every** tool, including the ones
/// answered from process memory.
///
/// The tempting exemption is the other direction from the one this track closed:
/// `list_lanes`, `get_authoring_guide` and the whole draft builder never touch
/// the platform, so it looks free to let them through when the trail is down.
/// It is not. A look at the lane catalogue is a look, `new_strategy_draft` is the
/// first step of authoring, and an audit with a "cheap tools" carve-out is an
/// audit with a list of tools to hide behind.
#[tokio::test]
async fn at64_even_a_tool_answered_from_memory_is_refused_without_a_record() {
    let (ctx, state) = ctx().await;
    state.lock().unwrap().audit_broken = true;

    for tool in ["list_lanes", "get_authoring_guide", "new_strategy_draft"] {
        let out = call(&ctx, tool, json!({})).await;
        assert_eq!(
            out.get("error").and_then(|e| e.as_str()),
            Some("audit_unavailable"),
            "{tool} ran without an audit record: {out}"
        );
    }
    assert!(state.lock().unwrap().audit.is_empty());
}

/// AT-64 · the trail survives a failing tool, because a failure is a look too.
///
/// A tool that errors has still been dispatched, and the `post` record is what
/// tells a reader afterwards that the platform answered rather than that the
/// agent never asked.
#[tokio::test]
async fn at64_a_failing_tool_still_closes_its_record() {
    let (ctx, state) = ctx().await;
    let out = call(
        &ctx,
        "get_backtest",
        json!({ "backtest_id": "00000000-0000-0000-0000-000000000000" }),
    )
    .await;
    assert!(out.get("error").is_some(), "the stub 404s this id: {out}");
    assert_eq!(audit_phases(&state), vec!["pre", "post"]);
}

/// AT-64 · an unknown tool is a look at the tool surface, and it is recorded.
///
/// The failure mode this guards is a dispatcher that opens the record only on
/// the paths it recognises: an agent probing for a tool that does not exist is
/// exactly the behaviour an audit is read to find.
#[tokio::test]
async fn at64_an_unknown_tool_is_still_recorded() {
    let (ctx, state) = ctx().await;
    let out = call(&ctx, "open_vault", json!({})).await;
    assert!(out.get("error").is_some(), "no such tool: {out}");
    let trail = state.lock().unwrap().audit.clone();
    assert_eq!(trail.len(), 2, "{trail:?}");
    assert_eq!(trail[0].1, "open_vault");
}
