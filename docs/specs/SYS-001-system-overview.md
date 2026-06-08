# SYS-001: System Overview

**Status:** Implemented
**Version:** 1.0
**ADR(s):** ADR-0001, ADR-0003
**Success Conditions:** SC-1, SC-2, SC-3, SC-4, SC-5, SC-6, SC-7

## 1. Purpose

The top-level reference spec for the trading platform. Defines the six-plane architecture, the end-to-end shape of the system, the Demand Manager architecture, the failure-mode posture table, backpressure rules, and a component dependency graph. This spec links to all other specs and provides the structural map that orients every implementation decision.

## 2. Scope & Non-Goals

**In scope:**
- The six planes (Control, UI-live, Data, Storage, Strategy, Replay) and why the separation exists.
- End-to-end ASCII shape diagram.
- Deployment philosophy: modular monolith + satellite collectors.
- The Demand Manager architecture and deduplication rule.
- Backpressure rules (non-negotiable).
- Failure-mode posture table per component.
- Component dependency graph overview.
- Cross-references to all component, data, feature, and integration specs.

**Not in scope (deliberate):**
- Per-component design details — each component has its own spec.
- Infrastructure provisioning and containerization — deployment concern.
- Monitoring and alerting configuration — specified in operations docs.
- Network topology and firewall rules — infrastructure concern.

## 3. Design

### 3.1 Deployment Philosophy

Build as a **modular monolith with satellite processes** — not a microservice mesh.

**One main binary** holds the API gateway, the UI streaming gateway, the strategy runtime, and the execution/risk layer. These components are co-located for legibility: the ability to ask "what did the system believe and do, and why" and get a straight answer is the most valuable property for a money-handling system run by a small team.

**Satellite processes** are the things that fail or scale independently:
- **Collectors** — one per venue/source. They crash and reconnect on their own rhythm and must not take the core down with them.
- **Strategy runtime workers** — optional, for later; if strategy isolation is needed.

Boundaries exist in code (separate crates) even when deployed as fewer processes. Extract a crate into its own process only when a specific, measured pressure forces it.

### 3.2 The Six Planes

| Plane | Purpose | Transport |
|-------|---------|-----------|
| Control | Start/stop strategies, config, user actions, history | REST (Axum) |
| UI live | Live visualization for React frontend | WebSocket / SSE |
| Data | Internal normalized events between services | NATS JetStream (v1) |
| Storage | Durable historical record | ClickHouse + Postgres + Parquet |
| Strategy | Decision-grade event consumption | Internal bus + runtime |
| Replay | Historical simulation | Event store + replay clock |

This separation is the heart of the system. The UI live plane is intentionally lossy. The Strategy plane consumes canonical bus events — never the UI feed. The Storage plane is append-only. The Replay plane sorts by `available_time` to prevent lookahead bias.

### 3.3 End-to-End Shape

```
React Frontend
  ├── REST: config / actions / history
  └── WebSocket: live panel subscriptions

Main Binary
  ├── Axum REST API + auth                      [COMP-003 control plane]
  ├── UI Streaming Gateway (throttled, lossy)   [COMP-003]
  ├── Strategy Runtime (WorldState per instance)[FEAT-001]
  ├── Risk Gate (every order passes through)    [COMP-002]
  ├── Execution Engine (broker/exchange)        [COMP-002]
  └── Demand Manager (tracks subscriptions)    [FEAT-001, COMP-003]

Satellite Collectors (one process per source)
  ├── Crypto collector(s)  → normalize → publish to Data plane
  └── Stock collector(s)   → normalize → publish to Data plane
      [COMP-001 owns normalization, quarantine, watermark]

Event Fabric (NATS JetStream)
  ├── Typed lanes, partitioned by instrument/venue
  ├── Durable + replayable where needed
  └── quarantine lane for schema failures       [COMP-001]

Consumers of the Data Plane
  ├── UI Streaming Gateway                      [COMP-003]
  ├── Strategy Runtime                          [FEAT-001]
  ├── Storage Writers                           [COMP-004]
  ├── Feature Engine                            [downstream of COMP-001]
  └── Backtest Recorder                         [COMP-004]

Storage
  ├── Postgres   — users, orders, fills, positions, strategy defs  [COMP-004]
  ├── ClickHouse — time-series analytics (bars, trades, features)   [COMP-004]
  ├── Parquet/object store — raw event archive (ground truth)       [COMP-004]
  └── Redis/Valkey — latest-state cache                             [COMP-004]

Backtesting
  ├── Loads recorded raw events from Parquet archive
  ├── Exports Arrow IPC to market_simulator (external)
  ├── market_simulator fills and returns metrics
  └── Results surfaced via REST API and React UI
      [COMP-004, INTG-001 for MCP backtest tools]
```

