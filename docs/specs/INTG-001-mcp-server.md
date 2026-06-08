# INTG-001: MCP Server

**Status:** Draft
**Version:** 0.1
**ADR(s):** ADR-0010
**Success Conditions:** SC-2

## 1. Purpose

Defines the MCP (Model Context Protocol) server: a thin front door that enables an AI agent (e.g. Claude) to author, validate, and apply trading strategies by producing the same canonical strategy definition JSON that the visual builder and the JSON API produce. The MCP server adds no privileged path — everything it does routes through the same validator, the same risk gate, and the same runtime as the other two front doors. It is a translator from agent intent to canonical JSON, nothing more.

## 2. Scope & Non-Goals

**In scope:**
- MCP server as one of three strategy front doors (visual builder, JSON API, MCP server).
- Tool definitions: the initial set of MCP tools and their purposes.
- No-privilege-escalation invariant: MCP cannot do anything the JSON API cannot do.
- Canonical strategy definition JSON as the single output artifact.
- Validation requirement before `create`/`apply`.
- Risk overrides tighten-only enforcement (same as DATA-004).
- Round-trip compatibility with the visual builder (node ids and graph positions are preserved).
- Guardrails: no order placement tool, mandatory validation, trust tier respected.

**Not in scope (deliberate):**
- MCP server transport/protocol implementation details — uses the MCP SDK.
- Authentication of MCP callers — resolved at the API layer that the MCP server calls.
- Strategy execution runtime — specified in FEAT-001.
- Strategy definition format details — specified in DATA-004.
- Risk gate enforcement logic — specified in COMP-002.
- Visual builder UI — frontend concern.
- Direct order placement via MCP — explicitly excluded in v1 (see §3.3).

## 3. Design

### 3.1 Three Front Doors, One Room

```
Visual Builder (n8n-style) ─┐
JSON Strategy API ──────────┼──▶  Strategy Definition JSON  ──▶  Validator  ──▶  Runtime
MCP Server (this spec) ─────┘
```

The MCP server is a sibling to the visual builder and JSON API, not a layer above them. All three produce the same document format (DATA-004). All three route to the same validator and runtime. This is the most important constraint: if the MCP server had its own definition format or its own runtime path, there would be two formats and two runtimes to keep in sync — exactly the divergence trap.

### 3.2 Why Thin Matters

The MCP server must target the canonical definition format from DATA-004. It is a translator from agent intent → canonical JSON. Any capability it exposes that is not also available through the JSON API or visual builder is a violation of the single-room principle. The no-bypass rule (SC-2) applies: orders triggered by MCP-authored strategies still pass through the risk gate like any other order intent.

### 3.3 MCP Tools (Initial Set)

| Tool | Purpose |
|------|---------|
| `list_lanes` | Return available lanes and which instruments publish them |
| `list_instruments` | Return instruments with asset class, metadata (hours, tick, trust tier) |
| `validate_strategy` | Validate a candidate definition JSON without persisting it; returns structured errors the agent can act on |
| `create_strategy` | Persist a validated definition to the user's strategy library |
| `apply_strategy` | Start a strategy instance on an instrument (initializes the instance in the runtime) |
| `stop_strategy` | Stop a running strategy instance |
| `list_strategies` | List defined and running strategies |
| `run_backtest` | Submit a definition and time range to the backtest service |
| `get_backtest_result` | Fetch metrics, P&L, and risk report for a completed backtest |

### 3.4 Guardrails

**No order placement tool.** The MCP server defines and runs strategies; any orders those strategies emit still pass the risk gate. There is no `place_order` MCP tool in v1. An agent cannot directly submit an order — it can only define a strategy that, when initialized and triggered by market data, emits order intents through the risk gate.

**Validation is mandatory before `create`/`apply`.** A `create_strategy` call that receives an invalid definition returns a structured `ValidationError` that the agent can parse and act on (e.g. fix the offending field and retry). Malformed or risk-loosening definitions are rejected before any state is mutated.

**Risk overrides tighten only.** The MCP server cannot author a definition that loosens the global risk gate. This is enforced by the shared validator (DATA-004 §3.6), not by trust in the MCP caller. The MCP server cannot sidestep the validator.

