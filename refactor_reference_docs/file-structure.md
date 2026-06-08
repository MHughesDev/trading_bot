# End-State File & Folder Structure (Rust Monorepo)

> **Status:** Target/end-state design. This is the structure the repository should have **after**
> the full Python → Rust refactor is complete. It is one of the canonical reference documents for
> the restructuring effort and is referenced by every phase plan in [`plans/`](./plans/).
>
> **How to read this:** Every directory and every file that should exist at the end state is
> enumerated below with a one-line description of its responsibility. Tree nodes ending in `/` are
> directories. The descriptions are normative — when a phase plan says "create the `domain` crate,"
> the file list and responsibilities here are the contract for what that means.
>
> **Design source:** This structure is derived directly from the specs in [`spec/`](./spec/),
> especially [09-tech-stack.md](./spec/09-tech-stack.md) (crate layout), [01-architecture.md](./spec/01-architecture.md)
> (planes), and [02-data-model.md](./spec/02-data-model.md) (the irreversible core). Where this
> document and a spec disagree, the spec wins and this document should be corrected.

---

## Guiding principles for the structure

1. **One Cargo workspace at the repo root.** All Rust crates are members. `frontend/` (React) is the
   only non-Rust first-class top-level tree.
2. **Libraries are crates under `crates/`; runnable processes are thin binaries under `apps/`.**
   A binary crate contains almost no logic — it wires together library crates and starts a runtime.
   This keeps every piece of real logic unit-testable as a library.
3. **`domain` is the root of the dependency graph.** It depends on nothing internal and is imported
   everywhere. The irreversible types live here.
4. **Boundaries live in code even when deployed as one process.** The main binary co-locates the API,
   UI gateway, strategy runtime, risk gate, and execution per [01-architecture.md](./spec/01-architecture.md),
   but each is a separate crate with a clean interface, so any of them can be extracted into its own
   process later without a rewrite.
5. **Pure vs effectful is a hard boundary.** `builders/` and `features/` are pure functions over event
   streams (same code runs live and in replay — see [03-data-engineering.md](./spec/03-data-engineering.md) §6).
   They must not depend on `storage`, `event-bus`, or any I/O crate.
6. **No file touches money as `f64`.** `Price`/`Size` newtypes from `domain` with no `From<f64>` are
   the only way prices/sizes are represented anywhere in the tree.

---

## Top-level layout