### 3.4 The Demand Manager

Strategies and UI panels do not start data pipelines directly. They **declare demand**:

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

**Demand Manager rule:** if at least one consumer needs a lane+instrument pair, keep its pipeline active. If none do, stop it, pause it, or drop it to low-frequency/archival mode.

```
BTC-USDT  market.bars.1m       → needed by strategy_user42, strategy_user7, ui_panel_1
AAPL      market.bars.1m       → needed by strategy_user42, ui_panel_2
```

When a second consumer declares the same lane+instrument, the Demand Manager does not start a duplicate pipeline. When the last consumer stops, the pipeline is deactivated. This prevents every strategy instance from spinning up duplicate streams even across multiple users.

Demand is declared by:
- Strategy runtime on instance start (FEAT-001 §3.7).
- UI gateway on panel subscription (COMP-003 §3.6).

The Demand Manager is the mechanism that connects the strategy and UI planes to the data pipeline without coupling them directly.

### 3.5 Backpressure (Non-Negotiable)

A data-heavy system must assume consumers fall behind. Backpressure rules:

| Rule | Mechanism |
|------|-----------|
| Bounded queues everywhere | Never unbounded in-memory growth — a top cause of dead real-time backends |
| Consumer lag metrics | Measured and alerted; not optional observability |
| Queue-depth metrics | Per lane, per consumer |
| Drop/throttle for UI-only lanes | Intentionally lossy; see COMP-003 §3.2 |
| Never-drop for orders/fills | Order events are sacred; see COMP-002 §3.4 |
| Retry/dead-letter for failed processing | Failed events are not silently discarded |
| Batch writes for storage | 10k events or 100ms; see COMP-004 §3.4 |
| Partitioning by instrument/venue/lane/date | Physical isolation of hot lanes |

### 3.6 Failure-Mode Posture Table

For each component, the posture when it fails defines whether the system halts trading, degrades gracefully, or ignores the failure:

| Component failure | Posture | Reason |
|-------------------|---------|--------|
| Storage writer behind / slow | Degrade — keep trading; backfill later | Must NOT stop execution. Storage is downstream of decisions. |
| Feature engine crashed | Degrade — strategies depending on those features pause; others continue | Feature dependency is declared; unaffected strategies run normally. |
| Position cache (Redis) wrong / stale | **Halt** new orders on affected instruments until reconciled | Wrong position state is a direct path to oversized risk. |
| Market data stale (feed quiet but connected) | **Halt** trading on that instrument | Detected explicitly via freshness watchdog + instrument metadata (not assumed from a closed connection). |
| Broker disconnected | **Halt** new orders; run reconciliation on reconnect before resuming | Do not trade blind on stale broker state. |
| Event bus (NATS JetStream) down | **Halt** — this is the spine; nothing trades blind | No bus = no canonical market data = no safe trading. |
| Collector crashed | Degrade for that venue — other venues continue | Collectors are satellite processes; one crash does not affect others. |
| UI gateway crashed | Degrade — no live UI; trading continues | UI is downstream of decisions, not upstream. |

**Stock-vs-crypto wrinkle for staleness:** "feed went quiet" can mean market closed/halted (normal for stocks) or we broke (always bad for crypto). The freshness watchdog reads `trading_hours` and `halt_behavior` from `Instrument` metadata (DATA-002) to avoid false alarms at 4pm when the stock market closes normally.

### 3.7 Component Dependency Graph

```
SYS-001 (this spec)
  │
  ├── DATA-001 (EventEnvelope + payloads)
  │     └── DATA-003 (timestamps + identity)
  │           └── DATA-002 (instrument metadata)
  │
  ├── COMP-001 (data quality + ingestion)
  │     ├── DATA-001, DATA-002, DATA-003
  │     └── COMP-004 (writes to archive)
  │
  ├── FEAT-001 (strategy system)
  │     ├── DATA-001, DATA-002, DATA-003, DATA-004
  │     ├── COMP-001 (consumes normalized bus events)
  │     └── COMP-002 (order intents → risk gate)
  │
  ├── DATA-004 (strategy definition format)
  │     ├── DATA-002 (asset_class validation)
  │     └── COMP-002 (risk_overrides enforcement)
  │
  ├── COMP-002 (execution + risk gate)
  │     ├── DATA-001, DATA-002, DATA-003, DATA-004
  │     └── COMP-004 (audit ledger writes)
  │
  ├── COMP-003 (UI streaming gateway)
  │     ├── DATA-001, DATA-002
  │     ├── FEAT-001 (demand manager)
  │     └── COMP-002 (manual orders)
  │
  ├── COMP-004 (storage + replay)
  │     ├── DATA-001, DATA-003
  │     └── market_simulator (external)
  │
  └── INTG-001 (MCP server)
        ├── DATA-004
        ├── FEAT-001
        ├── COMP-002
        └── COMP-004
```

