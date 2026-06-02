# MCP server — the AI agent's action surface

The trading platform is **human-first**: people drive it through the Streamlit UI and the
FastAPI control plane. This package exposes that *same* control-plane surface as
[Model Context Protocol](https://modelcontextprotocol.io) tools so an **AI agent can act on
the platform** — read context and place/manage trades — without being wired into the trading
process. The AI models (forecaster/policy) are a separate system that merely lives in the
same repo; they never call execution directly.

## Layering

| Module | Responsibility | Needs `mcp`? |
|---|---|---|
| `registry.py` | Transport-agnostic tool registry (`ToolSpec`, `ToolRegistry`). Pure Python. | no |
| `backend.py`  | `PlatformBackend` protocol + `HttpPlatformBackend` (client of the control-plane API). | no |
| `tools.py`    | The concrete read/act tool specs the agent is equipped with. | no |
| `server.py`   | Thin MCP/stdio wrapper around the registry. | yes (optional) |

The registry/tools/backend are fully unit-tested without the `mcp` dependency or a running
server (see `tests/test_mcp_server_tools.py`).

## Tools the agent gets

**Read:** `get_system_status`, `list_watched_assets`, `get_recent_bars`, `get_positions`,
`get_pnl`, `get_latest_decision`.

**Act:** `place_order` (buy/sell, market/limit), `flatten_position`, `set_asset_lifecycle`
(initialize/start/stop), `set_execution_mode` (paper/live/default).

Every *act* tool routes through the control-plane `/trade/*` and `/assets/*` endpoints, so the
**RiskEngine gates and risk-signing apply identically to the agent and to a human** — the agent
is well-equipped but cannot bypass risk. Blocked orders come back with `blocked` reason codes.

## Running

```bash
pip install -e ".[mcp]"            # optional MCP transport dependency
export NM_MCP_PLATFORM_URL=http://127.0.0.1:8000   # the control plane
export NM_CONTROL_PLANE_API_KEY=...                # operator key for mutating tools
python -m mcp_server.server        # stdio MCP server
```

Point an MCP client (e.g. Claude Desktop / Claude Code) at `python -m mcp_server.server`.

## Decoupling & triggering

The agent does **not** poll. The platform triggers a decision when a new bar closes (see
`decision_engine/bar_event_trigger.py` and the `BAR_CLOSED` topic on the shared message bus):
the platform calls the AI, not the other way around. This MCP server is how the AI then *acts*.