```
trading-platform/                      # repo root (the current trading_bot/ directory, refactored)
├── Cargo.toml                         # workspace manifest: members, shared deps, profiles, lints
├── Cargo.lock                         # committed lockfile (this is an application workspace)
├── rust-toolchain.toml                # pins the Rust toolchain version for reproducible builds
├── rustfmt.toml                       # formatting config (shared across all crates)
├── clippy.toml                        # lint config; deny-list for f64-on-money patterns where feasible
├── deny.toml                          # cargo-deny: license + advisory + duplicate-dep policy
├── .cargo/
│   └── config.toml                    # build aliases, target dir, linker flags, registry settings
├── .env.example                       # documented env vars (DB URLs, NATS URL, secrets placeholders)
├── README.md                          # repo overview, quickstart, how to run the stack locally
├── justfile                           # task runner: `just dev`, `just test`, `just migrate`, etc.
├── docker-compose.yml                 # local infra: NATS JetStream, Postgres, ClickHouse, Redis/Valkey
├── Dockerfile                         # multi-stage build for the main binary + collectors
├── .dockerignore
├── .gitignore
├── .github/
│   └── workflows/
│       ├── ci.yml                     # fmt + clippy + test + cargo-deny on every PR
│       ├── frontend.yml               # lint + typecheck + build the React SPA
│       └── release.yml                # tagged release: build binaries, publish artifacts
│
├── crates/                            # ── all LIBRARY crates (logic lives here) ──
│   ├── domain/                        # THE CORE — irreversible types; depends on nothing internal
│   ├── config/                        # typed runtime configuration loading + validation
│   ├── observability/                 # tracing/logging/metrics setup shared by all binaries
│   ├── event-bus/                     # NATS JetStream producer/consumer wrappers + lane naming
│   ├── storage/                       # Postgres, ClickHouse, Parquet, Redis persistence adapters
│   ├── builders/                      # PURE: order-book reconstruction, bar building (live == replay)
│   ├── features/                      # PURE: indicator/feature computation (live == replay)
│   ├── collectors/                    # venue connectors + normalize() → typed events
│   ├── risk/                          # the single risk gate + kill switch
│   ├── execution/                     # broker adapters, order state machine, fills, positions
│   ├── reconciliation/               # position/balance/sequence/freshness reconciliation
│   ├── strategy-runtime/             # WorldState + strategy execution engine
│   ├── strategy-validator/           # validates strategy-definition JSON against the frozen format
│   ├── demand-manager/               # aggregates lane/instrument demand; starts/stops pipelines
│   ├── venue-router/                 # resolves (AssetClass, DataType, Instrument) → VenueId; starts/stops collectors on demand
│   ├── ui-gateway/                   # throttled, lossy, frontend-shaped live views
│   ├── api/                          # axum REST routes + WS upgrade + auth (control plane)
│   ├── market-simulator-adapter/     # adapter to github.com/MHughesDev/market_simulator
│   └── mcp-server/                   # thin MCP front door → canonical strategy JSON
│
├── apps/                              # ── thin BINARY crates (wiring only) ──
│   ├── platform/                     # THE main binary (monolith): api+ui-gateway+runtime+risk+exec
│   ├── collector-crypto/             # satellite process: one crypto venue collector
│   ├── collector-equity/             # satellite process: one equity collector
│   └── mcp-server/                   # MCP server process (wraps crates/mcp-server)
│
├── migrations/                        # sqlx Postgres migrations (timestamped .sql files)
├── clickhouse/                        # ClickHouse DDL: table schemas, ReplacingMergeTree configs
├── config/                            # runtime config files (TOML) per environment
├── frontend/                          # React + Vite SPA (control plane UI + live panels)
├── tests/                             # cross-crate integration + end-to-end tests
├── xtask/                             # Rust-based dev automation (codegen, seed data, fixtures)
├── docs/                              # generated/maintained engineering docs (not the spec)
└── legacy_python/                     # the old Python system, quarantined; deleted in Phase 7
```

---

## `Cargo.toml` (workspace root)

```
trading-platform/Cargo.toml
```
- Declares `[workspace]` with `members = ["crates/*", "apps/*", "xtask"]` and `resolver = "2"`.
- `[workspace.dependencies]` pins every external crate **once** (the set from
  [09-tech-stack.md](./spec/09-tech-stack.md)); member crates reference them with `.workspace = true`.
- `[workspace.lints]` enables shared clippy/rustc lints (e.g. `clippy::all`, deny `unwrap_used` in libs).
- `[profile.release]` tuned (LTO, codegen-units) and `[profile.dev]` kept fast.

---

## `crates/domain/` — the irreversible core

> Build this first. Everything depends on it. It has **no internal dependencies** and the lightest
> possible external ones (serde, chrono, uuid, rust_decimal). Source: [02-data-model.md](./spec/02-data-model.md).

```
crates/domain/
├── Cargo.toml
├── src/
│   ├── lib.rs                         # re-exports the public API of every module below
│   ├── envelope.rs                    # EventEnvelope<T>: ids, type, lane, the 4 timestamps, trust, payload
│   ├── timestamp.rs                   # the 4 timestamps + semantics; available_time helpers
│   ├── money.rs                       # Price(Decimal) / Size(Decimal) newtypes — NO From<f64>
│   ├── trust.rs                       # TrustTier enum (regulated … social_derived) + ordering
│   ├── instrument.rs                  # Instrument metadata, AssetClass, TradingSchedule, HaltPolicy
│   ├── ids.rs                         # deterministic dedup-key / identity helpers (per 02 §identity)
│   ├── lanes.rs                       # canonical lane name constants + typed lane enum
│   ├── payloads/
│   │   ├── mod.rs                     # payload trait + versioned-payload registry glue
│   │   ├── trade.rs                   # TradePayload (price, size, side, exchange_trade_id)
│   │   ├── quote.rs                   # QuotePayload (L1 bid/ask + sizes)
│   │   ├── orderbook.rs               # OrderBookPayload (Snapshot|Delta, levels, sequence, is_tentative)
│   │   └── bar.rs                     # BarPayload (timeframe, OHLCV, trade_count, revision)
│   ├── order.rs                       # OrderRequest, OrderIntent, OrderState, Side, OrderType, idempotency key
│   ├── position.rs                    # Position, Balance domain types
│   ├── strategy_def/
│   │   ├── mod.rs                     # StrategyDefinition root type (definition_version = "1.0")
│   │   ├── inputs.rs                  # input declarations (lane, instrument, $each fan-out, features)
│   │   ├── nodes.rs                   # node graph types (condition/signal/etc.) + expression AST hook
│   │   ├── actions.rs                 # action types (place_order, size_mode, …)
│   │   └── risk_overrides.rs          # risk-override type with tighten-only invariants documented
│   └── error.rs                       # domain-level error types (NormalizeError, ValidationError seeds)
└── tests/
    ├── money_no_float.rs              # compile-fail / behavioral proof that f64 cannot become a Price
    ├── envelope_roundtrip.rs          # serde round-trip for envelope + every payload
    └── strategy_def_schema.rs         # the frozen 1.0 format serializes/deserializes stably
```

