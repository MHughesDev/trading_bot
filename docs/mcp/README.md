# Trading-Bot MCP Server

An MCP (Model Context Protocol) server that lets external AI agents — e.g. a
Claude Code session — **design trading algorithms, backtest them against real
historical data, wait out hour-long runs, read the results, and iterate**.
There is deliberately no order-placement tool: this server never trades.

Per [ADR-0022](../adr/0022-mcp-thin-client-and-internal-agent.md) it is a thin
authenticated client of the platform API — everything it creates is real,
user-scoped platform state, visible in the web UI.

## Setup

1. **Run the platform** (serves `127.0.0.1:7080` in local dev):

   ```
   just dev
   ```

2. **Mint a service token** — in the web UI: *Settings → Credentials → API
   Access → Mint token* (copy it immediately; it is shown once). Or via curl:

   ```
   curl -X POST http://127.0.0.1:7080/auth/service-tokens \
        -H "Authorization: Bearer <login token>" \
        -H "Content-Type: application/json" \
        -d '{"label":"mcp-server"}'
   ```

3. **Start the MCP server** with the token in its environment:

   ```
   PLATFORM_API_URL=http://127.0.0.1:7080
   PLATFORM_API_TOKEN=<minted token>
   just run-mcp
   ```

   It listens on `127.0.0.1:3002` (`MCP_PORT` to change).

4. **Connect Claude Code**:

   ```
   claude mcp add --transport http trading-bot http://127.0.0.1:3002/mcp
   ```

Then, in a Claude Code session: *"Design an EMA-crossover strategy for BTC-USD,
backtest it over the last 90 days, and iterate until it beats buy-and-hold."*
The agent should call `get_authoring_guide` first — the server instructions tell
it to.

## Tool catalogue

| Group | Tools |
|---|---|
| Protocol | `get_authoring_guide` |
| Discovery | `list_instruments` (bar coverage per timeframe), `list_lanes`, `list_asset_classes`, `get_instrument` |
| Market data | `get_bars` (OHLCV + summary stats, size-capped) |
| Asset onboarding | `list_initialized_assets`, `init_asset` (background seeding), `get_asset_init_job` |
| Authoring | `validate_strategy`, `create_strategy`, `get_strategy`, `list_strategies`, `list_compatible_strategies` |
| Draft builder (optional) | `new_strategy_draft`, `set_strategy_meta`, `add_strategy_input`, `add_condition_node`, `add_signal_node`, `add_strategy_action`, `set_risk_overrides`, `get_draft_summary`, `finalize_strategy`, `discard_draft` |
| Backtests | `create_backtest`, `wait_for_backtest`, `get_backtest`, `list_backtests`, `stop_backtest`, `rerun_backtest`, `delete_backtest`, `compare_backtests` |
| Portfolio (read-only) | `get_dashboard_rollup`, `get_paper_activity`, `get_trading_status`, `get_order` |
| Models (read-only) | `list_models`, `get_model` |
| Automations | `list_automations`, `create_automation`, `arm_automation`, `disarm_automation` (live mode gated by `MCP_ALLOW_LIVE_AUTOMATIONS`) |

Notes:

- **Strategies upsert by `strategy_id` slug** — iterating agents should mint
  versioned slugs (`my_strat_v2`, …).
- **Long waits**: `wait_for_backtest` blocks server-side up to 600 s per call
  and returns progress on timeout — agents chain calls for hour-long runs. Over
  streamable HTTP the server emits keep-alive/progress frames every 10 s so
  clients and proxies never see an idle connection.
- **Results**: `get_backtest` returns a size-capped summary by default; pass
  `detail: "full"` for every trade and the full equity curve.
- The Set J honest-evaluation suite (`/api/backtest/experiments`) is not
  exposed here yet — its executor is synthetic until Set K lands.

The full authoring protocol (definition format, expression grammar, supported
features, workflow) lives at
[`crates/mcp-server/docs/authoring_guide.md`](../../crates/mcp-server/docs/authoring_guide.md)
and is served verbatim by the `get_authoring_guide` tool.