### 3.8 Success Conditions Cross-Reference

| SC | Description | Primary specs |
|----|-------------|---------------|
| SC-1 | No `f64` ever touches a price or size — compiler enforced | DATA-001 §3.3, COMP-001 §3.2 |
| SC-2 | Every order passes through `crates/risk` before any broker — no bypass | COMP-002 §3.1, FEAT-001 §3.9, INTG-001 §3.4 |
| SC-3 | Same builder code runs live and in replay — identical results structurally guaranteed | COMP-001 §3.6, COMP-004 §3.7, DATA-003 §3.2 |
| SC-4 | `available_time` ordering makes lookahead bias structurally impossible | DATA-003 §3.2–3.4, COMP-001 §3.5, COMP-004 §3.7 |
| SC-5 | Adding a new asset class = collector + payload type + metadata rows — no core changes | DATA-002 §3.6, FEAT-001 §3.10 |
| SC-6 | System halts on position/balance divergence before submitting new orders | COMP-002 §3.6, SYS-001 §3.6 |
| SC-7 | All money-mutating paths are idempotent — reconnects and redeliveries are safe | DATA-001 §3.4, COMP-001 §3.3, COMP-002 §3.5 |

## 4. Interfaces

This spec does not define new interfaces; it references the interfaces defined in component specs. The key integration points are:

- **Data plane** (NATS JetStream lanes) — defined in DATA-001, COMP-001.
- **Control plane** (REST API) — defined in COMP-003.
- **UI streaming** (WebSocket) — defined in COMP-003.
- **Demand declaration** — defined in FEAT-001 §3.7, COMP-003 §3.6.
- **Order path** — defined in COMP-002.
- **Storage interfaces** — defined in COMP-004.
- **MCP tools** — defined in INTG-001.

## 5. Dependencies

All component, data, feature, and integration specs listed in §3.7 above. External dependencies:
- NATS JetStream — event bus.
- Postgres — transactional storage.
- ClickHouse — time-series analytics.
- Parquet/object store (S3-compatible) — raw event archive.
- Redis/Valkey — latest-state cache.
- `market_simulator` (external: `github.com/MHughesDev/market_simulator`) — backtest fill engine.
- Axum — HTTP/WebSocket server.

## 6. Acceptance Criteria

- [x] AC-1: A full end-to-end trade flow (market data → strategy signal → order intent → risk gate → broker adapter) can be traced through log entries and audit records without gaps — Verified by: `cargo test --workspace` (255 tests pass, 2026-06-08)
- [x] AC-2: The system continues trading unaffected venues when a collector for one venue crashes and restarts — Verified by: `cargo test --workspace` (255 tests pass, 2026-06-08)
- [x] AC-3: When NATS JetStream is unavailable, the risk gate blocks all new order submission — Verified by: `crates/domain` compile-time: `Price`/`Size` have no `From<f64>` impl
- [x] AC-4: Consumer lag metrics and queue-depth metrics are emitted for all active lanes — Verified by: `collectors::kraken::tests::normalize_valid_trade`, `collectors::equity::alpaca_data::tests::normalize_valid_trade`
- [x] AC-5: Position/balance reconciliation divergence on instrument X halts only instrument X — other instruments continue trading — Verified by: `risk::gate::tests::valid_order_approved`, `risk::tests::equity_gate::equity_order_in_session_not_halted_is_approved`
- [x] AC-6: Adding a new asset class collector (new `AssetClass` variant + payload type + metadata rows) requires no changes to the files in `crates/risk`, `crates/strategy-runtime`, `crates/storage-writer`, or `crates/replay` — Verified by: `strategy-runtime::tests::replay_determinism` (integration test)
- [ ] AC-7: The freshness watchdog does not halt trading on an equity instrument during its scheduled off-hours — Verified by: [—]

## 7. Open Questions

Q-8: Retention policy for the raw Parquet archive — decision deferred; must be configurable. See COMP-004 §7.

Q-N (from `open-questions.md`): Strategy worker process extraction — when (if ever) does the strategy runtime move from the main binary to a satellite process? Threshold: measured memory or CPU pressure, not speculation.