---

## `crates/config/` — typed configuration

```
crates/config/
├── Cargo.toml
└── src/
    ├── lib.rs                         # load(): env + TOML → typed Config; fail fast on missing/invalid
    ├── model.rs                       # Config structs: db urls, nats url, risk limits, watermarks, ports
    └── secrets.rs                     # secret resolution (env-only in v1; no secrets in TOML)
```

---

## `crates/observability/` — tracing, logs, metrics

> Throughput **and** correctness observability live together (per [03-data-engineering.md](./spec/03-data-engineering.md) §7).

```
crates/observability/
├── Cargo.toml
└── src/
    ├── lib.rs                         # init(): wires tracing-subscriber (JSON logs) + metrics exporter
    ├── tracing_setup.rs               # subscriber, env-filter, span conventions
    ├── metrics.rs                     # metric registry + helpers (counters/histograms/gauges)
    └── correctness.rs                 # correctness metrics: consumer lag, queue depth, quarantine rate,
                                       #   reconciliation divergences, freshness-watchdog state
```

---

## `crates/event-bus/` — the spine

> NATS JetStream wrappers. The only crate that knows the bus is NATS; everyone else uses these traits.

```
crates/event-bus/
├── Cargo.toml
└── src/
    ├── lib.rs                         # public traits: Producer, Consumer, and connect()
    ├── nats.rs                        # async-nats JetStream impl of Producer/Consumer
    ├── lanes.rs                       # lane → JetStream subject mapping + partition key rules
    ├── publish.rs                     # typed publish<T>(envelope) with serde encoding
    ├── subscribe.rs                   # typed durable subscriptions, ack/nack, redelivery handling
    ├── quarantine.rs                  # publish raw bytes + error to the quarantine lane
    └── backpressure.rs               # bounded-queue wrappers, lag/queue-depth metric hooks
```

---

## `crates/storage/` — durable record

> Storage split by access pattern (per [07-storage-and-replay.md](./spec/07-storage-and-replay.md)).
> Redis is cache only — never source of truth for orders/fills.

```
crates/storage/
├── Cargo.toml
└── src/
    ├── lib.rs                         # Storage facade trait grouping the backends
    ├── postgres/
    │   ├── mod.rs                     # sqlx pool + transaction helpers
    │   ├── instruments.rs             # instrument-metadata CRUD
    │   ├── orders.rs                  # orders/fills/positions persistence (+ append-only audit log)
    │   ├── strategies.rs              # strategy-definition persistence + versioning
    │   └── users.rs                   # users/accounts/permissions
    ├── clickhouse/
    │   ├── mod.rs                     # clickhouse client + batched insert helper
    │   ├── bars.rs                    # bars table writer (ReplacingMergeTree on dedup key)
    │   ├── trades.rs                  # trades table writer
    │   └── features.rs                # features table writer
    ├── parquet/
    │   ├── mod.rs                     # raw normalized event archive writer (ground truth)
    │   ├── partition.rs               # lane/instrument/date partition path logic
    │   └── compaction.rs              # nightly small-file → big-file compaction job
    ├── redis.rs                       # latest-state cache (latest:{lane}:{instrument}) + seen-set dedup
    └── writer.rs                      # the storage-writer consumer: batches (10k or 100ms), routes
```

---

## `crates/builders/` — PURE derivations (live == replay)

