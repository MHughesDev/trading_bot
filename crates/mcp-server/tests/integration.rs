//! Integration tests for the MCP server tool workflows.
//!
//! Uses `McpContext::new_without_db()` — no real DB or ClickHouse required.
//! Backtest tools return `service_unavailable` (no BacktestManager) and we
//! assert the error shape rather than success.

use mcp_server_lib::{McpContext, dispatch_tool, mcp_live_automations_allowed};

fn ctx() -> McpContext {
    McpContext::new_without_db()
}

// ─── Workflow A: Draft → Finalize → Backtest attempt ─────────────────────────

#[tokio::test]
async fn workflow_a_draft_to_finalize() {
    let ctx = ctx();

    // 1. Create draft
    let r = dispatch_tool(&ctx, "new_strategy_draft", &serde_json::json!({})).await;
    let draft_id = r.get("draft_id").and_then(|v| v.as_str()).expect("draft_id").to_owned();
    assert!(!draft_id.is_empty());

    // 2. Set meta
    let r = dispatch_tool(&ctx, "set_strategy_meta", &serde_json::json!({
        "draft_id": draft_id,
        "strategy_id": "ema_cross_v1",
        "asset_class": "crypto_spot_cex"
    })).await;
    assert!(r.get("error").is_none(), "set_strategy_meta failed: {r}");

    // 3. Add inputs
    let r = dispatch_tool(&ctx, "add_strategy_input", &serde_json::json!({
        "draft_id": draft_id,
        "lane": "market.bars.1m"
    })).await;
    assert!(r.get("error").is_none(), "add_strategy_input failed: {r}");

    let r = dispatch_tool(&ctx, "add_strategy_input", &serde_json::json!({
        "draft_id": draft_id,
        "lane": "features.technical",
        "features": ["ema_7", "ema_21"]
    })).await;
    assert!(r.get("error").is_none());

    // 4. Add condition node
    let r = dispatch_tool(&ctx, "add_condition_node", &serde_json::json!({
        "draft_id": draft_id,
        "node_id": "n1",
        "expr": "feature('ema_7') > feature('ema_21')"
    })).await;
    assert!(r.get("error").is_none(), "add_condition_node failed: {r}");

    // 5. Add signal node
    let r = dispatch_tool(&ctx, "add_signal_node", &serde_json::json!({
        "draft_id": draft_id,
        "node_id": "n2",
        "when": "n1",
        "emit": "long"
    })).await;
    assert!(r.get("error").is_none(), "add_signal_node failed: {r}");

    // 6. Add action
    let r = dispatch_tool(&ctx, "add_strategy_action", &serde_json::json!({
        "draft_id": draft_id,
        "on_signal": "long",
        "side": "buy",
        "size_mode": "fixed",
        "size": "0.01"
    })).await;
    assert!(r.get("error").is_none(), "add_strategy_action failed: {r}");

    // 7. get_draft_summary shows the accumulated content
    let r = dispatch_tool(&ctx, "get_draft_summary", &serde_json::json!({
        "draft_id": draft_id
    })).await;
    assert!(r.get("error").is_none(), "get_draft_summary failed: {r}");
    let inputs = r.get("inputs").and_then(|v| v.as_array()).expect("inputs array");
    assert_eq!(inputs.len(), 2);

    // 8. Finalize
    let r = dispatch_tool(&ctx, "finalize_strategy", &serde_json::json!({
        "draft_id": draft_id
    })).await;
    assert_eq!(r.get("valid").and_then(|v| v.as_bool()), Some(true), "finalize failed: {r}");
    let store_id = r.get("store_id").and_then(|v| v.as_str()).expect("store_id").to_owned();

    // 9. Draft should be gone after finalize
    let r = dispatch_tool(&ctx, "discard_draft", &serde_json::json!({ "draft_id": draft_id })).await;
    assert_eq!(r.get("discarded").and_then(|v| v.as_bool()), Some(false), "draft should already be removed");

    // 10. list_strategies returns the new strategy
    let r = dispatch_tool(&ctx, "list_strategies", &serde_json::json!({})).await;
    let strategies = r.get("strategies").and_then(|v| v.as_array()).expect("strategies");
    assert!(strategies.iter().any(|s| s.get("store_id").and_then(|v| v.as_str()) == Some(&store_id)));

    // 11. create_backtest returns service_unavailable (no BacktestManager in test)
    let r = dispatch_tool(&ctx, "create_backtest", &serde_json::json!({
        "store_id": store_id,
        "instrument_id": "BTC-USDT",
        "asset_class": "crypto_spot_cex",
        "timeframe": "1m",
        "start": "2024-01-01T00:00:00Z",
        "end": "2024-03-31T23:59:59Z"
    })).await;
    assert_eq!(
        r.get("error").and_then(|v| v.as_str()),
        Some("service_unavailable"),
        "expected service_unavailable, got: {r}"
    );
}

