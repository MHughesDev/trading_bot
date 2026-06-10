# Architecture

> **Status:** Implemented — reflects the as-built system after the Python → Rust refactor (Phase 7 complete).
> Where this document and a spec disagree, the spec wins and this document should be corrected.

---

## 1. Overview

The system is an **all-in-one, data & asset scalable** trading platform — a **modular Rust monolith** (one main binary + satellite collectors) built on **NATS JetStream** as the event fabric. Asset classes (crypto spot, equities, options, futures, FX, perpetuals, DEX/AMM, prediction markets) are added through instrument metadata rows and new collector/broker implementations; no changes to core runtime, risk, storage, or replay are ever required. Six planes separate concerns:

| Plane | Purpose | Transport |
|-------|---------|-----------|
| **Control** | Start/stop strategies, config, user actions, history | REST (Axum) |
| **UI Live** | Live visualization for React frontend | WebSocket / SSE |
| **Data** | Internal normalized events between services | NATS JetStream |
| **Storage** | Durable historical record | ClickHouse + Postgres + Parquet + Redis |
| **Strategy** | Decision-grade event consumption | Internal bus + runtime |
| **Replay** | Deterministic historical replay (`available_time` ordering) | Event store (Parquet archive) |

The **main binary** (`apps/platform`) co-locates the API gateway, UI streaming gateway, strategy runtime, risk gate, execution engine, and demand manager. These components are co-located for legibility — for a money-handling system run by a small team, the ability to ask "what did the system believe and do, and why" and get a straight answer is the most valuable property.

**Satellite collector processes** (one per venue/source) fail and reconnect independently without taking the core binary down. Each collector normalizes raw venue messages to typed `EventEnvelope` and publishes to the event bus.

The **React frontend** (`frontend/`) connects to the main binary via REST (control plane: strategies, orders) and WebSocket (live panel subscriptions). It is retained from the Python-era system and re-pointed at the new Rust API/WS contracts.

---

## 2. Components

| Component | Responsibility | Spec Reference |
|-----------|---------------|----------------|
| `crates/domain` | Core types: `Price`, `Size`, `EventEnvelope`, `Instrument`, `AssetClass`, payloads, strategy definition format, order/position types | DATA-001, DATA-002, DATA-003 |
| `crates/config` | Typed runtime configuration loading and validation (env + TOML); fail-fast on missing/invalid | SYS-001 |
| `crates/observability` | Tracing, structured logging, and metrics setup shared by all binaries; correctness metrics (consumer lag, queue depth, quarantine rate, reconciliation divergences) | SYS-001 |
| `crates/event-bus` | NATS JetStream producer/consumer wrappers, lane naming, quarantine lane, bounded-queue backpressure | COMP-001 |
| `crates/collectors` | Satellite connector logic: Kraken (crypto), Alpaca (equity), and web scraper venue connectors, `normalize()` → typed events, reconnect policy, gap detection | COMP-001 |
| `crates/graph` | TigerGraph capability/compatibility graph — idempotent schema DDL, vertex/edge upsert, `RegistrySnapshot` population from domain defaults | P7-T01, P7-T02 |
| `crates/semantic` | Milvus vector store — collection management, OpenAI embedding calls (`text-embedding-3-small`, 1536 dims), upsert and metadata-filtered similarity search | P7-T03 |
| `crates/builders` | **PURE** functions: order-book reconstruction from snapshot+deltas, bar building with watermark/revision logic. Zero I/O dependencies — same code runs live and in replay | DATA-003, COMP-001 |
| `crates/features` | **PURE** technical indicator functions: EMA, RSI, rolling window. Zero I/O dependencies — same code runs live and in replay | FEAT-001 |
| `crates/strategy-runtime` | `WorldState`/`WorldContext`, strategy interpreter (evaluates strategy-definition node graph), instance lifecycle, `world.now()` clock (no wall-clock reads) | FEAT-001 |
| `crates/strategy-validator` | Validates strategy-definition JSON against the frozen 1.0 format; all three front doors (REST, visual builder, MCP) target this single validator | FEAT-001 |
| `crates/risk` | Single risk gate + kill switch. Every order — manual or strategy-emitted — passes through here. Idempotent. Tighten-only risk overrides | COMP-002 |
| `crates/execution` | Broker adapters: Coinbase (live), Alpaca (paper). Order state machine, fill handling, position updates, execution audit trail | COMP-002 |
| `crates/reconciliation` | Position/balance reconciliation vs broker; per-lane freshness watchdog (market-hours-aware); sequence gap handling; divergence → kill switch trip | COMP-002 |
| `crates/storage` | Postgres/ClickHouse/Parquet/Redis persistence adapters; batched storage-writer consumer (10k events or 100ms) | COMP-004 |
| `crates/ui-gateway` | Throttled, intentionally lossy UI streaming; separate consumer view from canonical stream; per-panel rate limits; snapshot-on-connect | COMP-003 |
| `crates/demand-manager` | Tracks what lanes and instruments each consumer (strategy instance, UI panel) needs; aggregates counts; starts/stops pipelines | COMP-003, FEAT-001 |
| `crates/venue-router` | Resolves `(AssetClass, DataType)` → `VenueId` at runtime; starts/stops collector instances on demand; data pipelines never start at system init | COMP-002, ADR-0011 |
| `crates/api` | Axum REST routes + WS upgrade + auth; control-plane endpoints for strategies, orders, assets, kill switch | SYS-001 |
| `crates/mcp-server` | Thin MCP front door; targets the canonical strategy JSON via `strategy-validator`; no privileged broker path; no order-placement tool | INTG-001 |
| `apps/platform` | Main binary: wires api + ui-gateway + strategy-runtime + risk + execution + reconciliation + demand-manager; no logic, wiring only | SYS-001 |
| `apps/collector-crypto` | Satellite binary: starts the Kraken crypto collector; publishes to the bus; reconnects independently | COMP-001 |
| `apps/collector-equity` | Satellite binary: starts the Alpaca equity collector; publishes to the bus | COMP-001 |
| `apps/collector-web` | Satellite binary: robots.txt-compliant web scraper; emits `WebPageSnapshotPayload` events | P7-T04 |
| `apps/embedder` | Satellite binary: subscribes to social/web events; calls OpenAI embeddings API; upserts into Milvus | P7-T03 |