> No I/O. Pure functions over event streams. Must not depend on `storage` or `event-bus`.
> Source: [03-data-engineering.md](./spec/03-data-engineering.md) §6.

```
crates/builders/
├── Cargo.toml
├── src/
│   ├── lib.rs                         # re-exports builders; documents the purity contract
│   ├── orderbook.rs                   # L2 reconstruction from snapshot+deltas; gap detection
│   ├── bars.rs                        # bar builder (1s, 1m); watermark + revision emission logic
│   └── watermark.rs                   # watermark policy applied to windows (per-source configurable)
└── tests/
    ├── bars_watermark.rs              # late trade after watermark → revision event, original immutable
    ├── bars_determinism.rs            # same input stream → byte-identical bars every run
    └── orderbook_gaps.rs              # sequence gap → gap.detected + snapshot re-request signal
```

---

## `crates/features/` — PURE indicators (live == replay)

```
crates/features/
├── Cargo.toml
├── src/
│   ├── lib.rs                         # feature registry; each feature stamps its own available_time delay
│   ├── ema.rs                         # EMA(n)
│   ├── rsi.rs                         # RSI(n)
│   └── window.rs                      # rolling-window primitives shared by indicators
└── tests/
    └── features_versioned.rs          # feature values are versioned + reproducible across runs
```

---

## `crates/collectors/` — venue connectors

> Satellite logic. Each venue is built deliberately differently to prove the abstraction
> (per [10-open-questions.md](./spec/10-open-questions.md) Q2). `normalize()` returns
> `Result<Vec<EventEnvelope>, NormalizeError>`; failures go to quarantine.

```
crates/collectors/
├── Cargo.toml
└── src/
    ├── lib.rs                         # Collector trait: connect, stream, normalize, reconnect policy
    ├── normalizer.rs                  # shared normalize helpers; schema-on-write validation
    ├── reconnect.rs                   # backoff/reconnect + replay-on-reconnect (causes dedup load)
    ├── gap.rs                         # sequence-gap detection + snapshot re-request trigger
    ├── crypto/
    │   ├── mod.rs                     # crypto collector wiring
    │   └── kraken.rs                  # Kraken WS: trades, quotes, L2 orderbook, tickers → typed events
    └── equity/
        ├── mod.rs                     # equity collector wiring (hours/halt aware)
        └── alpaca_data.rs             # Alpaca WS data feed: equity trades, quotes, bars → typed events
```

---

## `crates/risk/` — the one chokepoint

> Every order (manual or automated) passes through here. Idempotent. Source: [05-execution-and-risk.md](./spec/05-execution-and-risk.md).

```
crates/risk/
├── Cargo.toml
├── src/
│   ├── lib.rs                         # RiskGate::check(intent) -> Result<ApprovedOrder, RiskRejection>
│   ├── gate.rs                        # the synchronous gate; idempotency-key dedup of intents
│   ├── limits.rs                      # max position, max order rate, price sanity, lot/tick, max daily loss
│   ├── trust_gate.rs                  # refuse orders derived from data below strategy min_trust_tier
│   ├── overrides.rs                   # apply risk_overrides as tighten-only (reject if loosening)
│   └── kill_switch.rs                 # global trading_enabled flag; auto-trip conditions + manual trip
└── tests/
    ├── tighten_only.rs                # an override that loosens a global limit is rejected
    ├── idempotent_gate.rs             # redelivered intent does not double-approve
    └── kill_switch_trips.rs           # each auto-trip condition blocks new orders
```

---

## `crates/execution/` — order flow

```
crates/execution/
├── Cargo.toml
├── src/
│   ├── lib.rs                         # ExecutionEngine + Broker adapter trait (same interface across all three systems)
│   ├── broker.rs                      # Broker trait: submit, cancel, query_open_orders, query_positions
│   ├── coinbase.rs                    # LIVE adapter — Coinbase REST+WS orders (all assets, all domains)
│   ├── alpaca.rs                      # PAPER adapter — Alpaca paper account (all assets, all domains)
│   ├── market_simulator.rs            # BACKTEST adapter — wraps market_simulator (github.com/MHughesDev/market_simulator) fills
│   ├── order_state.rs                 # order state machine (accepted→submitted→filled/cancelled/…)
│   ├── fills.rs                       # fill + partial-fill handling; idempotent by fill id
│   ├── positions.rs                   # position + balance updates from fills
│   ├── audit.rs                       # execution audit trail (append-only)
│   └── events.rs                      # publish the sacred orders.*/positions.*/balances.* lanes (never dropped)
└── tests/
    ├── idempotent_fills.rs            # replaying a fill is a no-op
    ├── ack_timeout_query.rs           # missing ack → query, never blind retry
    └── partial_fill.rs                # partial fills aggregate correctly into position
```

