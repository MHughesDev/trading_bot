# 08 — MCP Server

## Scope

A dedicated **MCP server** for the strategy creation / builder suite. It is a **thin front door**:
it lets an agent (e.g. Claude) author and apply strategies by producing the **same strategy
definition JSON** that the visual builder and the JSON API produce. It adds **no** privileged
path — everything it does goes through the same validator, the same runtime, and (for any order
it could trigger) the same risk gate.

```
Agent (via MCP) ──▶ MCP Server ──▶ Strategy Definition JSON ──▶ Validator ──▶ Runtime
                                   (identical to the other two front doors)
```

## Why thin matters

If the MCP server had its own way to define or run strategies, you'd have two strategy formats and
two runtimes to keep in sync — exactly the divergence trap. The MCP server **must** target the
canonical definition format from [04-strategy-system.md](./04-strategy-system.md). It is a
translator from agent intent → canonical JSON, nothing more.

## Tools (initial set)

| Tool | Purpose |
|------|---------|
| `list_lanes` | Return available lanes + which instruments publish them |
| `list_instruments` | Return instruments + asset class + metadata (hours, tick, trust tier) |
| `validate_strategy` | Validate a candidate definition JSON without applying it |
| `create_strategy` | Persist a validated definition |
| `apply_strategy` | Start a strategy instance over its `asset_universe` |
| `stop_strategy` | Stop a running instance |
| `list_strategies` | List defined + running strategies |
| `run_backtest` | Submit a definition + time range to the backtest service |
| `get_backtest_result` | Fetch metrics/PnL/risk for a completed backtest |

## Guardrails

- **No order placement tool.** The MCP server defines and runs *strategies*; any order those
  strategies emit still passes the risk gate. There is no "place this order now" MCP tool in v1.
- **Validation is mandatory** before `create`/`apply`. A malformed or risk-loosening definition is
  rejected with structured errors the agent can act on.
- **Risk overrides can only tighten.** The MCP server cannot author a definition that loosens the
  global risk gate (enforced by the validator, not by trust in the caller).
- **Trust tier respected.** Strategies authored via MCP carry a `min_trust_tier` like any other.

## Relationship to the visual builder

The visual builder and MCP server are siblings, not layers: both emit canonical JSON. An agent can
draft a strategy via MCP, a human can then open it in the visual builder and edit the node graph,
and the JSON round-trips because it is the single source of truth. Keep the format expressive
enough that a graph survives the round trip (node ids, positions optional-but-preserved).