---

## 3. Data Flow

```
Market Venues
  ├── Kraken WS (crypto)
  └── Alpaca WS (equity)
       │
       ▼ normalize() → EventEnvelope
Satellite Collectors
  (apps/collector-crypto, apps/collector-equity)
       │
       ▼ publish to lane
Event Bus (NATS JetStream)
  ├── Typed lanes, partitioned by instrument/venue
  ├── Durable + replayable consumers
  └── Quarantine lane for schema failures
       │
       ├──────────────────────────────────────────┐
       ▼                                          │
Storage Writers ──────────────────────────────────┤
  ├── ClickHouse (bars, trades, features)         │
  ├── Postgres (orders, fills, positions)         │
  └── Parquet (raw event archive — ground truth)  │
                                                  │
       ▼                                          │
Feature Engine (crates/features — PURE)           │
       │                                          │
       ▼                                          │
Strategy Runtime (crates/strategy-runtime)        │
  WorldState + strategy interpreter               │
       │ order intents                            │
       ▼                                          │
Risk Gate (crates/risk)                           │
  Single chokepoint — every order passes here     │
       │ approved orders                          │
       ▼                                          │
Execution Engine (crates/execution)               │
  ├── Coinbase adapter (live)                     │
  └── Alpaca adapter (paper)                      │
       │                                          │
       ▼                                          │
Broker APIs ◄─────────────────────────────────────┘
  (fills published back to Event Bus)

UI Gateway (crates/ui-gateway)
  Separate, intentionally lossy consumer view
       │ WebSocket frames
       ▼
React Frontend
  ├── REST ↔ Axum API ↔ Main Binary
  │   (strategies, orders, instruments)
  └── WebSocket ↔ UI Gateway
      (live panels: chart, orderbook, positions)
```

---

## 4. External Dependencies

| Service | Role | Relevant ADR |
|---------|------|-------------|
| NATS JetStream | Event bus — the spine of the Data plane; durable pub/sub; quarantine lane | ADR-0003 |
| PostgreSQL | Transactional storage: users, orders, fills, positions, strategy definitions, risk config | ADR-0004 |
| ClickHouse | Time-series storage: bars, trades, features; columnar scan for historical analytics | ADR-0004 |
| Redis / Valkey | Latest-state cache: price snapshots, subscription state, rate-limit counters. Never source of truth for orders/fills | ADR-0004 |
| TigerGraph | Capability/compatibility graph: instruments ↔ venues ↔ asset classes ↔ strategy definitions. REST++ API on port 9000. | P7-T01 |
| Milvus | Vector database for semantic search over social/web/strategy content. REST API v2 on port 9091. | P7-T03 |
| OpenAI Embeddings API | `text-embedding-3-small` (1536 dims) called by `apps/embedder` at ingest time | P7-T03 |
| Coinbase Advanced Trade API | Live execution broker for crypto (REST + WS) | ADR-0006 |
| Alpaca API | Paper execution (all assets) + equity market data feed | ADR-0006 |
| Kraken WS | Crypto market data source (trades, quotes, L2 order-book) | ADR-0006 |