---

## `crates/reconciliation/` — desync defense

> Where money is actually saved (per [05-execution-and-risk.md](./spec/05-execution-and-risk.md), [03-data-engineering.md](./spec/03-data-engineering.md) §7).

```
crates/reconciliation/
├── Cargo.toml
├── src/
│   ├── lib.rs                         # reconciliation orchestrator (scheduled, not heroic)
│   ├── positions.rs                   # internal vs broker positions; on-fill + 30s sweep + on-reconnect
│   ├── freshness.rs                   # per-lane freshness watchdog; reads instrument trading_hours/halt
│   ├── sequence.rs                    # consume gap.detected; mark windows suspect
│   └── divergence.rs                  # on divergence → trip kill switch for instrument + alarm
└── tests/
    ├── position_divergence_halts.rs   # divergence halts new orders on that instrument
    └── freshness_respects_hours.rs    # normal stock close at 4pm does NOT false-alarm
```

---

## `crates/strategy-runtime/` — decision-grade consumption

> Consumes canonical events (never the UI feed). Same interface live and in backtest.
> Source: [04-strategy-system.md](./spec/04-strategy-system.md).

```
crates/strategy-runtime/
├── Cargo.toml
├── src/
│   ├── lib.rs                         # Strategy trait + runtime instance lifecycle
│   ├── runtime.rs                     # load def → declare demand → subscribe canonical → on_event loop
│   ├── world.rs                       # WorldState/WorldContext: latest_bar, feature, position, place_order
│   ├── interpreter.rs                 # evaluates the strategy-definition node graph / expressions
│   ├── clock.rs                       # world.now(): real live, simulated in replay (no wall-clock reads)
│   └── intents.rs                     # emit order intents → risk gate (never a private broker path)
└── tests/
    ├── no_wallclock.rs                # strategy uses world.now(), not system time
    └── replay_determinism.rs          # same event sequence → identical decisions
```

---

## `crates/strategy-validator/` — the shared gatekeeper

> The single validator all three front doors target (per [04](./spec/04-strategy-system.md), [08](./spec/08-mcp-server.md)).

```
crates/strategy-validator/
├── Cargo.toml
├── src/
│   ├── lib.rs                         # validate(def) -> Result<ValidatedDefinition, Vec<ValidationError>>
│   ├── schema.rs                      # structural validation against frozen 1.0 format
│   ├── expressions.rs                 # condition/expression language validation
│   └── risk.rs                        # enforce risk_overrides tighten-only at author time
└── tests/
    └── rejects_loosening.rs           # a definition that loosens global risk is rejected with errors
```

---

## `crates/demand-manager/` — pipeline lifecycle

```
crates/demand-manager/
├── Cargo.toml
└── src/
    ├── lib.rs                         # DemandManager: register/decrement demand per (lane, instrument)
    ├── registry.rs                    # consumer → needs map; aggregate counts
    └── lifecycle.rs                   # start/keepalive/downshift/stop pipelines per aggregated demand
```

---

## `crates/venue-router/` — venue resolution and collector lifecycle

> Sits between the Demand Manager and the collectors. Maps `(AssetClass, DataType, Instrument)` to a
> `VenueId` (config-driven routing table) and starts/stops the correct collector when demand appears
> or disappears. **Data engines never start at system init** — they start only when at least one
> strategy or UI panel has declared demand via the Demand Manager. Adding a new venue is a new routing
> rule + a new collector; no other crate changes.
>
> Routing table (resolved decisions, all assets/domains):
> - `(Crypto, Any DataType, *)` → **Kraken** (market data)
> - `(Equity, Any DataType, *)` → **Alpaca data feed** (market data)
> - Execution routing is separate: live → Coinbase, paper → Alpaca paper account, backtest → market_simulator

