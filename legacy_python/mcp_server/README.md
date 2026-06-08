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

The registry exposes essentially the entire control-plane surface — ~55 tools spanning trading,
asset administration, universes, system controls, charts, and APEX governance. Every tool routes
through the same FastAPI control-plane endpoints a human uses, so **RiskEngine gates and
risk-signing apply identically to the agent and to a human** — the agent is well-equipped but
cannot bypass risk. Blocked orders come back with `blocked` reason codes; mutating tools carry the
`mutating=True` flag in their `ToolSpec` so a client can gate/confirm them.

| Group | Read tools | Act (mutating) tools |
|---|---|---|
| Core | `get_system_status`, `list_watched_assets`, `get_recent_bars`, `get_positions`, `get_pnl`, `get_latest_decision` | `place_order`, `flatten_position`, `set_asset_lifecycle`, `set_execution_mode` |
| Universe | `list_alpaca_universe`, `list_coinbase_universe`, `list_platform_supported_universe`, `search_universe` | `sync_alpaca_universe`, `sync_coinbase_universe` |
| System | `get_scheduler_status`, `get_microservices_health`, `get_pnl_series`, `get_routes`, `get_params`, `get_system_mode`, `get_system_power`, `get_execution_profile`, `list_models` | `set_params`, `set_system_mode`, `system_flatten`, `set_system_power`, `set_execution_profile`, `set_model_version` |
| Asset admin | `get_asset_model_manifest`, `get_asset_lifecycle`, `get_asset_execution_mode`, `get_asset_init_job` | `put_asset_model_manifest`, `delete_asset_model_manifest` |
| Charts | `get_latest_bar`, `get_trade_markers` | — |
| Governance | `get_release_evidence`, `get_config_diff_audit`, `get_governance_monitoring`, `get_probation_status`, `get_shadow_comparison`, `get_rollback_playbook`, `list_release_objects`, `get_release_object`, `list_experiments`, `get_experiment` | `diff_release_evidence`, `run_shadow_comparison`, `create_release_object`, `evaluate_release_gates`, `create_experiment`, `delete_experiment` |

### Deliberately excluded

A few endpoints are **not** exposed as agent tools, on purpose:

- `/auth/*` (register/login/logout/me/touch) — human session/cookie management; not a fit for a
  stateless tool-calling agent.
- `/auth/venue-credentials` (read+write) — encrypts/stores third-party exchange API secrets;
  letting an agent read or rewrite venue credentials is a credential-exfiltration / lockout risk
  that isn't worth the convenience.
- `/metrics` and `/assets/chart/stream` — Prometheus plaintext and SSE streaming don't fit the
  request/response tool-call model.

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