---

## 5. Key Decisions

| ADR | Title | Status |
|-----|-------|--------|
| ADR-0011 | On-Demand Pipeline Startup via Demand Manager + Venue Router | Accepted |
| ADR-0010 | MCP Server as Thin Strategy Front Door | Accepted |
| ADR-0009 | Backtest Engine Delegated to market_simulator | Superseded — backtesting removed from repo scope (2026-06-10) |
| ADR-0008 | Strategy Runtime Uses world.now() — No Wall-Clock Reads | Accepted |
| ADR-0007 | Strategy Definition Frozen at v1.0 Before Front Doors Built | Accepted |
| ADR-0006 | Broker and Venue Selection: Coinbase + Alpaca + Kraken | Accepted |
| ADR-0005 | Single Risk Gate — No Bypass Path | Accepted |
| ADR-0004 | Storage Split: Postgres + ClickHouse + Parquet + Redis | Accepted |
| ADR-0003 | NATS JetStream as Event Fabric | Accepted |
| ADR-0002 | Decimal Money Newtypes — No f64 for Price or Size | Accepted |
| ADR-0001 | Rust Modular Monolith with Satellite Collectors | Accepted |

Full ADR documents: `docs/adr/`

---

## 6. Known Constraints and Boundaries

**Deployment constraints:**
- Single team, private network, local-first deployment
- No multi-tenant isolation (per-user scoping is enforced on the wire but no tenant-isolation machinery)
- No Kafka / Redpanda — NATS JetStream is sufficient at this scope; revisit only if measured scale demands it

**Purity rules (enforced in CI):**
- `crates/builders` and `crates/features` must have **zero dependencies** on `storage`, `event-bus`, `redis`, `sqlx`, or `clickhouse`. They are pure functions over event streams. The same code runs live and in replay.
- No `From<f64>` on `Price` or `Size` anywhere in the workspace. The compiler enforces this.

**Single chokepoint:**
- All orders — manual or strategy-emitted — pass through `crates/risk`. There is no private path from the strategy runtime or the UI to a broker. Only `crates/risk` and `crates/execution` may construct an `ApprovedOrder`.

**On-demand pipelines:**
- Data pipelines (collectors, bar builders, feature engine) start **only** when at least one consumer (strategy instance or UI panel) has declared demand via the Demand Manager. They are never started at system initialization (ADR-0011).

**No backtesting in this repository:**
- Backtesting is explicitly out of scope (removed 2026-06-10). No backtest engine, no backtest adapter, no backtest API endpoints. The deterministic-replay invariants (`available_time` ordering, pure builders/features, `world.now()`) are retained because they guarantee live correctness and keep the door open for replay tooling later.

---

## 7. Repository Structure

As-built layout after the Python → Rust refactor. All Python artifacts removed in Phase 7.