```
crates/venue-router/
├── Cargo.toml
└── src/
    ├── lib.rs                         # VenueRouter: ensure_running(lane, instrument); stop_if_unneeded()
    ├── registry.rs                    # config-driven routing table: (AssetClass, DataType) → VenueId
    ├── resolver.rs                    # resolve_venue(asset_class, data_type, instrument) → VenueId
    └── lifecycle.rs                   # ref-counted start/stop of collector instances per (VenueId, lane, instrument)
```

---

## `crates/ui-gateway/` — lossy human views

> Intentionally drops frames. Separate consumer view; never the canonical stream.
> Source: [06-ui-and-streaming.md](./spec/06-ui-and-streaming.md).

```
crates/ui-gateway/
├── Cargo.toml
└── src/
    ├── lib.rs                         # gateway entry: manages subscriptions → WS frames
    ├── subscriptions.rs               # register/remove panel subscriptions; authz per subscription
    ├── throttle.rs                    # per-panel rate limits; top-N orderbook snapshot every 50–150ms
    ├── shaping.rs                     # UI-safe payload shaping (public vs private lane enforcement)
    ├── snapshot.rs                    # snapshot-on-connect + reconnect recovery
    └── transport.rs                   # WS/SSE framing, heartbeat/ping-pong, compression, batching
```

---

## `crates/api/` — control plane

> REST is the control plane, not the data plane. Source: [06-ui-and-streaming.md](./spec/06-ui-and-streaming.md).

```
crates/api/
├── Cargo.toml
└── src/
    ├── lib.rs                         # build_router(state) -> axum Router; serves frontend/dist static
    ├── state.rs                       # AppState: handles to risk, execution, runtime, storage, bus
    ├── auth/
    │   ├── mod.rs                     # auth middleware (per-user scoping for private data)
    │   └── session.rs                 # session/token handling for the trusted group
    ├── routes/
    │   ├── mod.rs                     # route registration
    │   ├── strategies.rs              # create/start/stop/get config; targets strategy-validator
    │   ├── orders.rs                  # POST /api/orders (manual) → risk gate; DELETE /api/orders/{id}
    │   ├── backtests.rs               # POST /api/backtests; GET /api/backtests/{id}
    │   ├── assets.rs                  # GET /api/assets; GET /api/instruments/{id}
    │   ├── streams.rs                 # GET /api/streams/available; POST /api/ui/subscriptions
    │   └── trading.rs                 # POST /api/trading/kill (kill switch)
    └── ws/
        ├── mod.rs                     # GET /ws/live upgrade
        └── live.rs                    # bridges WS connection ↔ ui-gateway subscriptions
```

---

## `crates/market-simulator-adapter/` — backtest bridge

> This repo does NOT own a fill simulator or replay engine. Backtesting is delegated entirely to
> `github.com/MHughesDev/market_simulator`. This crate is the thin adapter between this repo's
> domain types and the market_simulator's Arrow IPC contracts.
> Source: [07-storage-and-replay.md](./spec/07-storage-and-replay.md).

```
crates/market-simulator-adapter/
├── Cargo.toml
└── src/
    ├── lib.rs                         # run_backtest(strategy_def, range) -> BacktestReport
    ├── export.rs                      # read Parquet raw archive → Arrow IPC files in market_simulator format
    ├── run_request.rs                 # build market_simulator RunRequest from strategy def + range + data bindings
    ├── results.rs                     # parse market_simulator TradeRecord stream → BacktestReport domain type
    └── contract.rs                    # typed representations of market_simulator's data contracts (kept in sync
                                       #   with github.com/MHughesDev/market_simulator specs)
```

No replay engine. No fill model. No `available_time` loop. Those all live in market_simulator.
This crate: takes data out of this repo's storage, formats it for market_simulator, calls it,
and translates back.

---

## `crates/mcp-server/` — thin front door

> Targets the canonical strategy JSON; no privileged path; no order-placement tool.
> Source: [08-mcp-server.md](./spec/08-mcp-server.md).

```
crates/mcp-server/
├── Cargo.toml
└── src/
    ├── lib.rs                         # MCP server construction; registers tools
    └── tools/
        ├── mod.rs                     # tool registry
        ├── discovery.rs               # list_lanes, list_instruments
        ├── authoring.rs               # validate_strategy, create_strategy (→ strategy-validator)
        ├── lifecycle.rs               # apply_strategy, stop_strategy, list_strategies
        └── backtest.rs                # run_backtest, get_backtest_result
```