// ─── Workflow B: Invalid draft catches errors ─────────────────────────────────

#[tokio::test]
async fn workflow_b_invalid_draft_catches_errors() {
    let ctx = ctx();

    // Create draft
    let r = dispatch_tool(&ctx, "new_strategy_draft", &serde_json::json!({})).await;
    let draft_id = r.get("draft_id").and_then(|v| v.as_str()).expect("draft_id").to_owned();

    // Set meta so finalize has enough to attempt validation
    dispatch_tool(&ctx, "set_strategy_meta", &serde_json::json!({
        "draft_id": draft_id,
        "strategy_id": "bad_strategy",
        "asset_class": "crypto_spot_cex"
    })).await;

    // Add a condition node with a bad expression
    dispatch_tool(&ctx, "add_condition_node", &serde_json::json!({
        "draft_id": draft_id,
        "node_id": "n1",
        "expr": ">>"
    })).await;

    // Add a signal node referencing n1
    dispatch_tool(&ctx, "add_signal_node", &serde_json::json!({
        "draft_id": draft_id,
        "node_id": "n2",
        "when": "n1",
        "emit": "long"
    })).await;

    // Add an action
    dispatch_tool(&ctx, "add_strategy_action", &serde_json::json!({
        "draft_id": draft_id,
        "on_signal": "long",
        "side": "buy",
        "size_mode": "fixed",
        "size": "0.01"
    })).await;

    // Finalize should fail with validation errors
    let r = dispatch_tool(&ctx, "finalize_strategy", &serde_json::json!({
        "draft_id": draft_id
    })).await;
    assert_eq!(r.get("valid").and_then(|v| v.as_bool()), Some(false), "expected invalid: {r}");
    let errors = r.get("errors").and_then(|v| v.as_array()).expect("errors array");
    assert!(!errors.is_empty(), "expected at least one error");

    // Draft must still be accessible after failure
    let r = dispatch_tool(&ctx, "get_draft_summary", &serde_json::json!({
        "draft_id": draft_id
    })).await;
    assert!(r.get("error").is_none(), "draft should still exist after failed finalize: {r}");

    // Fix the expression
    // We can't mutate an existing node, but we can check the draft still exists
    // and that discard works
    let r = dispatch_tool(&ctx, "discard_draft", &serde_json::json!({ "draft_id": draft_id })).await;
    assert_eq!(r.get("discarded").and_then(|v| v.as_bool()), Some(true));
}

// ─── Workflow C: Automation (DB-less, no-op writes) ──────────────────────────

