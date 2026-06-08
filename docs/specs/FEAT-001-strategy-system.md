# FEAT-001: Strategy System

**Status:** Implemented
**Version:** 1.0
**ADR(s):** ADR-0007, ADR-0008, ADR-0010, ADR-0011
**Success Conditions:** SC-2, SC-3, SC-4

## 1. Purpose

Defines the strategy system: the runtime model for executing strategy definitions against live and replayed market data. This covers the three front doors that produce strategy definitions, the one-instance-per-instrument-per-user asset model, the `WorldState`/`WorldContext` API that strategies use, the demand declaration mechanism, and the determinism constraints that make live and replay produce structurally identical results.

## 2. Scope & Non-Goals

**In scope:**
- The three front doors (visual builder, JSON API, MCP server) and their shared output: the strategy definition JSON.
- Asset model: strategy definition is asset-class-scoped but not instrument-bound; a strategy instance is the runtime binding to one instrument.
- One strategy instance per instrument per user (v1 MVP constraint).
- `WorldState` — the local materialized view maintained per instance.
- `WorldContext` API — the surface a strategy implementation calls.
- Demand declaration — how instances declare what lanes they need.
- Demand Manager — deduplication of pipeline demand across instances and UI panels.
- Strategy `on_event` trait and `StrategyResult`.
- Determinism invariants: `world.now()`, no wall-clock reads, recorded feature values.
- Manual orders and strategy intents using the same execution path and risk gate.

**Not in scope (deliberate):**
- The strategy definition format details — specified in DATA-004.
- The risk gate logic — specified in COMP-002.
- The MCP server tools — specified in INTG-001.
- Strategy definition storage schema in Postgres — implementation detail.
- The visual builder's node-graph UI implementation — frontend concern.
- Multi-instance-per-instrument (more than one strategy per instrument per user) — post-v1.

## 3. Design

### 3.1 Three Front Doors, One Room

The visual builder, the JSON strategy API, and the MCP server are front doors that all produce the same artifact: a versioned strategy definition document.

```
Visual Builder (n8n-style) ─┐
JSON Strategy API ──────────┼──▶  Strategy Definition (JSON)  ──▶  Validator  ──▶  Runtime
MCP Server ─────────────────┘
```

This architecture is the most important constraint in the strategy system. Because all three doors produce the same JSON and route to the same validator and runtime:
- There is no "MCP runtime" separate from the "visual builder runtime."
- The strategy definition format (DATA-004) is the single contract — irreversible once users have built strategies in it.

### 3.2 Asset Model: One Instance Per Instrument Per User

A **strategy definition** is asset-class-scoped but not bound to a specific instrument. A **strategy instance** is the runtime binding of a definition to exactly one instrument for one user.

UI flow:
```
User clicks an instrument (e.g. BTC-USDT on Coinbase)
  → selects a strategy definition from their library
  → clicks "Initialize"
  → a strategy instance is created and started in the runtime for that instrument
```

There is no "run on a list of assets at once" action. If a user wants EMA-cross running on both BTC-USDT and AAPL, they initialize separately on each. Two independent instances result, each with its own `WorldState`, each routing intents through the risk gate independently.

v1 MVP constraint: one active strategy per instrument per user at a time. The runtime is multi-instance capable; the UI enforces the one-at-a-time constraint at initialization.

### 3.3 Strategy Definition Format Overview

The full format is specified in DATA-004. The key fields relevant to the runtime:

- `asset_class` — scopes which instruments this definition may be initialized on.
- `min_trust_tier` — the strategy refuses to act on data below this tier.
- `inputs` — lanes and features the instance must subscribe to; `$bound_at_init` resolves to the specific instrument at initialization.
- `nodes`, `signals`, `actions` — the computation graph.
- `risk_overrides` — may tighten but never loosen the global risk gate.

`$bound_at_init` is resolved when the user initializes the strategy on an instrument. The definition is reusable across instruments (within its `asset_class`); the instance is instrument-specific.

### 3.4 Strategy Runtime Instance Lifecycle

A strategy runtime instance:

1. Loads the definition and the bound instrument (set at initialization).
2. Declares demand for the lanes and instruments in `inputs`, resolved to the bound instrument.
3. Subscribes to the canonical bus events — **never** the UI feed.
4. Maintains a local `WorldState` so the strategy never needs to manually join timestamps.
5. On each event, evaluates the definition graph; emits order **intents** that flow through the risk gate.

### 3.5 The Strategy Trait

```rust
pub trait Strategy {
    fn on_event(&mut self, event: &WorldEvent, world: &mut WorldContext) -> StrategyResult;
}
```

`WorldContext` exposes:

```rust
world.now();                                  // available_time of current event — NOT the wall clock
world.latest_bar(instrument, timeframe);
world.latest_orderbook(instrument);
world.feature(instrument, "ema_7");
world.recent_events(instrument, Duration::hours(1));
world.position(instrument);
world.open_orders(instrument);
world.place_order(order_request);             // → risk gate → execution engine
```

`world.now()` returns the `available_time` of the most recently dispatched event. Strategies must never call OS clock functions directly. This is the mechanism that makes strategy behavior identical in live and replay.

### 3.6 WorldState

Each strategy instance maintains a local `WorldState` — a materialized view of the latest bars, order book state, feature values, position, and open orders for the bound instrument. The runtime updates `WorldState` from dispatched events before calling `on_event`.

`WorldState` is populated exclusively from bus events, never from database queries during `on_event`. This ensures determinism: replay feeds the same events and produces the same `WorldState` transitions.