---

## `apps/` — thin binaries (wiring only)

```
apps/
├── platform/
│   ├── Cargo.toml
│   └── src/
│       └── main.rs                    # the monolith: load config+obs, connect bus+storage, mount
│                                      #   api + ui-gateway + strategy-runtime + risk + execution +
│                                      #   reconciliation + demand-manager, serve. (per 01-architecture)
├── collector-crypto/
│   ├── Cargo.toml
│   └── src/main.rs                    # start the crypto collector; publish to the bus; reconnect alone
├── collector-equity/
│   ├── Cargo.toml
│   └── src/main.rs                    # start the equity collector; publish to the bus
└── mcp-server/
    ├── Cargo.toml
    └── src/main.rs                    # start the MCP server process (wraps crates/mcp-server)
```

Note: there is no `backtest-runner` binary in this repo. Backtest runs are submitted via
`POST /api/backtests` (REST), which delegates to `crates/market-simulator-adapter`. There is no
CLI runner because the simulator is a library called from within the platform binary, not a
separate process.

---

## `migrations/` — Postgres schema

```
migrations/
├── 0001_users_accounts.sql            # users, accounts, permissions
├── 0002_instruments.sql               # instrument metadata table (asset_class, precision, hours, …)
├── 0003_orders_fills_positions.sql    # orders, fills, positions + append-only audit/event log
├── 0004_strategies.sql                # strategy definitions + versions
└── 0005_risk_config.sql               # per-user risk limits + global trading_enabled flag
```

## `clickhouse/` — time-series DDL

```
clickhouse/
├── 01_trades.sql                      # trades table; ReplacingMergeTree(ORDER BY instrument,available_time)
├── 02_bars.sql                        # bars table; partition by month; dedup key ordering
└── 03_features.sql                    # features table
```

## `config/` — runtime config

```
config/
├── default.toml                       # base config: ports, watermarks, batch sizes, risk defaults
├── local.toml                         # local-dev overrides (gitignored values via env)
└── lanes.toml                         # lane → partition + retention policy declarations
```

---

## `frontend/` — React SPA (retained, refactored)

> Already migrated from Streamlit to Vite+React (per project memory). Kept and pointed at the new Rust
> API/WS contracts. Source: [06-ui-and-streaming.md](./spec/06-ui-and-streaming.md).

```
frontend/
├── package.json
├── vite.config.ts
├── tsconfig*.json
├── eslint.config.js
├── index.html
├── public/
└── src/
    ├── main.tsx                       # app entry
    ├── App.tsx                        # router + layout shell
    ├── api/
    │   ├── rest.ts                    # typed REST client for /api/* (strategies, orders, backtests…)
    │   └── ws.ts                      # /ws/live subscription client (subscribe/remove panels)
    ├── lib/
    │   ├── types.ts                   # TS mirrors of domain payloads + lane names (kept in sync)
    │   └── format.ts                  # decimal-safe display formatting (no float math on money)
    ├── pages/
    │   ├── Dashboard.tsx              # landing: P&L + win rate + active strategies, broken down by asset class
    │   ├── InstrumentDetail.tsx       # per-asset view: chart + strategy panel + manual trade
    │   └── AccountSettings.tsx        # connected venues, API credential management
    ├── panels/
    │   ├── ChartPanel.tsx             # bars + features subscription (1m OHLCV for MVP)
    │   ├── OrderBookPanel.tsx         # ui.orderbook.snapshot — disabled/placeholder for MVP
    │   ├── TradePanel.tsx             # manual order entry → POST /api/orders
    │   ├── PositionsPanel.tsx         # private positions/balances (per-user scoped)
    │   └── StrategyPanel.tsx          # initialize/stop strategy on THIS instrument; decision log
    ├── builder/                       # visual n8n-style strategy builder (Phase 5)
    │   ├── BuilderCanvas.tsx          # node graph editor
    │   ├── nodes/                     # node components mapping to strategy-def node types
    │   └── serialize.ts               # graph ↔ canonical strategy-definition JSON round-trip
    ├── components/                    # shared UI primitives
    └── state/                         # client state (subscriptions, auth, selected instrument)
```

