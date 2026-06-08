# 01 — Architecture

## Deployment philosophy for v1

Build as a **modular monolith with a few satellite processes**, not a microservice mesh.

- **One main binary** holds the API gateway, the UI streaming gateway, the strategy runtime, and
  the execution/risk layer. These are easiest to reason about and debug when co-located, and
  legibility (the ability to ask "what did the system believe and do, and why" and get a straight
  answer) is the most valuable property for a money-handling system run by a small team.
- **Satellite processes** are the things that fail or scale independently:
  - **Collectors** — one per venue/source. They crash and reconnect on their own rhythm and must
    not take the core down with them.
  - **Strategy runtime workers** (optional, later) — if you want strategy isolation.

Boundaries exist **in code** (separate crates) even when deployed as fewer processes. Extract a
crate into its own process only when a specific, measured pressure forces it.

## The planes

| Plane | Purpose | Transport |
|-------|---------|-----------|
| Control | Start/stop strategies, config, user actions, history | REST (Axum) |
| UI live | Live visualization for React | WebSocket / SSE |
| Data | Internal normalized events between services | NATS JetStream (v1) |
| Storage | Durable historical record | ClickHouse + Postgres + Parquet |
| Strategy | Decision-grade event consumption | Internal bus + runtime |
| Replay | Historical simulation | Event store + replay clock |

This separation is the heart of the system.

## End-to-end shape

```
React Frontend
  ├── REST: config / actions / history
  └── WebSocket: live panel subscriptions

Main Binary
  ├── Axum REST API + auth
  ├── UI Streaming Gateway (throttled, frontend-shaped)
  ├── Strategy Runtime (live WorldState per strategy instance)
  ├── Risk Gate (every order, manual or automated, passes through)
  ├── Execution Engine (broker/exchange order handling)
  └── Demand Manager (tracks UI + strategy subscriptions)

Satellite Collectors (one process per source)
  ├── Crypto collector(s)   → normalize → publish
  └── Stock collector(s)    → normalize → publish

Event Fabric (NATS JetStream)
  ├── Typed lanes, partitioned by instrument/venue
  ├── Durable + replayable where needed
  └── quarantine lane for schema failures

Consumers
  ├── UI Gateway
  ├── Strategy Runtime
  ├── Storage Writers
  ├── Feature Engine
  └── Backtest Recorder

Storage
  ├── Postgres   — transactional/core (users, orders, fills, positions, strategy defs)
  ├── ClickHouse — time-series analytics (bars, trades, features)
  ├── Parquet/object store — raw event archive (ground truth)
  └── Redis/Valkey — latest-state cache

Backtesting
  ├── Loads recorded raw events
  ├── Replays by available_time through the SAME builders/runtime
  ├── Simulates execution/fills
  └── Produces performance/risk reports
```

## The Demand Manager

Strategies and UI panels do **not** start engines directly. They **declare demand**:

```json
{
  "consumer_id": "strategy_instance_123",
  "consumer_type": "strategy_runtime",
  "needs": [
    { "lane": "market.bars.1m", "instrument": "BTC-USDT" },
    { "lane": "features.technical", "instrument": "BTC-USDT" }
  ]
}
```

The Demand Manager aggregates demand across all consumers and starts / keeps alive / downshifts /
stops the underlying collectors and feature pipelines:

```
BTC-USDT market.bars.1m       needed by 42 consumers
AAPL     market.orderbook.l2  needed by 7 consumers
```

Rule: if at least one consumer needs a lane, keep its pipeline active. If none do, stop it,
pause it, or drop it to low-frequency/archival mode. This prevents every strategy from spinning
up duplicate streams.

## Backpressure (non-negotiable)

A data-heavy system must assume consumers fall behind. The system uses:

- **Bounded queues** everywhere — never unbounded in-memory growth (a top cause of dead
  real-time backends).
- **Consumer lag metrics** and **queue-depth metrics**.
- **Drop/throttle policies for UI-only lanes**; **never-drop policies for orders/fills**.
- **Retry / dead-letter** for failed processing.
- **Batch writes** for storage.
- **Partitioning** by instrument/venue/lane/date.

## Failure-mode posture per component

Define, per component, whether its failure **halts trading**, **degrades gracefully**, or is
**ignored**:

| Component down | Effect |
|----------------|--------|
| Storage writer behind | Degrade — keep trading; backfill later. Must NOT stop execution. |
| Feature engine crashed | Degrade — strategies depending on those features pause; others continue. |
| Position cache wrong | **Halt** new orders on affected instruments until reconciled. |
| Market data stale (feed quiet but connected) | **Halt** trading on that instrument; this is detected explicitly, not assumed from a closed connection. |
| Broker disconnected | **Halt** new orders; run reconciliation on reconnect before resuming. |
| Bus (JetStream) down | **Halt** — this is the spine; nothing trades blind. |

The stock-vs-crypto wrinkle: "feed went quiet" can mean *market closed/halted* (normal for
stocks) or *we broke* (always bad). The freshness watchdog must read the instrument metadata
(trading hours, halt state) so it does not false-alarm at 4pm when the stock market closes
normally. See [02-data-model.md](./02-data-model.md) and [03-data-engineering.md](./03-data-engineering.md).
