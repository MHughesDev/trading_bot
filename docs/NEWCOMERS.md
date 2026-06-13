# NEWCOMERS.md — Trading Platform: Complete Learning Guide

> Everything you need to understand this codebase — terminology, folder maps, data flows, and
> engineering deep-dives — is self-contained in this one file.
> Read the Glossary first, then work through the Modules in order.

---

## Table of Contents

- [Glossary](#glossary)
- [Module 1: The Big Picture](#module-1-the-big-picture)
- [Module 2: Codebase Map](#module-2-codebase-map)
- [Module 3: The Domain Layer — Core Types](#module-3-the-domain-layer--core-types)
- [Module 4: Data Ingestion — Venue to Bus](#module-4-data-ingestion--venue-to-bus)
- [Module 5: Builders and Features — Pure Computation](#module-5-builders-and-features--pure-computation)
- [Module 6: Strategy System — Definition to Signal](#module-6-strategy-system--definition-to-signal)
- [Module 7: The Risk Gate — Every Order's Checkpoint](#module-7-the-risk-gate--every-orders-checkpoint)
- [Module 8: Execution and Fills — Order to Position](#module-8-execution-and-fills--order-to-position)
- [Module 9: UI Data Flow — Subscription to Screen](#module-9-ui-data-flow--subscription-to-screen)
- [Module 10: Replay — Deterministic Simulation](#module-10-replay--deterministic-simulation)
- [Module 11: Platform Boot Sequence](#module-11-platform-boot-sequence)
- [Module 12: Adding a New Asset Class](#module-12-adding-a-new-asset-class)
- [Module 13: Critical Invariants — The Rules That Must Never Break](#module-13-critical-invariants--the-rules-that-must-never-break)

---

## Glossary

Each entry has a one-line definition and a link to the module that teaches it in depth.

| Term | One-line definition | Learn more |
|------|--------------------|-----------:|
| **Action** | Maps a named signal to an order to be placed | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **ADR** | Architecture Decision Record — an immutable document recording a major design choice | [Module 1](#module-1-the-big-picture) |
| **ApprovedOrder** | An order that passed the risk gate; the only type the broker accepts | [Module 7](#module-7-the-risk-gate--every-orders-checkpoint) |
| **AssetClass** | Broad economic category of a tradable instrument (equity, crypto, FX, etc.) | [Module 3](#module-3-the-domain-layer--core-types) |
| **available_time** | When a strategy is *allowed* to act on an event — the replay sort key | [Module 10](#module-10-replay--deterministic-simulation) |
| **Bar / BarPayload** | An OHLCV candlestick (open, high, low, close, volume) for a time period | [Module 5](#module-5-builders-and-features--pure-computation) |
| **Broker** | Trait (interface) for submitting orders to a venue (Coinbase, Alpaca, etc.) | [Module 8](#module-8-execution-and-fills--order-to-position) |
| **Builder** | Pure function that reconstructs derived data (bars, orderbook) from events | [Module 5](#module-5-builders-and-features--pure-computation) |
| **$bound_at_init** | Placeholder in strategy definitions resolved to a real instrument at runtime | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **ClickHouse** | Columnar time-series database for bars, trades, and features | [Module 2](#module-2-codebase-map) |
| **EmbedSource** | Tag on a Milvus vector indicating which domain produced it (`social.post`, `web.page_snapshot`, `strategy.description`) | [Module 2](#module-2-codebase-map) |
| **Collector** | Satellite process that connects to a venue, normalizes data, and publishes to the bus | [Module 4](#module-4-data-ingestion--venue-to-bus) |
| **Condition** | Boolean predicate over market state that a strategy evaluates on each event | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **DemandRegistry** | Ref-counted tracker of who needs what data lanes — starts/stops pipelines automatically | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **EventBus** | The internal messaging system (NATS JetStream) that carries all events between services | [Module 4](#module-4-data-ingestion--venue-to-bus) |
| **EventEnvelope** | Universal wrapper around every event: timestamps, lane, instrument, payload | [Module 3](#module-3-the-domain-layer--core-types) |
| **FeatureValue** | A computed indicator (EMA, RSI) carrying its version and availability time | [Module 5](#module-5-builders-and-features--pure-computation) |
| **Fill** | A confirmed trade execution against an order | [Module 8](#module-8-execution-and-fills--order-to-position) |
| **HaltPolicy** | Whether an instrument can be halted by an exchange (`Haltable` / `NonHaltable`) | [Module 3](#module-3-the-domain-layer--core-types) |
| **IdempotencyKey** | A deterministic UUID on every order that makes retries safe | [Module 7](#module-7-the-risk-gate--every-orders-checkpoint) |
| **InstanceManager** | Tracks all active strategy instances; deduplicates pipeline demand | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **Instrument** | A tradable asset with metadata: tick size, lot size, trading hours, trust tier, etc. | [Module 3](#module-3-the-domain-layer--core-types) |
| **JetStream** | NATS's persistent streaming feature used as the event bus | [Module 4](#module-4-data-ingestion--venue-to-bus) |
| **KillSwitch** | Global flag that immediately blocks all new orders | [Module 7](#module-7-the-risk-gate--every-orders-checkpoint) |
| **Lane** | A typed stream on the event bus (e.g., `market.bars.1m`, `features.technical`) | [Module 4](#module-4-data-ingestion--venue-to-bus) |
| **MCP Server** | AI agent front door — 7 tools for strategy authoring, zero broker bypass | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **Milvus** | Vector database storing text embeddings for semantic search over social/web/strategy content | [Module 2](#module-2-codebase-map) |
| **Modular monolith** | One main binary + satellite collectors; loose coupling via event bus | [Module 1](#module-1-the-big-picture) |
| **NATS** | The messaging system underlying the event bus | [Module 4](#module-4-data-ingestion--venue-to-bus) |
| **NodeKind** | The type of a computation node in a strategy graph (Condition or Signal) | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **Normalizer** | Code that converts raw venue bytes into typed `EventEnvelope`s | [Module 4](#module-4-data-ingestion--venue-to-bus) |
| **OrderIntent** | An intent to trade *before* it goes through the risk gate | [Module 7](#module-7-the-risk-gate--every-orders-checkpoint) |
| **OrderSpec** | The order specification embedded in a strategy Action (side, size, type) | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **Panel** | A frontend visualization component (chart, order book, positions) | [Module 9](#module-9-ui-data-flow--subscription-to-screen) |
| **Parquet** | Columnar file format for raw event archiving — the immutable ground truth | [Module 2](#module-2-codebase-map) |
| **Payload** | The typed content inside an EventEnvelope (Trade, Quote, Bar, OrderBook) | [Module 3](#module-3-the-domain-layer--core-types) |
| **PipelineFactory** | Interface for starting/stopping data pipelines on demand | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **Position** | Current signed quantity held (positive = long, negative = short) | [Module 8](#module-8-execution-and-fills--order-to-position) |
| **Quarantine** | Failsafe lane for events that fail schema validation — stored for later replay | [Module 4](#module-4-data-ingestion--venue-to-bus) |
| **RegistrySnapshot** | A point-in-time snapshot of asset classes, instruments, venues, and data types used to populate the TigerGraph capability graph | [Module 2](#module-2-codebase-map) |
| **Reconciliation** | Continuous checking that internal state matches broker state | [Module 8](#module-8-execution-and-fills--order-to-position) |
| **ReplayClock** | Simulated clock advanced by the replay engine — never reads wall time | [Module 10](#module-10-replay--deterministic-simulation) |
| **RiskGate** | The single synchronous chokepoint every order must pass through | [Module 7](#module-7-the-risk-gate--every-orders-checkpoint) |
| **RiskOverrides** | Per-strategy limits that can only *tighten* global limits, never loosen them | [Module 7](#module-7-the-risk-gate--every-orders-checkpoint) |
| **Satellite binary** | A standalone process (collector) that crashes and reconnects independently | [Module 4](#module-4-data-ingestion--venue-to-bus) |
| **ShapingLayer** | UI gateway's intentionally-lossy, throttled view — strategies never see this | [Module 9](#module-9-ui-data-flow--subscription-to-screen) |
| **Signal** | A named event emitted by a strategy when a condition is true | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **Six planes** | The six architectural layers: Control, UI Live, Data, Storage, Strategy, Replay | [Module 1](#module-1-the-big-picture) |
| **SizeMode** | How an order quantity is computed (Fixed, PercentOfBalance, RiskUnit) | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **StorageWriter** | Batched consumer that writes to Postgres, ClickHouse, Parquet, and Redis | [Module 4](#module-4-data-ingestion--venue-to-bus) |
| **StrategyDefinition** | The frozen v1.0 JSON contract describing a strategy — template, not running code | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **StrategyInstance** | A running binding of a StrategyDefinition to one instrument for one user | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **StrategyClock** | Trait that abstracts time so strategies work identically live and in replay | [Module 10](#module-10-replay--deterministic-simulation) |
| **StrategyStore** | Persistent store of user-defined strategy definitions | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **Timeframe** | Bar interval enum (1s, 1m, 5m, 15m, 1h, 4h, 1d) | [Module 5](#module-5-builders-and-features--pure-computation) |
| **TigerGraph** | Graph database storing the capability/compatibility graph — which venues support which instruments/asset classes | [Module 2](#module-2-codebase-map) |
| **TradingSchedule** | When an instrument trades — sessions, timezone, pre/post market flags | [Module 3](#module-3-the-domain-layer--core-types) |
| **TrustTier** | Ordered enum for data trustworthiness — strategies declare a minimum required tier | [Module 3](#module-3-the-domain-layer--core-types) |
| **ValidatedDefinition** | A strategy definition that passed all validator checks and may be executed | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **VenueRouter** | Resolves (AssetClass) → VenueId at runtime and manages collector lifecycle | [Module 4](#module-4-data-ingestion--venue-to-bus) |
| **WallClock** | Real wall-clock time for live execution — never used in strategy evaluation | [Module 10](#module-10-replay--deterministic-simulation) |
| **WorldContext** | The read-only view a strategy calls during event processing (`now()`, `feature()`, `bar()`) | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **WorldEvent** | A typed event delivered to a strategy instance (Bar, Feature, PositionUpdate) | [Module 6](#module-6-strategy-system--definition-to-signal) |
| **WorldState** | A strategy instance's live local snapshot of bars, features, and position | [Module 6](#module-6-strategy-system--definition-to-signal) |

---

## Module 1: The Big Picture

### What is this system?

This is an **all-in-one, data & asset scalable trading platform**. That phrase means two things:

1. **Data scalable** — the platform can consume data from any venue, any protocol, any asset class by adding a new Collector. The core runtime never changes.
2. **Asset scalable** — adding a new market type (equities, options, DEX pools, prediction markets) requires adding metadata rows and a new Collector/Broker implementation. Zero changes to the risk gate, strategy runtime, storage, or replay engine.

The system is written in Rust. It handles the full lifecycle: ingesting market data → computing indicators → evaluating strategy logic → passing orders through a risk gate → executing at a broker → recording every event immutably.

### The Modular Monolith

The system is a **modular monolith** — one main deployable binary (`platform`) plus separate **satellite** collector processes. This is a deliberate middle ground:

- Not microservices: no inter-service RPC overhead, no distributed consensus to manage
- Not a traditional monolith: collectors fail and reconnect independently without taking the core down; each is its own process

The satellite binaries are `apps/collector-crypto` (Kraken), `apps/collector-equity` (Alpaca), `apps/collector-web` (web scraper), and `apps/embedder` (Milvus embedding pipeline). They connect to venues or external services, normalize data, and publish to the event bus. If a satellite crashes, it restarts. The main binary doesn't care.

### The Six Planes

The system architecture is divided into six planes — six separate concerns that can be reasoned about independently:

| Plane | What it does | How it moves data |
|-------|-------------|-------------------|
| **Control** | Start/stop strategies, manage config, user actions, history | REST (HTTP) |
| **UI Live** | Live visualization for the React frontend | WebSocket / SSE |
| **Data** | Internal normalized events between services | NATS JetStream |
| **Storage** | Durable historical record | ClickHouse + Postgres + Parquet + Redis |
| **Strategy** | Decision-grade event consumption and order generation | Internal bus + runtime |
| **Replay** | Deterministic historical replay invariants (`available_time` ordering) | Parquet event archive |

These planes don't call each other directly. They communicate through the event bus or REST. This separation means you can test the risk gate without a live market feed, and replay without a real broker.

### Architecture Decision Records (ADRs)

The `docs/adr/` directory contains 11 immutable ADRs. These are the *why* behind every major decision. If you're confused about why something works a certain way, check the ADRs first. Key ones:

- **ADR-0002** — Why `Decimal` instead of `f64` for prices (floating point rounding errors in money are unacceptable)
- **ADR-0005** — Why there's a single risk gate with no bypass path
- **ADR-0008** — Why `available_time` is the replay sort key (prevents lookahead bias)
- **ADR-0011** — Why pipelines start on-demand, never at boot

---

## Module 2: Codebase Map

### Top-level folders

```
trading_bot/
├── apps/               ← Thin binary wiring (no business logic)
├── crates/             ← All library code (business logic lives here)
├── docs/               ← Architecture, ADRs, specs, procedures
├── frontend/           ← React SPA (TypeScript)
├── config/             ← TOML configuration files
├── migrations/         ← Postgres schema files (sqlx)
├── clickhouse/         ← ClickHouse DDL
├── xtask/              ← Custom cargo build tasks
├── Cargo.toml          ← Workspace root (all crates listed here)
├── docker-compose.yml  ← Infrastructure (NATS, Postgres, ClickHouse, Redis)
└── README.md           ← Platform overview
```

### `apps/` — Binaries (thin wiring only)

Binaries contain no business logic. They wire together crates and start the server.

| Binary | Purpose |
|--------|---------|
| `apps/platform/` | Main binary — REST API + WebSocket server |
| `apps/collector-crypto/` | Satellite: Kraken crypto data collector |
| `apps/collector-equity/` | Satellite: Alpaca equity data collector |
| `apps/collector-web/` | Satellite: robots.txt-compliant web page scraper |
| `apps/embedder/` | Satellite: embedding pipeline — subscribes to social/web events, upserts into Milvus |
| `apps/mcp-server/` | MCP server binary (AI agent interface) |

### `crates/` — Library crates (all logic lives here)

Think of crates like packages. Each crate has one job. Here they are in dependency order (leaf crates first):

#### Foundation (no dependencies on other crates in this repo)

| Crate | Purpose |
|-------|---------|
| `crates/domain/` | **THE CORE.** All shared types: EventEnvelope, OrderIntent, Instrument, Payload types, money newtypes, lanes, timestamps, strategy definition format. Every other crate depends on this. |
| `crates/config/` | Typed configuration loading from TOML + env vars. Fail-fast on startup if config is invalid. |
| `crates/observability/` | Structured logging and tracing setup. JSON logs or human-readable, depending on environment. |

#### Data Pipeline

| Crate | Purpose |
|-------|---------|
| `crates/event-bus/` | NATS JetStream wrappers. The **only** crate that imports `async_nats`. Exposes `Publisher`, `Subscriber`, `QuarantinePublisher`. |
| `crates/collectors/` | Venue connectors: Kraken (crypto) and Alpaca (equity). Implements the `Collector` trait. Normalizes raw WS frames into `EventEnvelope<T>`. |
| `crates/builders/` | **Pure** bar and order-book reconstruction from events. **Zero I/O** — same code runs live and in replay. |
| `crates/features/` | **Pure** technical indicators (EMA, RSI). **Zero I/O**. Same purity contract as builders. |
| `crates/storage/` | Multi-backend write: Postgres (orders/fills), ClickHouse (bars/trades), Parquet (raw archive), Redis (latest-state cache). |

#### Trading Core

| Crate | Purpose |
|-------|---------|
| `crates/risk/` | The single risk gate every order passes through. Also: kill switch, rate limits, position checks, trust tier validation. |
| `crates/execution/` | Order submission, fill processing, position tracking, broker adapters (Coinbase, Alpaca). |
| `crates/reconciliation/` | Continuous checking that internal positions match broker state. Halts on divergence. |

#### Strategy System

| Crate | Purpose |
|-------|---------|
| `crates/strategy-validator/` | Validates strategy definitions against the frozen v1.0 format. All errors collected before returning. |
| `crates/strategy-runtime/` | Interprets strategy definitions against live/replayed events. Maintains WorldState per instance. |
| `crates/demand-manager/` | Ref-counted pipeline demand tracking. Starts pipelines when needed; stops them when no consumer remains. |
| `crates/venue-router/` | Maps AssetClass → VenueId at runtime. Manages collector lifecycle on-demand. |

#### Knowledge Layer (Phase 7)

| Crate | Purpose |
|-------|---------|
| `crates/graph/` | TigerGraph capability/compatibility graph — vertex/edge schema, idempotent DDL init, population from registry snapshots. Maps instruments ↔ venues ↔ asset classes ↔ strategy definitions. |
| `crates/semantic/` | Milvus vector store wrappers — collection management, OpenAI embedding calls (`text-embedding-3-small`, 1536 dims), upsert, and metadata-filtered similarity search. |

#### Interfaces

| Crate | Purpose |
|-------|---------|
| `crates/api/` | REST API (Axum) + WebSocket gateway for the frontend. |
| `crates/ui-gateway/` | Intentionally-lossy live view for the frontend — throttled, sampled, aggregated. |
| `crates/mcp-server/` | 9 Model Context Protocol tools for AI-agent strategy authoring. |

### `docs/` — Engineering documentation

```
docs/
├── architecture.md         ← Current-state system map (read this after this guide)
├── adr/                    ← 11 immutable Architecture Decision Records
├── specs/                  ← 12 component specs (all marked Implemented)
├── plans/                  ← 10-phase Python→Rust refactor plan (complete)
├── procedures/             ← Step-by-step operational instructions (add-a-venue, etc.)
└── research/               ← Technology evaluation notes
```

### `frontend/` — React SPA

```
frontend/src/
├── App.tsx                 ← Routes, providers
├── api/                    ← Typed REST + WebSocket clients
├── pages/                  ← Dashboard, InstrumentDetail, StrategyBuilder
├── panels/                 ← ChartPanel, OrderBookPanel, PositionsPanel
├── builder/                ← Visual node-graph strategy builder
├── store/                  ← Client state (subscriptions, auth, selected instrument)
└── types/                  ← TypeScript interfaces
```

### `migrations/` — Postgres schema

```
migrations/
├── 0001_users_accounts.sql         ← Users, accounts, API keys
├── 0002_instruments.sql            ← Instrument metadata table
├── 0003_orders_fills_positions.sql ← Orders, fills, positions, audit ledger
├── 0004_strategies.sql             ← Strategy definitions and instances
└── 0005_risk_config.sql            ← Global risk config, kill switch state
```

---

## Module 3: The Domain Layer — Core Types

The `crates/domain/` crate is the foundation everything else depends on. It contains zero I/O — no database calls, no network, no clock reads. Just types, newtypes, and pure transformations.

### Money Safety: `Price` and `Size`

The most important rule in the entire codebase:

> **No `f64` on price or size. Ever.**

Floating-point arithmetic can produce rounding errors like `0.1 + 0.2 == 0.30000000000000004`. In financial systems this compounds into real money losses.

The solution: `Price(Decimal)` and `Size(Decimal)` newtypes. `Decimal` is an exact decimal arithmetic type. These newtypes have no `From<f64>` implementation, so the compiler refuses any code that tries to create a price from a float.

```rust
// crates/domain/src/money.rs
pub struct Price(Decimal);
pub struct Size(Decimal);
// No impl From<f64> for Price  ← compiler enforces this
```

The CI check "Money safety (no f64 on price/size)" runs `cargo xtask` to grep the codebase and fail if `f64` appears anywhere near price/size contexts.

### `EventEnvelope<T>`

Every single event in the system is wrapped in an `EventEnvelope<T>`. Think of it as the envelope on a letter — the address and metadata on the outside, the letter (payload) on the inside.

```
crates/domain/src/envelope.rs
```

Key fields:

| Field | Type | Meaning |
|-------|------|---------|
| `event_id` | UUID v5 | Deterministic from dedup key — same event always gets the same ID |
| `lane` | String | Which stream this belongs to (e.g., `"market.bars.1m"`) |
| `instrument_id` | String | Which asset (e.g., `"BTC-USDT"`, `"AAPL"`) |
| `venue_id` | String | Which venue produced this (e.g., `"kraken"`, `"alpaca"`) |
| `trust_tier` | TrustTier | How trustworthy this source is |
| `event_time` | Option<DateTime> | When the event happened at the venue |
| `observed_time` | DateTime | When this process received it |
| `ingested_time` | DateTime | When it was written to storage |
| `available_time` | DateTime | When a strategy is *allowed* to act on it (replay sort key) |
| `sequence` | u64 | Monotonically increasing per lane+instrument |
| `payload` | T | The actual data (TradePayload, BarPayload, etc.) |

The four timestamps matter for replay — see [Module 10](#module-10-replay--deterministic-simulation).

### `Instrument` — Metadata-Driven Design

`crates/domain/src/instrument.rs`

An `Instrument` is not just a ticker symbol — it carries everything the system needs to know about how to handle an asset. This is the key to the platform's asset-class scalability.

```rust
pub struct Instrument {
    pub instrument_id:   String,        // "BTC-USDT", "AAPL"
    pub asset_class:     AssetClass,    // CryptoSpotCex, Equity, ...
    pub venue_id:        String,        // "kraken", "alpaca"
    pub tick_size:       Decimal,       // minimum price increment
    pub lot_size:        Decimal,       // minimum order size increment
    pub trading_hours:   TradingSchedule,
    pub halt_behavior:   HaltPolicy,    // Haltable or NonHaltable
    pub trust_tier:      TrustTier,     // trustworthiness of this source
    pub active:          bool,
    pub watermark_secs:  u64,           // bar watermark delay in seconds
}
```

The risk gate, strategy runtime, and storage never branch on `AssetClass`. They branch on *properties* — `tick_size`, `halt_behavior`, `trading_hours`. This means adding a new asset class requires only:
1. A new `AssetClass` enum variant
2. New `Instrument` metadata rows in the database
3. Zero changes to core runtime code

### `AssetClass`

```rust
pub enum AssetClass {
    CryptoSpotCex,      // Active: Coinbase, Kraken
    Equity,             // Active: Alpaca
    Etf,                // Planned
    CryptoSpotDex,      // Planned (AMM/DEX pools)
    FuturesExpiring,    // Planned
    PerpetualSwap,      // Planned
    Option,             // Planned
    Bond,               // Planned
    Fx,                 // Planned
    Nft,                // Planned
    PredictionMarket,   // Planned
}
```

All planned variants already exist in the enum. The infrastructure to add them is described in [Module 12](#module-12-adding-a-new-asset-class).

### `TrustTier`

`crates/domain/src/trust.rs`

Data sources are not all equally reliable. `TrustTier` is an ordered enum:

```
SocialDerived      ← lowest  (sentiment signals, social feeds)
OnchainTentative              (unconfirmed on-chain data)
OnchainConfirmed              (confirmed on-chain)
CentralizedExchange           (Kraken, Coinbase, Alpaca)
Regulated          ← highest (licensed exchange data)
```

Each `EventEnvelope` is stamped with its source's trust tier. Each strategy declares a `min_trust_tier`. The risk gate refuses orders triggered by events below the declared minimum — so a strategy that requires exchange-quality data can't accidentally fire on a social media sentiment spike.

### `TradingSchedule` and `HaltPolicy`

`TradingSchedule` defines when an instrument trades: timezone, session open/close times, pre/post-market flags. Crypto uses `always_open()` — empty sessions list means 24/7. Equities define US market hours.

`HaltPolicy` has two variants:
- `Haltable` — exchanges can pause trading (equities)
- `NonHaltable` — no halt mechanism exists (crypto, DEX)

The risk gate checks both. A strategy trading AAPL won't fire orders at 3am or during a halt. A strategy trading BTC-USDT trades freely all the time.

### Payload Types

`crates/domain/src/payloads/`

Four types of market data payloads:

| Type | What it represents | Key fields |
|------|--------------------|-----------|
| `TradePayload` | A single executed trade | price, size, side, trade_id |
| `QuotePayload` | Best bid/ask (L1) | bid_price, bid_size, ask_price, ask_size |
| `OrderBookPayload` | Full L2 order book | bids: Vec<(price, size)>, asks: Vec<(price, size)>, is_snapshot |
| `BarPayload` | OHLCV candlestick | timeframe, open, high, low, close, volume, trade_count, revision |
| `WebPageSnapshotPayload` | Scraped web page content | url, title, text, word_count, fetch_method (Http/Playwright), occurred_at_ms |

The `revision` field on `BarPayload` is important: when a late trade arrives after a bar closes, a new bar event with `revision > 0` is published. The original bar is **never mutated**. This is the append-only invariant.

### Lanes

`crates/domain/src/lanes.rs`

A `Lane` is a named stream on the event bus. Think of it like a TV channel — publishers broadcast on it, subscribers tune in.

Key lane constants:

```rust
pub const MARKET_TRADES:      &str = "market.trades";
pub const MARKET_BARS_1M:     &str = "market.bars.1m";
pub const MARKET_BARS_5M:     &str = "market.bars.5m";
pub const MARKET_QUOTES:      &str = "market.quotes";
pub const MARKET_ORDERBOOK:   &str = "market.orderbook.l2";
pub const FEATURES_TECHNICAL: &str = "features.technical";
pub const ORDERS_COMMANDS:    &str = "orders.commands";
pub const ORDERS_EVENTS:      &str = "orders.events";
pub const QUARANTINE:         &str = "quarantine";
```

The actual NATS subject is `{lane}.{instrument_id}` — e.g., `market.bars.1m.BTC-USDT`. Multiple subscribers can read the same subject; NATS delivers independently.

---

## Module 4: Data Ingestion — Venue to Bus

### The Full Path

```
Venue WebSocket
  ↓  (raw bytes: JSON or binary)
Collector (satellite process)
  ↓  normalize() → Result<Vec<EventEnvelope<T>>, NormalizeError>
Normalizer
  ├─ success → Publisher → NATS lane
  └─ failure → QuarantinePublisher → quarantine lane
               (stored for replay after normalizer fix)
NATS lane
  ├─ StorageWriter (batched: Postgres + ClickHouse + Parquet + Redis)
  ├─ BarBuilder (pure: trades → OHLCV bars)
  ├─ FeatureEngine (pure: bars → indicators)
  ├─ StrategyRuntime (events → signals → order intents)
  └─ UI Gateway (lossy: throttled view for frontend)
```

### The `Collector` Trait

`crates/collectors/src/lib.rs`

Every data source implements a single trait:

```rust
#[async_trait]
pub trait Collector: Send + Sync {
    async fn run(
        &self,
        publisher:   Arc<event_bus::Publisher>,
        quarantine:  Arc<event_bus::QuarantinePublisher>,
    ) -> Result<(), CollectorError>;
}
```

Implementations: `KrakenCollector` (crypto), `AlpacaCollector` (equity). The trait is how you add a new data source — implement it, create a satellite binary, done.

### The Normalizer

`crates/collectors/src/normalizer.rs`

Each venue speaks its own protocol. The normalizer converts raw venue messages into the system's universal `EventEnvelope<T>` format. This is where **schema-on-write validation** happens:

- Parse raw bytes
- Validate required fields are present and the right type
- Compute `available_time` from the event and observed timestamps
- Assign a deterministic `event_id` (UUID v5 from the dedup key)
- On success: publish to the appropriate market lane
- On failure: publish raw bytes + error to the quarantine lane (never drop data)

The quarantine lane is a safety net. If a normalizer has a bug and misreads a field, the original raw bytes are preserved. After fixing the normalizer, quarantine messages can be replayed through the fixed code.

### NATS JetStream — The Event Bus

NATS is the messaging backbone. JetStream adds persistence: messages are durably stored and replayed to subscribers that were offline. This means:

- A strategy runtime that restarts can catch up on events it missed
- Storage writers can fall behind and catch up without dropping data
- Consumers can subscribe at different points (latest vs. beginning)

The `crates/event-bus/` crate is the only place that imports `async_nats`. Everything else uses the `Publisher`, `Subscriber`, and `QuarantinePublisher` abstractions from this crate. This isolation means changing the message broker requires only touching one crate.

### Satellite Binaries

Collectors run as separate OS processes. `apps/collector-crypto/` runs Kraken; `apps/collector-equity/` runs Alpaca. Their structure:

```
main.rs:
  1. Load config
  2. Init tracing
  3. Connect to NATS
  4. Create Collector
  5. collector.run(publisher, quarantine).await
```

If Kraken goes down, the collector crashes and restarts. The main platform binary doesn't crash. The bar builder and strategy runtime handle a gap in data gracefully (the freshness watchdog in `crates/reconciliation/src/freshness.rs` detects silence and distinguishes "market closed" from "feed broken" using the instrument's `TradingSchedule`).

### Storage: Four Backends, Four Purposes

`crates/storage/`

| Backend | What goes there | Access pattern |
|---------|----------------|----------------|
| **Postgres** | Orders, fills, positions, user data, strategies | Transactional — must be consistent |
| **ClickHouse** | Bars, trades, features | Time-series range scans — fast for historical analytics |
| **Parquet** | Raw event archive | Append-only, immutable ground truth for replay and audit |
| **Redis** | Latest state per lane+instrument | Sub-millisecond reads — snapshot-on-connect for UI |

The `StorageWriter` (`crates/storage/src/writer.rs`) batches events before writing — 10,000 events or 100ms, whichever comes first. Writing one event per insert would destroy database performance at market data rates.

---

## Module 5: Builders and Features — Pure Computation

### The Purity Contract

Builders (`crates/builders/`) and features (`crates/features/`) share a critical architectural contract:

> **Zero I/O. No database imports. No network imports. No wall-clock reads.**

This is enforced by the CI check "builders/features purity (no I/O deps)". If anyone accidentally adds a `use storage::` or `use tokio::time::Instant::now()` import to these crates, CI fails.

Why? Because the same code runs in two contexts:
1. **Live**: consume bus events → emit derived events
2. **Replay**: consume recorded events from Parquet archive → emit derived events

If the code touches I/O, it can behave differently in the two contexts. Pure functions are always identical.

### Bar Builder

`crates/builders/src/bars.rs`

The bar builder takes a stream of `TradePayload` events and aggregates them into OHLCV bars.

**How a 1-minute bar is produced:**

1. Trades arrive with their `available_time`
2. The builder groups them by 1-minute window based on `available_time`
3. A bar is not emitted immediately when the minute ends — it waits for the **watermark**
4. The watermark (2 seconds for liquid CEX instruments) is the delay after the window closes before the bar is finalized
5. After the watermark elapses, the bar is published to `market.bars.1m`

**Late data handling**: If a trade arrives after the watermark (due to network delay), a **revision event** is published to `market.bars.1m.revised` with `revision > 1`. The original bar is never modified. Both the original and the revision are stored at their true `available_time`. This is the append-only invariant.

### Feature Engine

`crates/features/`

Indicators are computed from bar events. They are pure stateful objects:

```rust
// crates/features/src/ema.rs
pub struct Ema {
    period: usize,
    value:  Option<f64>,  // Note: f64 is acceptable for indicators, not money
}

impl Ema {
    pub fn update(&mut self, price: f64) -> Option<f64> { ... }
}
```

Note: `f64` is acceptable here because indicator values are:
1. Not money — they're dimensionless signals, not order prices
2. Versioned — each `FeatureValue` carries a `feature_version` integer
3. Recorded at their `available_time` — replay fetches pre-computed values, not recomputing them

A `FeatureValue` looks like:

```rust
pub struct FeatureValue {
    pub name:            String,    // "ema_7", "rsi_14"
    pub value:           f64,
    pub feature_version: u32,       // incremented when calculation logic changes
    pub available_time:  DateTime,  // when this value became available
}
```

When a feature's calculation logic changes, `feature_version` is incremented. This prevents stale cached values from being compared to newly calculated ones.

---

## Module 6: Strategy System — Definition to Signal

### The Three Front Doors

Users can define strategies in three ways, all producing the same JSON:

1. **Visual Builder** — `frontend/src/builder/` — drag-and-drop node graph (n8n-style)
2. **REST API** — `POST /api/strategies` — submit JSON directly
3. **MCP Server** — `crates/mcp-server/` — AI agents call `create_strategy` tool

All three routes through the same validator (`crates/strategy-validator/`) and the same runtime (`crates/strategy-runtime/`). There is no privileged path.

### `StrategyDefinition` — The Frozen Format

`crates/domain/src/strategy_def/mod.rs`

The strategy definition format is frozen at v1.0. Adding new node types is only possible in v2.0+ (which doesn't exist yet). This freeze was intentional — it lets the validator and runtime be simple and correct, without version negotiation complexity.

```json
{
  "strategy_id": "ema_cross_v1",
  "definition_version": "1.0",
  "asset_class": "CryptoSpotCex",
  "min_trust_tier": "CentralizedExchange",
  "inputs": [
    { "lane": "market.bars.1m", "instrument": "$bound_at_init" },
    { "lane": "features.technical", "instrument": "$bound_at_init" }
  ],
  "nodes": [
    {
      "id": "bull_cross",
      "kind": "condition",
      "expression": "feature('ema_7') > feature('ema_21')"
    },
    {
      "id": "long_signal",
      "kind": "signal",
      "when": "bull_cross"
    }
  ],
  "actions": [
    {
      "on_signal": "long_signal",
      "kind": "place_order",
      "order": {
        "side": "buy",
        "order_type": "market",
        "size_mode": "fixed",
        "size": "0.01"
      }
    }
  ],
  "risk_overrides": {
    "max_position": "5000.00",
    "max_order_rate_per_minute": 10
  }
}
```

### `$bound_at_init`

`crates/domain/src/strategy_def/inputs.rs`

The string `"$bound_at_init"` in an `inputs[*].instrument` field is a placeholder. When a user initializes the strategy on a specific instrument (e.g., BTC-USDT), the runtime resolves this placeholder to that instrument. The definition itself stays reusable — the same definition can be initialized on ETH-USDT or AAPL independently.

### `NodeKind` — What Goes in a Strategy Graph

v1.0 supports two node types:

- **Condition**: Evaluates a boolean expression over market state. Uses the expression language (feature refs, bar refs, comparisons, arithmetic). Example: `"feature('ema_7') > feature('ema_21')"`.
- **Signal**: Emits a named signal when its `when` condition is true. Signals are consumed by Actions.

Unknown node types are rejected by the validator (fail-closed). This prevents silently ignoring unrecognized future node types.

### `Action` and `OrderSpec`

Actions connect signals to orders. v1.0 supports one action type: `place_order`. When the named signal fires, the runtime creates an `OrderIntent` from the `OrderSpec`:

```rust
pub struct OrderSpec {
    pub side:      Side,       // Buy or Sell
    pub order_type: OrderType, // Market, Limit, StopLimit
    pub size_mode: SizeMode,   // Fixed (v1), PercentOfBalance (future), RiskUnit (future)
    pub size:      Decimal,    // literal size if Fixed
    pub limit_price: Option<Decimal>, // required for Limit orders
}
```

### `RiskOverrides` — Tighten Only

`crates/domain/src/strategy_def/risk_overrides.rs`

Strategies can override global risk limits, but only to make them *tighter*. A strategy cannot say "allow me to hold a larger position than the global limit." The validator checks this at load time, not at order time.

```rust
pub struct RiskOverrides {
    pub max_position:             Option<Decimal>,  // must be <= global
    pub max_order_rate_per_minute: Option<u32>,     // must be <= global
    pub max_order_rate_per_second: Option<u32>,     // must be <= global
}
```

### `StrategyInstance` and `WorldState`

`crates/strategy-runtime/src/runtime.rs` and `world.rs`

When a user initializes a strategy on an instrument, an `InstanceManager` creates a `StrategyInstance`. Each instance maintains its own `WorldState`:

```rust
pub struct WorldState {
    pub bars:         HashMap<Timeframe, BarPayload>,    // latest bar per timeframe
    pub orderbook:    Option<OrderBookPayload>,           // latest L2 snapshot
    pub features:     HashMap<String, f64>,               // computed indicators
    pub position:     Decimal,                            // current position
    pub current_time: DateTime,                           // available_time of most recent event
}
```

`WorldState` is updated from bus events **before** the strategy's conditions are evaluated. The strategy never reads from a database during execution — all information is already in `WorldState`.

### Event Processing Flow (per instance, per event)

```
WorldEvent arrives (Bar, Feature, PositionUpdate)
  ↓
WorldState::apply_event()       — update bars/features/position/current_time
  ↓
evaluate_signals()               — evaluate all condition nodes using interpreter
  ↓  (returns: set of signal names that fired)
build_intents_for_signals()      — find matching actions, create OrderIntents
  ↓
return Vec<OrderIntent>          — sent to risk gate
```

### `DemandRegistry` — Pipeline Lifecycle

`crates/demand-manager/src/registry.rs`

The demand registry answers: "what data do we actually need right now?" It's ref-counted:

- `demand.add(&lane, "BTC-USDT")` — first caller (count 0→1): starts the pipeline
- `demand.add(&lane, "BTC-USDT")` — second caller (count 1→2): no-op, pipeline already running
- `demand.remove(&lane, "BTC-USDT")` — first removal (count 2→1): no-op
- `demand.remove(&lane, "BTC-USDT")` — last removal (count 1→0): stops the pipeline

This means: if two users both run EMA-cross on BTC-USDT, there's exactly one `market.bars.1m.BTC-USDT` pipeline. When both stop, it shuts down. Pipelines are never running unless something needs them.

### `ValidatedDefinition`

`crates/strategy-validator/src/lib.rs`

The validator runs three passes in order, collecting all errors before returning:

1. **Schema** — structural correctness (version, IDs, references)
2. **Expressions** — condition expression syntax per the frozen grammar
3. **Risk** — tighten-only invariant against global limits

`#[non_exhaustive]` on the struct means it can only be constructed inside the `strategy-validator` crate — the return of `validate()`. No other code can create a `ValidatedDefinition` directly, so only validated definitions reach the runtime.

---

## Module 7: The Risk Gate — Every Order's Checkpoint

### The No-Bypass Guarantee

The risk gate is the only path from an `OrderIntent` to a broker. This is enforced at the **type system level**, not by convention:

```rust
// crates/risk/src/gate.rs
pub struct ApprovedOrder {
    pub intent: OrderIntent,
    _sealed: (),   // Private field — cannot be constructed outside this module
}
```

The `_sealed: ()` field is private. No code outside `crates/risk/src/gate.rs` can write `ApprovedOrder { intent, _sealed: () }`. The compiler refuses it. The only way to get an `ApprovedOrder` is to call `RiskGate::check()` and have it pass every check.

Brokers only accept `ApprovedOrder`. So there is no possible code path where an order reaches a broker without going through the gate.

### The Kill Switch

`crates/risk/src/kill_switch.rs`

The kill switch is the first check — if it's active, no other check runs. All new orders are blocked.

**Automatic triggers:**
- Max daily loss exceeded
- Reconciliation finds a position divergence between internal state and broker
- Data feed stale beyond the expected window
- Broker connection lost

**Manual triggers:**
- `POST /api/trading/kill` (REST endpoint or UI button)

The kill switch does **not** force-close existing positions. It only blocks new orders. Resuming requires explicit human action: `POST /api/trading/resume`.

Kill switch state is persisted to Postgres (`global_risk_config` table). If the platform restarts, it reads the stored state — it doesn't assume "not tripped" on startup.

### All 11 Checks (in order)

`crates/risk/src/gate.rs` and `limits.rs`

| # | Check | What it validates | Who it protects |
|---|-------|------------------|----------------|
| 1 | Kill switch | Is trading globally halted? | Everything |
| 2 | Idempotency cache | Has this exact order already been decided? | Safe retries |
| 3 | Apply overrides | Strategy overrides only tighten limits | Global integrity |
| 4 | Instrument active | Is this instrument loaded and not delisted? | Bad instrument IDs |
| 5 | Trading session | Is the market open right now? | Session-bound instruments |
| 6 | Halt state | Is this instrument currently halted? | Equity halts |
| 7 | Rate limit (second) | Too many orders in the last second? | Flash crash prevention |
| 8 | Rate limit (minute) | Too many orders in the last minute? | Runaway strategies |
| 9 | Position size | Would this order exceed max position? | Overexposure |
| 10 | Price sanity | Is the limit price within X% of market? | Fat-finger errors |
| 11 | Lot size | Is the order size a valid multiple of lot_size? | Exchange rejection |
| 12 | Daily loss | Has today's loss hit the limit? | Account drain |
| 13 | Trust tier | Is the triggering event trustworthy enough? | Signal quality |

### Idempotency

Every `OrderIntent` carries an `idempotency_key` (UUID). The risk gate caches its decision:

```
First time key K → run all checks → cache decision → return result
Second time key K → return cached decision immediately (no re-evaluation)
```

Why? If NATS redelivers a message (which it does for reliability), or a retry happens after a network hiccup, the same order isn't evaluated twice. The second call returns the same result as the first — approved stays approved, rejected stays rejected.

### `GateContext`

The risk gate doesn't read from a database mid-check. The caller assembles a `GateContext` before calling `check()`:

```rust
pub struct GateContext {
    pub instrument:             Instrument,
    pub is_in_session:          bool,
    pub is_halted:              bool,
    pub current_position:       Decimal,
    pub market_price:           Option<Price>,
    pub recent_orders_per_sec:  u32,
    pub recent_orders_per_min:  u32,
    pub daily_loss_usd:         Decimal,
    pub risk_overrides:         RiskOverrides,
    pub event_trust_tier:       TrustTier,
}
```

All the data the gate needs is passed in. This makes the gate a pure function — testable in isolation, no database setup required.

---

## Module 8: Execution and Fills — Order to Position

### The `Broker` Trait

`crates/execution/src/broker.rs`

All brokers implement one interface:

```rust
#[async_trait]
pub trait Broker: Send + Sync {
    async fn submit(&self, order: &ApprovedOrder) -> Result<String, BrokerError>;
    async fn cancel(&self, broker_order_id: &str) -> Result<(), BrokerError>;
    async fn query_order(&self, id: &str) -> Result<BrokerOrderStatus, BrokerError>;
    async fn query_open_orders(&self) -> Result<Vec<BrokerOrderStatus>, BrokerError>;
    async fn query_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError>;
}
```

Current implementations:

| Broker | File | Purpose |
|--------|------|---------|
| `CoinbaseAdapter` | `coinbase.rs` | Live crypto (Coinbase Advanced Trade API) |
| `AlpacaAdapter` | `alpaca.rs` | Paper/live equity (Alpaca API) |
| `NoBroker` | `broker.rs` | Fallback when no credentials are set |

Adding a new broker (e.g., Interactive Brokers for futures): implement the trait, add a match arm in `apps/platform/src/main.rs`. Zero changes to the risk gate or strategy runtime.

### Fill Processing — Idempotent

`crates/execution/src/fills.rs`

Fills arrive from brokers either via WebSocket push or polling. They're converted to `Fill` domain objects and processed idempotently:

```rust
pub struct Fill {
    pub idempotency_key: Uuid,      // matches the originating OrderIntent
    pub broker_order_id: String,
    pub instrument_id:   String,
    pub side:            Side,
    pub filled_size:     Size,
    pub fill_price:      Price,
    pub commission:      Price,
    pub filled_at:       DateTime,
}
```

The `FillProcessor` maintains a `HashSet<Uuid>` of processed `idempotency_key`s. If the same fill arrives twice (broker sends it twice, or NATS redelivers it), the second time is a no-op. No double-counting of positions.

### Order State Machine

An order passes through these states:

```
Pending → Submitted → PartiallyFilled → Filled
                   ↓
               Cancelled
                   ↓
                Rejected (by broker)
```

State transitions are persisted to Postgres (`orders` table). Every transition is an audit event.

### Reconciliation

`crates/reconciliation/`

Even with idempotent fills, the system continuously verifies its internal state against the broker. This catches cases the fill stream misses (e.g., a manual trade placed directly at the broker).

Reconciliation triggers:
- **Fast path**: after every fill (compare just the affected instrument)
- **Sweep**: every 30 seconds (compare all instruments)
- **Startup**: full snapshot after reconnecting to a broker

On divergence:
1. Trip the kill switch (block new orders)
2. Raise an alert
3. Wait for explicit human confirmation before resuming

---

## Module 9: UI Data Flow — Subscription to Screen

### The Critical Split

There are **two separate data flows** in this system. Newcomers often confuse them:

| Flow | Consumers | Characteristics |
|------|-----------|----------------|
| **Canonical** | Storage, strategy runtime | Lossless, ordered, every event |
| **Lossy UI view** | Frontend panels | Throttled, sampled, may drop frames |

The strategy runtime **never reads the UI feed**. The canonical events go to the strategy runtime directly from NATS. The UI gateway is a separate consumer that applies lossy transformations.

### Frontend → Backend Connection

```
Browser opens WebSocket to ws://localhost:8081
  ↓  (HTTP upgrade)
crates/api/src/ws/live.rs
  ↓
Axum WebSocket handler
```

### Subscription Lifecycle

**Frontend sends:**
```json
{ "type": "subscribe", "lane": "market.bars.1m", "instrument_id": "BTC-USDT" }
```

**Backend (`crates/ui-gateway/src/subscriptions.rs`):**
1. Validate the lane exists
2. Check the user has permission for this lane (private lanes like `orders.alice.*` are scoped)
3. Fetch snapshot from Redis: `GET latest:market.bars.1m:BTC-USDT`
4. Send snapshot immediately (cold-start: user sees current state before live updates)
5. Subscribe to NATS subject: `market.bars.1m.BTC-USDT`
6. Declare demand in `DemandRegistry` (keeps the pipeline alive)

### The Shaping Layer

`crates/ui-gateway/src/shaping.rs`

Raw NATS events arrive at up to hundreds per second for an active instrument. The UI can't process that. The shaping layer:

- **Throttles**: `market.bars.1m` → max 20 fps (one frame every 50ms)
- **Aggregates**: Order book → collapse to top-5 bid/ask levels
- **Rounds**: Prices rounded to 2 decimal places for readability

Dropped frames are intentional. The strategy runtime never reads this feed, so dropping frames here has zero effect on trading decisions.

### Per-Panel Rate Limiting

Each panel subscription has its own rate limit. An order book panel showing 5 depth levels at 20fps is very different from a raw feed at 500 events/sec. The shaping layer handles this per-subscription.

### Frame Format

```json
{
  "lane": "market.bars.1m",
  "instrument_id": "BTC-USDT",
  "data": {
    "timeframe": "minutes_1",
    "open": "50000.00",
    "high": "50500.00",
    "low": "49800.00",
    "close": "50250.00",
    "volume": "123.45",
    "available_time": "2026-06-09T15:05:00Z"
  }
}
```

---

## Module 10: Replay — Deterministic Simulation

> **Note:** Backtesting is explicitly **out of scope** for this repository (removed 2026-06-10). This module covers the *replay invariants* — the timestamp discipline and pure-function rules that guarantee live correctness and determinism. These invariants stay even though no backtest engine exists here.

### The Core Problem with Historical Simulation

Most historical-simulation systems have **lookahead bias** — the simulation accidentally uses information that wouldn't have been available at the time a trade was made. For example:

- Using a bar's closing price to decide to trade at the open
- Using features computed from the "full" bar before the bar is finalized
- Sorting events by `event_time` (when they *happened*) instead of `available_time` (when you *knew* about them)

This system eliminates lookahead bias structurally. It's not a rule — it's enforced by the types.

### The Four Timestamps

Every `EventEnvelope` carries four timestamps:

| Timestamp | When it is | Who sets it |
|-----------|-----------|-------------|
| `event_time` | When the event happened at the venue | The venue itself (may be absent or wrong) |
| `observed_time` | When this process received the raw bytes | The collector |
| `ingested_time` | When it was written to the event bus | The normalizer |
| `available_time` | When a strategy is *allowed* to use it | Computed: `max(event_time, observed_time) + watermark + delay` |

The `available_time` formula accounts for:
- Processing delay (network latency, normalization time)
- Watermark (the bar builder waits this long after window close before emitting a bar)

### `StrategyClock` — Abstracting Time

`crates/strategy-runtime/src/clock.rs`

```rust
pub trait StrategyClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct WallClock;   // live: returns Utc::now()
pub struct ReplayClock { time: Arc<RwLock<DateTime<Utc>>> } // replay: controlled externally
```

When a strategy calls `world.now()`, it calls the injected clock. In live mode this is wall clock time. In replay mode the replay engine advances the `ReplayClock` to each event's `available_time` before dispatching the event. The strategy cannot tell the difference — it always just calls `world.now()`.

### Replay Sort Order

The replay engine loads events from the Parquet archive and sorts them strictly by `available_time`. This means:

- Events from different venues are interleaved correctly (e.g., Kraken trade at T+0ms, Alpaca trade at T+3ms, but if the Alpaca trade was available later due to network, it appears later)
- A bar event with `available_time = T+2s` (because of the 2s watermark) appears after trades at T+0s and T+1s
- Features computed from a bar (which take additional processing time) appear after the bar

This ordering makes it structurally impossible for a strategy to see something from its own future.

### Same Builder Code, Always

The bar builder and feature engine are pure functions — no I/O. They run identically in live and replay. This guarantees that:

- The 1-minute bar produced from replayed events is identical to the 1-minute bar produced live
- Replayed EMA values are identical to the live values (recorded with their `available_time`)
- There is no "recomputed with full data" problem

These guarantees exist for **live correctness and auditability** — being able to reconstruct exactly what the system saw and when. Backtesting tooling, if it ever returns, would build on the same invariants, but no backtest engine, adapter, or API exists in this repository.

---

## Module 11: Platform Boot Sequence

`apps/platform/src/main.rs`

When you run `cargo run -p platform`, here is exactly what happens in order:

**Step 1: Load Config**
```rust
let cfg = cfg::load()?;
```
Merges `config/default.toml` + `APP__*` env vars. Crashes on startup if any required field is missing or invalid. You never get a partially-configured system.

**Step 2: Init Tracing**
```rust
observability::init_json("platform");   // or init() for human-readable
```
Sets up structured logging. All subsequent log lines are JSON (in production) or human-readable (in development).

**Step 3: Connect to Postgres**
```rust
let pg = storage::postgres::connect(&cfg.database.url).await?;
```
Creates a connection pool. Runs any pending `sqlx` migrations automatically.

**Step 4: Load Kill Switch State**
```rust
let initially_halted = load_kill_switch_state(&pg).await;
let kill_switch = Arc::new(risk::KillSwitch::new(initially_halted));
```
Reads `trading_enabled` from the `global_risk_config` table. If the system was halted before shutdown, it starts halted. Safety-first: default is halted.

**Step 5: Create Risk Gate**
```rust
let risk_gate = Arc::new(risk::RiskGate::new(
    risk::GlobalRiskLimits::default(),
    Arc::clone(&kill_switch),
));
```
The single risk gate. Shared across all order paths.

**Step 6: Create Execution Engine**
```rust
let broker = match execution::alpaca::AlpacaBroker::from_env() {
    Ok(b)  => Arc::new(b) as Arc<dyn Broker>,
    Err(_) => Arc::new(NoBroker),
};
let execution_engine = Arc::new(execution::ExecutionEngine::new(broker));
```
Tries to load Alpaca credentials from env. Falls back to `NoBroker` (which logs a warning and rejects all submissions). This means the platform starts and serves the API even without broker credentials — useful for development.

**Step 7: Create Demand Manager and UI Gateway**
```rust
let demand_registry = Arc::new(demand_manager::DemandRegistry::new(
    Arc::new(demand_manager::NoopPipelineFactory),
));
let gateway = Arc::new(ui_gateway::SubscriptionRegistry::new(demand_registry));
```
MVP uses `NoopPipelineFactory` (no-op). In production this is replaced with a real factory that starts collectors.

**Step 8: Build AppState and Router**
```rust
let app_state = api::AppState::new(pg, risk_gate, kill_switch, execution_engine, gateway);
let router = api::router(app_state);
```
All components assembled into one shared state struct. The Axum router gets all components through `AppState`.

**Step 9: Listen**
```rust
let listener = tokio::net::TcpListener::bind(&addr).await?;
axum::serve(listener, router).await?;
```
Server is live. REST and WebSocket requests are now handled.

**No automatic strategy startup. No automatic collector startup.** Everything starts on-demand.

---

## Module 12: Adding a New Asset Class

This module answers: "what do I actually touch when I add, say, DEX/AMM trading?"

### The Core Rule

> **Do not branch on `AssetClass` in core code. All asset-class differences live in instrument metadata.**

This rule is documented in `docs/procedures/add-a-venue.md` and verified by the Phase 6 equity addition — adding equities required zero changes to `crates/risk`, `crates/strategy-runtime`, `crates/storage`, or `crates/builders`.

### Step-by-Step: Adding DEX/AMM (Example)

**1. Activate the enum variant** (already exists)
```rust
// crates/domain/src/instrument.rs — already there, remove "planned" comment
CryptoSpotDex,
```

**2. Create the Collector**
```
crates/collectors/src/dex/uniswap.rs
```
Implement the `Collector` trait. Connect to 0x Swap API or a Uniswap subgraph. Normalize pool state events into `EventEnvelope<AmmPoolPayload>`.

**3. Add the Payload Type**
```
crates/domain/src/payloads/amm_pool.rs
```
```rust
pub struct AmmPoolPayload {
    pub reserve_0:    Decimal,  // token0 reserves
    pub reserve_1:    Decimal,  // token1 reserves
    pub fee_tier:     u32,      // e.g., 3000 = 0.3%
    pub price_impact: Decimal,  // estimated for a standard trade size
}
```

**4. Create a Satellite Binary**
```
apps/collector-dex/src/main.rs
```
Same structure as `apps/collector-crypto/`. Load config, connect NATS, run the collector.

**5. Update the Venue Router**
```rust
// crates/venue-router/src/resolver.rs
AssetClass::CryptoSpotDex => "uniswap",
```

**6. Implement the Broker**
```
crates/execution/src/dex.rs
```
Implement the `Broker` trait. `submit()` calls the 0x Swap API or signs and broadcasts a transaction.

**7. Insert Instrument Rows**
```sql
INSERT INTO instruments (instrument_id, asset_class, venue_id, tick_size, lot_size,
                         halt_behavior, trust_tier, trading_hours, watermark_secs)
VALUES ('WETH-USDC-0.05', 'CryptoSpotDex', 'uniswap', 0.000001, 0.001,
        'NonHaltable', 'OnchainConfirmed', '{"always_open": true}', 2);
```

**8. Wire the new broker in `apps/platform/src/main.rs`**
Add a match arm for DEX environment variables.

**What you did NOT change:** `crates/risk`, `crates/strategy-runtime`, `crates/storage`, `crates/builders`, `crates/features`. They work identically because they branch on metadata properties, not the `AssetClass` enum.

### Asset Class Roadmap

| Phase | Asset Class | API to start with | Status |
|-------|-------------|------------------|--------|
| 1 | Crypto Spot (CEX) | Coinbase Advanced Trade / Kraken | Active |
| 2 | Equities + ETFs | Alpaca | Active |
| 3 | Options | Tradier | Planned |
| 4 | Futures | Tradovate | Planned |
| 5 | FX | OANDA | Planned |
| 6 | Perpetuals | Binance Futures / Kraken Futures | Planned |
| 7 | DEX / AMM | 0x Swap API | Planned |
| 8 | Prediction Markets | Kalshi | Planned |
| 9 | Bonds | Interactive Brokers | Planned |
| 10 | NFTs | OpenSea API | Low priority |

---

## Module 13: Critical Invariants — The Rules That Must Never Break

These are the hardcoded constraints that the entire system's correctness depends on. If you're making a change and it feels like it might violate one of these, stop and discuss first.

### Invariant 1: No `f64` on Price or Size

**What:** All prices and sizes use `Price(Decimal)` and `Size(Decimal)` newtypes. No `f64` for money values anywhere.

**Why:** Floating-point arithmetic produces rounding errors. `0.1 + 0.2 != 0.3` in IEEE 754. On financial data this compounds into real monetary loss.

**Enforcement:** `crates/domain/src/money.rs` has no `From<f64>` impl. The CI check "Money safety (no f64 on price/size)" runs `cargo xtask` to grep for violations.

**Where indicators use f64:** `crates/features/` uses `f64` for indicator values (EMA, RSI). This is acceptable because indicator values are not money — they're dimensionless signals, versioned, and recorded. Never put indicator values directly into a price or size field.

---

### Invariant 2: Single Risk Gate, No Bypass Path

**What:** Every order — manual or strategy-generated — passes through `RiskGate::check()`. There is no code path from intent to broker that skips the gate.

**Why:** The risk gate is what prevents runaway strategies from blowing up the account.

**Enforcement:** `ApprovedOrder` has a private `_sealed: ()` field. Only `RiskGate::check()` can construct it. Brokers only accept `ApprovedOrder`. The compiler makes bypass physically impossible.

**Corollary:** Never add a `pub fn new()` or any other constructor to `ApprovedOrder`. Never move `ApprovedOrder` construction outside the `gate.rs` module.

---

### Invariant 3: Append-Only Event History

**What:** Events are never mutated after publication. Late data (e.g., a late trade) produces a new revision event, not a mutation of the original.

**Why:** Mutable history makes replay unpredictable. If you change a bar after the fact, the replay sees different data than the live system did.

**Enforcement:** The `revision` field on `BarPayload`. Original bars have `revision = 0`. Late revisions have `revision > 0` and are published to a separate lane (`market.bars.1m.revised`). Parquet is append-only by design.

**Corollary:** Never write an UPDATE to the bars or trades tables. Never overwrite a Parquet file.

---

### Invariant 4: Same Builder Code Live and Replay

**What:** `crates/builders/` and `crates/features/` contain zero I/O. They are pure functions. The same code runs live (fed by bus events) and in replay (fed by Parquet events).

**Why:** If builders or features behave differently in replay than live, any historical reconstruction or audit is meaningless.

**Enforcement:** CI check "builders/features purity (no I/O deps)" fails if these crates import anything that touches the network, database, clock, or file system.

**Corollary:** Never add `use storage::`, `use event_bus::`, or `use tokio::time::` to `crates/builders/` or `crates/features/`. If you need time, receive it as a parameter — don't read `Utc::now()`.

---

### Invariant 5: `available_time` Ordering Prevents Lookahead

**What:** The replay engine sorts events strictly by `available_time`. The `ReplayClock` is advanced to each event's `available_time` before the event is dispatched to the strategy.

**Why:** Using `event_time` (when the event *happened*) instead of `available_time` (when you *knew* about it) is lookahead bias. A strategy can't trade on information it didn't yet have.

**Enforcement:** `StrategyClock` is a trait injected into the runtime. `world.now()` returns the clock's value, not `Utc::now()`. In replay mode, the clock is a `ReplayClock` that only advances when the replay engine advances it. There is no way for strategy code to call `Utc::now()` directly through the `WorldContext` API.

**Corollary:** Never add a `Utc::now()` call inside `crates/strategy-runtime/src/interpreter.rs` or any code called from `process_event()`. Time must always come from `world.now()`.

---

### Invariant 6: Idempotency on All Money-Mutating Paths

**What:** Orders, fills, and risk gate decisions are all keyed by an `idempotency_key`. Redelivering the same message is always a no-op.

**Why:** NATS guarantees *at least once* delivery, not exactly once. Any money-mutating path will see duplicates. Without idempotency, a duplicate fill would double-count a position.

**Enforcement:**
- Orders: `OrderIntent.idempotency_key` (UUID v4 set by caller)
- Fills: `Fill.idempotency_key` (matches originating order)
- Risk gate: `seen_keys` HashMap caches decisions by key

**Corollary:** Never skip setting `idempotency_key` when creating an `OrderIntent`. Never process a fill without checking the `idempotency_key` cache first.

---

### Invariant 7: Canonical vs Lossy Split

**What:** The strategy runtime consumes the canonical NATS stream (lossless, all events). The UI gateway consumes a separate, intentionally-lossy view (throttled, frames may be dropped).

**Why:** The strategy runtime needs every event to maintain correct state (e.g., every trade to build accurate bars). The frontend needs a human-viewable rate, not 500 events/second.

**Enforcement:** The strategy runtime never subscribes to a UI gateway topic. The UI gateway's `shaping.rs` has explicit drop logic. These are two separate subscription paths from NATS.

**Corollary:** Never route the strategy runtime through the UI gateway's throttled feed. Never add a database read or UI gateway call inside `StrategyInstance::process_event()`.

---

*End of NEWCOMERS.md — you now have a complete map of this codebase from terminology to invariants.*