---

## `tests/` — cross-crate integration

```
tests/
├── README.md                          # how integration tests spin up infra (docker-compose)
├── ingest_to_storage.rs               # collector → bus → storage writer → ClickHouse/Parquet path
├── manual_order_flow.rs               # REST order → risk gate → paper execution → position update
├── strategy_end_to_end.rs             # definition → runtime (single asset) → intent → risk → paper fill
├── backtest_adapter.rs                # archive export → Arrow IPC → market_simulator contract check
├── quarantine_replay.rs               # malformed feed → quarantine → fix → replay → storage
└── reconciliation_halt.rs             # forced divergence halts the instrument
```

Note: `backtest_reproducibility` is tested inside market_simulator, not here. The adapter test
(`backtest_adapter.rs`) verifies the export format and contract compliance, not fill simulation
correctness.

## `xtask/` — dev automation

```
xtask/
├── Cargo.toml
└── src/main.rs                        # `cargo xtask seed`, `xtask gen-fixtures`, `xtask check-money-f64`
```

## `docs/` — the canonical system-design workspace (template structure)

> This is the documentation set the whole repo is structured around — scaffolded in **Phase A** from
> the template at `refactor_reference_docs/template/docs/` and populated by migrating the spec,
> architecture, decisions, and plans into it. It is the design/decision/plan record — **not** where
> system code or runtime config lives. Every later phase is authored and executed from here. See
> [`plans/phase-A-documentation.md`](./plans/phase-A-documentation.md).

```
docs/
├── README.md                          # workspace overview + contents map
├── artifact.md                        # foundational project definition (SC-N success, FM-N failure)
├── open-questions.md                  # living Q-N register: options, status, resolution, evidence
├── architecture.md                    # current-state map — INCLUDES the enumerated repo structure
├── glossary.md                        # shared terms (migrated from spec/12-glossary)
├── adr/                               # Architecture Decision Records (NNNN-title.md, immutable) + index
├── specs/                            # FEAT/COMP/DATA/INTG/SYS specs (<TYPE>-<NNN>) + index
├── research/                         # research briefs (stack evaluation, broker/venue selection) + index
├── plans/                           # Formal plans — the whole refactor (master + phases) lives here + index
├── procedures/                      # atomic, source-of-truth task instructions (template tooling)
└── skills/                          # composed agent workflows referencing procedures (template tooling)
```

Operational docs (stack runbook, the "adding a venue" checklist, the REST/WS API reference) live as
**procedures**/specs inside this workspace (e.g. `docs/procedures/operate-the-stack.md`,
`docs/procedures/add-a-venue.md`, and the `SYS`/`COMP` specs for the API contract) rather than as
loose top-level files — that is the whole point of using the template structure.

## `legacy_python/` — quarantined old system

```
legacy_python/                         # the entire current Python tree moved here at migration start;
                                       #   referenced for behavior parity, deleted in Phase 7.
```

---

## Crate dependency graph (who may import whom)

```
domain  ──────────────────────────────────────────────────────────  (depends on nothing internal)
  ▲  ▲  ▲  ▲  ▲  ▲
  │  │  │  │  │  └── config, observability      (leaf utilities; domain only)
  │  │  │  │  └───── builders, features         (PURE; domain only — NO storage/bus)
  │  │  │  └──────── event-bus, storage         (domain + their backend client)
  │  │  └─────────── collectors                 (domain + event-bus + builders)
  │  └────────────── risk, execution, reconciliation, strategy-validator
  └───────────────── strategy-runtime, demand-manager, venue-router, ui-gateway,
                     market-simulator-adapter, mcp-server, api
                        (compose the above; api is the top of the graph)

apps/*  depend on crates only; contain wiring, no logic.

market_simulator  ←── market-simulator-adapter (external dep; not in this workspace)
```

**Hard rules enforced in review/CI:**
- `builders` and `features` must not list `storage`, `event-bus`, `redis`, `sqlx`, or `clickhouse`
  as dependencies (purity).
- Nothing except `risk` and `execution` may construct an `ApprovedOrder`/submit to a broker
  (single-chokepoint).
- Only `domain::money` defines `Price`/`Size`; no other crate may define a money type.
- `market-simulator-adapter` must not contain fill simulation logic — it is a translation layer only.