**Trust tier respected.** Strategies authored via MCP carry a `min_trust_tier` field like any other definition. The risk gate enforces it at order-submission time.

**No privilege escalation.** An agent using the MCP server operates with the same permissions as the authenticated user. It cannot access other users' strategies, positions, or data. Authentication and authorization are resolved at the API layer the MCP tools call.

### 3.5 Round-Trip with Visual Builder

The visual builder and MCP server are siblings: both emit canonical JSON. An agent can:
1. Draft a strategy via `create_strategy` (MCP).
2. A human then opens it in the visual builder and edits the node graph.
3. The JSON round-trips because the canonical format is the single source of truth.

Node `id` fields and optional graph positions are preserved in the definition JSON. The format is expressive enough that a graph survives the round trip without losing structure.

### 3.6 Relationship to the Three-Front-Door Architecture

The three-front-door architecture is a deliberate invariant: any new capability that one front door gets must be achievable by the others (or be genuinely UI/agent-specific with no impact on the canonical format). Adding a new node type to the strategy format means:
- The validator accepts it.
- The visual builder can render it.
- The MCP server can produce it via `create_strategy`.
- No special handling is needed in any of the three entry points.

## 4. Interfaces

**MCP tool signatures (logical):**

```
list_lanes()
  → [{ lane: string, instruments: string[], description: string }]

list_instruments(asset_class?: string)
  → [{ instrument_id: string, asset_class: string, venue_id: string,
       trading_hours: TradingSchedule, tick_size: string, trust_tier: string }]

validate_strategy(definition_json: string)
  → { valid: bool, errors: ValidationError[] }

create_strategy(definition_json: string)
  → { strategy_id: string } | ValidationError[]

apply_strategy(strategy_id: string, instrument_id: string)
  → { instance_id: string } | Error

stop_strategy(instance_id: string)
  → { stopped: bool }

list_strategies()
  → [{ strategy_id: string, status: "defined"|"running", instance_id?: string, instrument_id?: string }]

run_backtest(strategy_id: string, instrument_id: string, from: string, to: string)
  → { backtest_id: string }

get_backtest_result(backtest_id: string)
  → { status: "pending"|"complete"|"failed", metrics?: BacktestMetrics }
```

**Internal routing:** Each MCP tool call translates to the equivalent REST API call against the platform's internal API. The MCP server is a thin translation layer — it does not have its own database connections or business logic.

**Strategy definition format:** see DATA-004 for the full JSON contract.

## 5. Dependencies

- DATA-004 — canonical strategy definition format that all MCP tools target.
- FEAT-001 — strategy runtime; `apply_strategy` starts an instance here.
- COMP-002 — risk gate; orders from MCP-authored strategies pass through it identically.
- COMP-004 — backtest service; `run_backtest` and `get_backtest_result` call the adapter.
- MCP SDK — protocol implementation (not owned by this repo).

## 6. Acceptance Criteria

- [ ] AC-1: A strategy definition created via `create_strategy` is identical in structure to one submitted via `POST /api/strategies` with the same JSON — the same validator accepts or rejects both — Verified by: [—]
- [ ] AC-2: An agent calling `apply_strategy` results in an order intent that passes through `RiskGate::check()` before reaching any broker adapter — Verified by: [—]
- [ ] AC-3: `validate_strategy` called with a definition containing `risk_overrides` that loosen a global limit returns a structured `ValidationError` and does not persist the definition — Verified by: [—]
- [ ] AC-4: There is no MCP tool that directly submits an order to a broker or bypasses the risk gate — Verified by: [—]
- [ ] AC-5: A definition created via `create_strategy` (MCP) can be opened in the visual builder and its node graph is structurally intact (node ids and edge references are preserved) — Verified by: [—]
- [ ] AC-6: An MCP tool call authenticated as user A cannot read or modify user B's strategies or positions — Verified by: [—]

## 7. Open Questions

None at this revision. The MCP SDK transport and authentication handshake are infrastructure concerns resolved at deployment time.