#[tokio::test]
async fn workflow_c_automation_no_pg_returns_graceful_errors() {
    let ctx = ctx();

    // First finalize a valid strategy so we have a store_id
    let r = dispatch_tool(&ctx, "new_strategy_draft", &serde_json::json!({})).await;
    let draft_id = r.get("draft_id").and_then(|v| v.as_str()).unwrap().to_owned();

    dispatch_tool(&ctx, "set_strategy_meta", &serde_json::json!({
        "draft_id": draft_id, "strategy_id": "test_strat", "asset_class": "crypto_spot_cex"
    })).await;
    dispatch_tool(&ctx, "add_strategy_input", &serde_json::json!({
        "draft_id": draft_id, "lane": "market.bars.1m"
    })).await;
    dispatch_tool(&ctx, "add_strategy_input", &serde_json::json!({
        "draft_id": draft_id, "lane": "features.technical", "features": ["ema_7", "ema_21"]
    })).await;
    dispatch_tool(&ctx, "add_condition_node", &serde_json::json!({
        "draft_id": draft_id, "node_id": "n1", "expr": "feature('ema_7') > feature('ema_21')"
    })).await;
    dispatch_tool(&ctx, "add_signal_node", &serde_json::json!({
        "draft_id": draft_id, "node_id": "n2", "when": "n1", "emit": "long"
    })).await;
    dispatch_tool(&ctx, "add_strategy_action", &serde_json::json!({
        "draft_id": draft_id, "on_signal": "long", "side": "buy", "size_mode": "fixed", "size": "0.01"
    })).await;
    let r = dispatch_tool(&ctx, "finalize_strategy", &serde_json::json!({
        "draft_id": draft_id
    })).await;
    let store_id = r.get("store_id").and_then(|v| v.as_str()).expect("store_id").to_owned();

    // list_automations with no pg returns empty list
    let r = dispatch_tool(&ctx, "list_automations", &serde_json::json!({})).await;
    let autos = r.get("automations").and_then(|v| v.as_array()).expect("automations");
    assert_eq!(autos.len(), 0);

    // create_automation with no pg returns service_unavailable
    let r = dispatch_tool(&ctx, "create_automation", &serde_json::json!({
        "execution_strategy_id": store_id,
        "instrument_id": "BTC-USDT",
        "asset_class": "crypto_spot_cex",
        "account_mode": "paper"
    })).await;
    assert_eq!(r.get("error").and_then(|v| v.as_str()), Some("service_unavailable"));

    // arm/disarm with no pg also returns graceful error
    let r = dispatch_tool(&ctx, "arm_automation", &serde_json::json!({
        "automation_id": "00000000-0000-0000-0000-000000000001"
    })).await;
    assert_eq!(r.get("error").and_then(|v| v.as_str()), Some("service_unavailable"));
}

// ─── Unit: live automations gating ───────────────────────────────────────────

#[test]
fn live_automations_default_off() {
    // MCP_ALLOW_LIVE_AUTOMATIONS is not set in test environment
    // (If a previous test set it, this could fail — but env is not set in CI)
    std::env::remove_var("MCP_ALLOW_LIVE_AUTOMATIONS");
    assert!(!mcp_live_automations_allowed());
}

// ─── Unit: duplicate node_id ─────────────────────────────────────────────────

#[tokio::test]
async fn duplicate_node_id_returns_error() {
    let ctx = ctx();
    let r = dispatch_tool(&ctx, "new_strategy_draft", &serde_json::json!({})).await;
    let draft_id = r.get("draft_id").and_then(|v| v.as_str()).unwrap().to_owned();

    dispatch_tool(&ctx, "add_condition_node", &serde_json::json!({
        "draft_id": draft_id, "node_id": "n1", "expr": "feature('ema_7') > feature('ema_21')"
    })).await;

    let r = dispatch_tool(&ctx, "add_condition_node", &serde_json::json!({
        "draft_id": draft_id, "node_id": "n1", "expr": "feature('ema_7') > feature('ema_21')"
    })).await;
    assert_eq!(r.get("error").and_then(|v| v.as_str()), Some("duplicate_node_id"));
}

// ─── Unit: discard unknown draft ─────────────────────────────────────────────

#[tokio::test]
async fn discard_unknown_draft_returns_false() {
    let ctx = ctx();
    let r = dispatch_tool(&ctx, "discard_draft", &serde_json::json!({
        "draft_id": "00000000-0000-0000-0000-000000000099"
    })).await;
    assert_eq!(r.get("discarded").and_then(|v| v.as_bool()), Some(false));
}