```
trading-platform/                      # repo root (current trading_bot/, refactored)
├── Cargo.toml                         # workspace: members, shared deps, profiles, lints
├── Cargo.lock                         # committed lockfile
├── rust-toolchain.toml                # pins Rust toolchain version
├── rustfmt.toml                       # formatting config (all crates)
├── clippy.toml                        # lint config; deny-list for f64-on-money patterns
├── deny.toml                          # cargo-deny: license + advisory + duplicate-dep policy
├── .cargo/config.toml                 # build aliases, target dir, linker flags
├── .env.example                       # documented env vars (DB URLs, NATS URL, secrets)
├── README.md                          # repo overview, quickstart, local stack instructions
├── justfile                           # task runner: just dev, just test, just migrate, etc.
├── docker-compose.yml                 # local infra: NATS, Postgres, ClickHouse, Redis/Valkey
├── Dockerfile                         # multi-stage build for main binary + collectors
├── .github/workflows/
│   ├── ci.yml                         # fmt + clippy + test + cargo-deny on every PR
│   ├── frontend.yml                   # lint + typecheck + build React SPA
│   └── release.yml                    # tagged release: build binaries, publish artifacts
│
├── crates/                            # ── all LIBRARY crates (logic lives here) ──
│   ├── domain/                        # THE CORE — irreversible types; depends on nothing internal
│   │   └── src/
│   │       ├── envelope.rs            # EventEnvelope<T>
│   │       ├── timestamp.rs           # the 4 timestamps + semantics
│   │       ├── money.rs               # Price(Decimal) / Size(Decimal) — NO From<f64>
│   │       ├── trust.rs               # TrustTier enum
│   │       ├── instrument.rs          # Instrument metadata, AssetClass, TradingSchedule
│   │       ├── ids.rs                 # dedup-key / identity helpers
│   │       ├── lanes.rs               # canonical lane name constants + typed lane enum
│   │       ├── payloads/              # trade, quote, orderbook, bar payloads
│   │       ├── order.rs               # OrderRequest, OrderIntent, OrderState
│   │       ├── position.rs            # Position, Balance domain types
│   │       ├── strategy_def/          # StrategyDefinition root + inputs/nodes/actions/risk_overrides
│   │       └── error.rs               # domain-level error types
│   ├── config/                        # typed config loading + validation
│   ├── observability/                 # tracing/logging/metrics setup; correctness metrics
│   ├── event-bus/                     # NATS JetStream wrappers + lane naming + quarantine
│   ├── storage/                       # Postgres, ClickHouse, Parquet, Redis adapters + writer
│   ├── builders/                      # PURE: order-book reconstruction, bar building (live == replay)
│   ├── features/                      # PURE: indicator computation — EMA, RSI (live == replay)
│   ├── collectors/                    # venue connectors: Kraken (crypto), Alpaca (equity), web scraper
│   ├── graph/                         # TigerGraph capability graph: schema init, populate, rebuild
│   ├── semantic/                      # Milvus collection, OpenAI embedding, filtered search
│   ├── risk/                          # single risk gate + kill switch
│   ├── execution/                     # Coinbase (live), Alpaca (paper)
│   ├── reconciliation/                # position/balance/freshness/sequence reconciliation
│   ├── strategy-runtime/              # WorldState + strategy interpreter + instance lifecycle
│   ├── strategy-validator/            # validates strategy-definition JSON (frozen 1.0 format)
│   ├── demand-manager/                # aggregates lane/instrument demand; starts/stops pipelines
│   ├── venue-router/                  # resolves (AssetClass, DataType) → VenueId; on-demand lifecycle
│   ├── ui-gateway/                    # throttled, lossy, frontend-shaped live views
│   ├── api/                           # axum REST routes + WS upgrade + auth
│   └── mcp-server/                    # thin MCP front door → canonical strategy JSON
│
├── apps/                              # ── thin BINARY crates (wiring only) ──
│   ├── platform/                      # THE main binary: api+ui-gateway+runtime+risk+exec
│   ├── collector-crypto/              # satellite: Kraken crypto collector
│   ├── collector-equity/              # satellite: Alpaca equity collector
│   ├── collector-web/                 # satellite: robots.txt-compliant web scraper
│   ├── embedder/                      # satellite: OpenAI embeddings → Milvus upsert
│   └── mcp-server/                    # MCP server process (wraps crates/mcp-server)
│
├── migrations/                        # sqlx Postgres migrations (timestamped .sql files)
│   ├── 0001_users_accounts.sql
│   ├── 0002_instruments.sql
│   ├── 0003_orders_fills_positions.sql
│   ├── 0004_strategies.sql
│   └── 0005_risk_config.sql
├── clickhouse/                        # ClickHouse DDL: trades, bars, features tables
├── config/                            # runtime config TOML per environment
│   ├── default.toml
│   ├── local.toml
│   └── lanes.toml
├── frontend/                          # React + Vite SPA (retained, re-pointed at Rust API)
│   └── src/
│       ├── api/                       # typed REST + WS clients
│       ├── pages/                     # Dashboard, InstrumentDetail, AccountSettings
│       ├── panels/                    # ChartPanel, OrderBookPanel, TradePanel, PositionsPanel, StrategyPanel
│       ├── builder/                   # visual n8n-style strategy builder (Phase 5)
│       └── state/                     # client state: subscriptions, auth, selected instrument
├── tests/                             # cross-crate integration + end-to-end tests
│   ├── ingest_to_storage.rs
│   ├── manual_order_flow.rs
│   ├── strategy_end_to_end.rs
│   ├── quarantine_replay.rs
│   └── reconciliation_halt.rs
├── xtask/                             # Rust-based dev automation (seed, gen-fixtures, check-money-f64)
└── docs/                              # engineering docs: architecture, ADRs, specs, plans, research
```

### Crate dependency graph

```
domain  ──────────────────────────────────────────────────────  (no internal deps)
  ▲  ▲  ▲  ▲  ▲  ▲
  │  │  │  │  │  └── config, observability      (leaf utilities)
  │  │  │  │  └───── builders, features         (PURE — NO storage/bus deps)
  │  │  │  └──────── event-bus, storage         (domain + backend client)
  │  │  └─────────── collectors                 (domain + event-bus + builders)
  │  │               graph, semantic            (domain + reqwest — knowledge layer)
  │  └────────────── risk, execution, reconciliation, strategy-validator
  └───────────────── strategy-runtime, demand-manager, venue-router,
                     ui-gateway, mcp-server, api
                       (compose the above; api is top of the graph)

apps/*  depend on crates only — wiring, no logic.
```