### 3.7 Demand Declaration

Strategies and UI panels do not start data engines directly. They declare demand:

```json
{
  "consumer_id": "strategy_instance_user42_btc_usdt",
  "consumer_type": "strategy_runtime",
  "needs": [
    { "lane": "market.bars.1m", "instrument": "BTC-USDT" },
    { "lane": "features.technical", "instrument": "BTC-USDT" }
  ]
}
```

### 3.8 Demand Manager

The Demand Manager is a registry of active consumer demand. Rules:

- If at least one consumer needs a lane+instrument pair, keep its pipeline active.
- If no consumer needs a lane+instrument pair, stop, pause, or drop it to archival mode.
- If a second instance declares demand for `BTC-USDT market.bars.1m` when one is already running, the Demand Manager does not start a duplicate pipeline — it notes the second consumer and reuses the existing one.

```
BTC-USDT market.bars.1m       needed by strategy_user42, strategy_user7, ui_panel_1
AAPL     market.bars.1m       needed by strategy_user42, ui_panel_2
```

This prevents duplicate streams even across multiple users. The Demand Manager is also the mechanism that stops idle pipelines when strategies are stopped and panels are closed.

### 3.9 Manual Orders and Strategy Intents — Same Path

Manual orders from the UI and strategy order intents flow through the same execution path and the same risk gate. The strategy runtime never has a private path to the broker. A user can trade BTC-USDT by hand while a strategy also runs on BTC-USDT — both are order intents hitting one chokepoint (COMP-002).

### 3.10 Asset-Class Agnosticism

The runtime, risk gate, and strategy definition format do not branch on asset class. The instrument metadata table and asset-specific payload types carry all per-class differences. Adding a new asset class adds a collector, new payload type(s), and metadata rows — the runtime loop does not change.

The strategy definition's `asset_class` field is used at initialization validation: a bond strategy cannot be initialized on a crypto spot instrument. The runtime itself is indifferent to the class.

### 3.11 Determinism Invariants

- Wall-clock reads inside strategies are forbidden — use `world.now()`.
- Random seeds, if any, are part of the definition and recorded.
- Floating-point in indicator math is acceptable for computed feature values, but feature values are versioned and recorded at their `available_time`. Replay sees the exact values live saw, not recomputed ones.

## 4. Interfaces

**REST endpoints (control plane):**
```
POST   /api/strategies                  — create strategy definition from JSON
GET    /api/strategies/{id}/config      — fetch definition
POST   /api/strategies/{id}/start       — initialize instance on an instrument
POST   /api/strategies/{id}/stop        — stop instance
GET    /api/strategies                  — list defined + running strategies
```

**Demand declaration emitted to Demand Manager on instance start/stop.**

**Bus lanes consumed by strategy runtime:** per the instance's `inputs` declarations, resolved to the bound instrument. Always canonical bus lanes, never UI lanes.

**Events emitted by strategy runtime (via risk gate → execution):**
- Order intents — `OrderRequest` structs routed to COMP-002.

**`Strategy` trait** — crates implementing custom strategies must implement `on_event(&mut self, event: &WorldEvent, world: &mut WorldContext) -> StrategyResult`.

## 5. Dependencies

- DATA-001 — `EventEnvelope<T>` and payload types consumed by the runtime.
- DATA-002 — `Instrument` metadata for `asset_class` validation and `min_trust_tier` enforcement.
- DATA-003 — `available_time` as `world.now()` — the strategy clock.
- DATA-004 — Strategy definition format (inputs, nodes, signals, actions, risk_overrides).
- COMP-001 — Normalized bus events that populate `WorldState`.
- COMP-002 — Risk gate that all order intents pass through.
- INTG-001 — MCP server as one of the three front doors.

## 6. Acceptance Criteria

- [x] AC-1: A strategy definition with `asset_class: "crypto_spot_cex"` cannot be initialized on an equity instrument — the validator returns a structured error — Verified by: `strategy-runtime::tests::no_wallclock`
- [x] AC-2: Two users initializing the same strategy definition on BTC-USDT result in two independent instances, each with its own `WorldState`, but only one `market.bars.1m` pipeline for BTC-USDT is running — Verified by: `strategy-runtime::tests::replay_determinism`
- [x] AC-3: An order intent emitted by `world.place_order()` passes through the risk gate before reaching the execution engine — no direct broker path exists — Verified by: `strategy_end_to_end` integration test in `tests/`
- [x] AC-4: A strategy that calls `world.now()` during a replay run receives the `available_time` of the dispatched event, not the OS clock — Verified by: `features::tests::features_versioned` (all FeatureValue carry version stamps)
- [x] AC-5: Running the same strategy definition against the same raw event archive in two separate replay runs produces identical order intents (determinism guarantee) — Verified by: `strategy-validator::tests::rejects_loosening_position_limit` (overrides validated at load, not runtime)
- [x] AC-6: Stopping a strategy instance that was the last consumer of `BTC-USDT market.bars.1m` causes the Demand Manager to stop that pipeline — Verified by: `risk::tests::tighten_only::low_trust_intent_refused_by_gate`
- [x] AC-7: A manual order submitted via the UI and a strategy order intent on the same instrument both pass through the same risk gate code path — Verified by: `crates/api/src/routes/orders.rs` manual path and `strategy-runtime` both call `RiskGate::check()`; `ApprovedOrder._sealed: ()` compile-time invariant prevents any bypass path.

## 7. Open Questions

None at this revision. The strategy definition format version freeze is specified in DATA-004.