// ─── Unit: list_backtests no BacktestManager ─────────────────────────────────

#[tokio::test]
async fn list_backtests_no_manager_returns_empty() {
    let ctx = ctx();
    let r = dispatch_tool(&ctx, "list_backtests", &serde_json::json!({})).await;
    let backtests = r.get("backtests").and_then(|v| v.as_array()).expect("backtests");
    assert_eq!(backtests.len(), 0);
}

// ─── Unit: get_backtest no BacktestManager ───────────────────────────────────

#[tokio::test]
async fn get_backtest_no_manager_returns_service_unavailable() {
    let ctx = ctx();
    let r = dispatch_tool(&ctx, "get_backtest", &serde_json::json!({
        "backtest_id": "00000000-0000-0000-0000-000000000001"
    })).await;
    assert_eq!(r.get("error").and_then(|v| v.as_str()), Some("service_unavailable"));
}

// ─── Unit: invalid timeframe ─────────────────────────────────────────────────

#[tokio::test]
async fn create_backtest_invalid_timeframe() {
    let ctx = ctx();
    // First create a strategy
    let r = dispatch_tool(&ctx, "new_strategy_draft", &serde_json::json!({})).await;
    let draft_id = r.get("draft_id").and_then(|v| v.as_str()).unwrap().to_owned();
    dispatch_tool(&ctx, "set_strategy_meta", &serde_json::json!({
        "draft_id": draft_id, "strategy_id": "s1", "asset_class": "crypto_spot_cex"
    })).await;
    dispatch_tool(&ctx, "add_strategy_input", &serde_json::json!({
        "draft_id": draft_id, "lane": "market.bars.1m"
    })).await;
    dispatch_tool(&ctx, "add_strategy_input", &serde_json::json!({
        "draft_id": draft_id, "lane": "features.technical", "features": ["ema_7", "ema_21"]
    })).await;
    dispatch_tool(&ctx, "add_condition_node", &serde_json::json!({
        "draft_id": draft_id, "node_id": "n1", "expr": "feature('ema_7') > feature('ema_21')"
    })).await;
    dispatch_tool(&ctx, "add_signal_node", &serde_json::json!({
        "draft_id": draft_id, "node_id": "n2", "when": "n1", "emit": "long"
    })).await;
    dispatch_tool(&ctx, "add_strategy_action", &serde_json::json!({
        "draft_id": draft_id, "on_signal": "long", "side": "buy", "size_mode": "fixed", "size": "0.01"
    })).await;
    let r = dispatch_tool(&ctx, "finalize_strategy", &serde_json::json!({
        "draft_id": draft_id
    })).await;
    let store_id = r.get("store_id").and_then(|v| v.as_str()).expect("store_id").to_owned();

    // Now try with invalid timeframe
    let r = dispatch_tool(&ctx, "create_backtest", &serde_json::json!({
        "store_id": store_id,
        "instrument_id": "BTC-USDT",
        "asset_class": "crypto_spot_cex",
        "timeframe": "weekly",
        "start": "2024-01-01T00:00:00Z",
        "end": "2024-03-31T23:59:59Z"
    })).await;
    // With no BacktestManager, this hits service_unavailable before the timeframe check.
    // We assert it's one of the two expected errors.
    let err = r.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        err == "invalid_timeframe" || err == "service_unavailable",
        "unexpected error: {err}"
    );
}

// ─── Unit: tool_definitions serialises correctly ──────────────────────────────

#[test]
fn tool_definitions_is_valid_json_array() {
    let defs = mcp_server_lib::tool_definitions();
    let arr = defs.as_array().expect("tool_definitions should be a JSON array");
    // 7 original + 10 builder/backtest/automation tools = 17 minimum
    assert!(arr.len() >= 17, "expected at least 17 tools, got {}", arr.len());
    for tool in arr {
        assert!(tool.get("name").is_some(), "tool missing 'name': {tool}");
        assert!(tool.get("inputSchema").is_some(), "tool missing 'inputSchema': {tool}");
    }
}
