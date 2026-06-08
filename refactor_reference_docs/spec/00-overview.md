# 00 — Overview

## Framing: data platform first, trading application second

This system is **not** a CRUD app with a database behind it. That mental model is too small.
It is a data-intensive, high-throughput, low-latency system where market data, order-book
updates, bars, indicators, news, orders, fills, and strategy events all move at **different
rates**. The backend answers ordinary requests (`GET /assets`, `POST /orders`) *and* carries
continuous flows (trades streaming, order-book deltas many times per second, bars closing,
indicators updating, strategies consuming in real time).

So the system is modeled as a set of **planes**, not as one backend:

```
REST Control Plane      — start/stop strategies, config, user actions, history
Streaming Data Plane    — internal normalized events between services
UI Streaming Gateway    — throttled, frontend-shaped live views to React
Durable Event Store     — append-only record of the world, for replay/backtest
Strategy Runtime Plane  — decision-grade event consumption
Historical Replay Plane — simulation using the same strategy code
```

REST exists, but **REST is not the heavy data path.**

## The core architectural rule

Do **not** wire producer engines directly to the UI or to strategies.

Bad (becomes unmanageable under load):

```
Engine → WebSocket → UI
Engine → Strategy
Engine → Database
```

Good:

```
Producer Engines
    │  (publish one normalized event)
    ▼
Event Fabric / Message Bus
    ├── Storage Writers      (persist)
    ├── Strategy Runtime     (consume exact events)
    ├── Feature Engines      (derive)
    ├── UI Gateway           (throttle + shape for humans)
    └── Backtest Recorder    (replay later)
```

An engine publishes **one** normalized event. Every consumer decides whether it needs it.

## The lane / channel mental model

Think of the bus as a set of parallel **lanes**, each a stream/topic with its own natural
frequency. You do not force everything onto one clock; every event carries timestamps and
ordering rules, and each consumer reads the lanes it needs.

```
┌──────────────────────── EVENT FABRIC ────────────────────────┐
│ lane: market.trades            ───────────────────────────▶   │
│ lane: market.quotes            ───────────────────────────▶   │
│ lane: market.orderbook.l2      ───────────────────────────▶   │
│ lane: market.bars.1s           ───────────────────────────▶   │
│ lane: market.bars.1m           ───────────────────────────▶   │
│ lane: features.technical       ───────────────────────────▶   │
│ lane: strategy.signals         ───────────────────────────▶   │
│ lane: orders.commands          ───────────────────────────▶   │
│ lane: orders.events            ───────────────────────────▶   │
│ lane: positions.events         ───────────────────────────▶   │
│ lane: quarantine               ───────────────────────────▶   │
└───────────────────────────────────────────────────────────────┘
```

### Why lanes make the system scalable

The runtime and UI subscribe to *lanes*, not to asset types. A stock and a crypto pair both
publish `market.orderbook.l2`; their **differences live in the payload schema version and the
instrument metadata table**, not in branching code. Adding options later means: write a
collector, add a payload type, add rows to instrument metadata. The runtime, UI, storage, and
risk gate do not move. **That is what "scalable from the start" means here — universal seams,
asset-specific implementations.**

## Money correctness is the real spec

The dangerous failures are not "the UI lags." They are:

- A strategy thinks it holds a position it doesn't.
- An order is submitted twice.
- A fill arrives and nothing updates the position.

These are **reconciliation** failures and they are where trading systems actually lose money.
The system must continuously answer: *does my internal view of positions/balances match the
broker's view?* When they diverge — and they will — the system halts new orders on the affected
instrument rather than trading on stale state. See
[05-execution-and-risk.md](./05-execution-and-risk.md).

## Lossy UI vs canonical data

- **UI streams are lossy views.** If 500 order-book updates arrive in a second, the UI gateway
  may send 20 snapshots per second. That is fine for human visualization.
- **Strategy and storage streams are canonical.** They receive exact, ordered events.

The strategy runtime must **never** consume the UI WebSocket feed. UI feed is *human-shaped*;
strategy feed is *correctness-shaped*.

## What v1 deliberately is and is not

- **Is:** Coinbase (crypto spot) + Alpaca (equities), 1-minute OHLCV data, order-book capable by
  design but bar-first for MVP, local hosting, small trusted user group, one main binary plus a
  few satellite collector processes, modular monolith with clean internal boundaries. Backtesting
  delegated to the external `market_simulator` library (this repo owns the adapter, not the engine).
- **Is not:** twelve microservices, multi-tenant isolation, every asset class simultaneously, Kafka
  by default. Those are over-built for this scope. We start boring and extract complexity only when
  a measured pressure forces it.

## Asset class expansion model

The system is designed from the start to support eleven asset classes eventually — equities, ETFs,
crypto spot (CEX and DEX/AMM), expiring futures, perpetual swaps, options, bonds, FX, NFTs, and
prediction markets. The architecture ensures this does not require redesign:

- **The runtime, risk gate, and event bus never branch on asset class.** Differences live in
  instrument metadata, payload types, and collector implementations.
- **Adding a new asset class = a new collector + a new payload type + new instrument metadata rows
  + an asset spec in `docs/`.** Nothing in the core changes.
- **The `market_simulator` external dependency expands independently** (it is designed for all
  eleven classes). As it adds engines, the adapter in this repo is extended to match.

v1 MVP proves the abstraction with two asset classes (Coinbase crypto + Alpaca equities) built
deliberately differently. Every subsequent asset class is an additive extension.
