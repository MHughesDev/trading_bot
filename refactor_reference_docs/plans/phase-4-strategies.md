# Phase 4 — Strategies (feature engine + runtime + market_simulator adapter + multi-instance)

> **Self-contained execution doc.** You need only: this file, [`../file-structure.md`](../file-structure.md),
> and the specs — especially [`04-strategy-system.md`](../spec/04-strategy-system.md),
> [`03-data-engineering.md`](../spec/03-data-engineering.md) §5–§6, and
> [`07-storage-and-replay.md`](../spec/07-storage-and-replay.md).

## Phase goal

After this phase, strategies run: a pure **feature engine** computes a small versioned indicator
set; a **strategy runtime** loads a frozen-format definition, declares demand, consumes
**canonical** events, maintains a `WorldState`, and emits order **intents through the Phase 2
risk gate**; a user can click an asset, select a strategy definition, and click "Initialize" to
start a runtime instance bound to that instrument; the **market_simulator adapter** exports
archived OHLCV data as Arrow IPC and submits Run Requests to
`github.com/MHughesDev/market_simulator`, returning backtest results via the REST API; and
multiple strategy instances run concurrently (one per instrument per user) while manual trading
continues.

**Key constraint:** This repo does NOT build a fill simulator or replay engine. Backtest fill
simulation is owned entirely by `market_simulator`.

## Prerequisites

- Phase 0 (frozen strategy format, payloads), Phase 1 (bus, builders, storage/raw archive), Phase 2
  (risk gate + paper execution) complete. Phase 3 (UI) helps observe but is not required.
- `legacy_python/strategies/`, `legacy_python/backtesting/`, and
  `legacy_python/training_pipeline/` contain prior strategy/replay behavior — read for parity.

## Invariants this phase must respect

- **Same builder/feature code live and in replay.** The runtime consumes canonical bus events live;
  replay feeds the *same* pure `builders`/`features` from the raw archive. No second implementation.
- **`available_time` ordering = no lookahead.** The replay loop dequeues strictly by `available_time`;
  a strategy can never be handed an event before its `available_time`. This is structural.
- **No wall-clock reads in strategies.** Strategies read `world.now()` (real live, simulated in
  replay). Any RNG seed is part of the definition and recorded.
- **Intents go through the risk gate.** The runtime has no private broker path; it emits intents that
  hit `crates/risk` exactly like manual orders.
- **Features are pure and versioned.** Feature values are versioned and recorded so replay sees the
  exact values live saw.

---

## Tasks

### P4-T01 — Feature engine (pure, versioned)
- **Goal:** A small indicator set as pure functions that stamp their own `available_time` processing
  delay, publishing `features.technical`.
- **Files:** `crates/features/src/{lib,ema,rsi,window}.rs`,
  `crates/features/tests/features_versioned.rs`; a live adapter (bus I/O) wired in `apps/platform`.
- **Context:** EMA(n), RSI(n) over bars, built on a shared rolling `window`. Per
  [`../spec/03-data-engineering.md`](../spec/03-data-engineering.md) §5–§6 and
  [`../spec/04-strategy-system.md`](../spec/04-strategy-system.md) §determinism: pure, no I/O;
  feature values **versioned** (a `feature_version`) and recorded; each feature stamps
  `available_time` to include its processing delay so live and replay agree. Float math is acceptable
  for *feature values* because they are versioned and recorded.
- **Acceptance:** `features_versioned` proves the same bar stream yields identical feature values
  across runs and that values carry a version; `crates/features` has no I/O-crate deps (`cargo tree`).
- **Depends on:** Phase 1 (bars on the bus, storage for `features` table).

### P4-T02 — WorldState / WorldContext
- **Goal:** The local view a strategy reads so it never manually joins across tables.
- **Files:** `crates/strategy-runtime/src/{world,clock}.rs`.
- **Context:** Per [`../spec/04-strategy-system.md`](../spec/04-strategy-system.md) §runtime:
  `world.now()`, `latest_bar(instrument, timeframe)`, `latest_orderbook(instrument)`,
  `feature(instrument, name)`, `recent_events(instrument, Duration)`, `position(instrument)`,
  `open_orders(instrument)`, `place_order(req)` → risk gate → execution. `clock.rs` is real live,
  simulated in replay. WorldState is updated from incoming events.
- **Acceptance:** unit test: WorldState updates from a sequence of events; `world.now()` returns the
  simulated clock under a replay clock and real time live; `place_order` routes to the gate (mock).
- **Depends on:** Phase 0, Phase 2 (gate), P4-T01.

### P4-T03 — Definition interpreter + runtime loop
- **Goal:** Load a frozen-format definition, declare demand, subscribe canonical events, evaluate the
  node graph, emit intents.
- **Files:** `crates/strategy-runtime/src/{lib,runtime,interpreter,intents}.rs`,
  `crates/strategy-runtime/tests/{no_wallclock,replay_determinism}.rs`.
- **Context:** Per [`../spec/04-strategy-system.md`](../spec/04-strategy-system.md): `runtime.rs`
  loads a `StrategyDefinition`, declares demand to the Demand Manager for its `inputs`
  (expanding `$each` across `asset_universe`), subscribes to **canonical** bus lanes (never the UI
  feed), and on each event calls the strategy. `interpreter.rs` evaluates the node graph /
  `condition.expr` per the grammar frozen in Phase 0 and produces signals; `intents.rs` turns
  actions into `OrderIntent`s (with idempotency keys) routed through the risk gate. Implement the
  `Strategy` trait (`on_event(&mut self, &WorldEvent, &mut WorldContext) -> StrategyResult`).
- **Acceptance:** `no_wallclock` proves the runtime uses `world.now()` not system time;
  `replay_determinism` proves the same event sequence yields identical decisions; an EMA-cross
  definition (from the spec example) runs on one asset and emits an intent that hits the gate.
- **Depends on:** P4-T02, Phase 3 (Demand Manager) or a direct-demand fallback.

### P4-T04 — market_simulator adapter
- **Goal:** Export archived OHLCV data as Arrow IPC and submit backtest Run Requests to
  `github.com/MHughesDev/market_simulator`, returning results through the REST API.
- **Files:** `crates/market-simulator-adapter/src/{lib,export,run_request,results,contract}.rs`,
  `tests/backtest_adapter.rs`.
- **Context:** Per [`../spec/07-storage-and-replay.md`](../spec/07-storage-and-replay.md):
  - `export.rs`: read raw archive (Parquet/ClickHouse) for the requested instruments/lanes/range;
    serialize to Apache Arrow IPC files matching market_simulator's data contract. For MVP, export
    1-minute OHLCV `BarPayload` events — this is what Coinbase and Alpaca provide reliably.
  - `run_request.rs`: build the market_simulator `RunRequest` JSON from the strategy definition,
    date range, starting capital, and Arrow IPC data bindings.
  - `results.rs`: parse `TradeRecord` stream and aggregate metrics returned by market_simulator
    into this repo's `BacktestReport` domain type.
  - `contract.rs`: typed Rust structs mirroring market_simulator's contracts (kept in sync with
    `github.com/MHughesDev/market_simulator` specs). These are data-transfer types only —
    no logic.
  - **No replay engine. No fill model.** The adapter does not run a simulation loop. It translates
    data in, calls the library, translates results out.
  - market_simulator currently supports order-book-style assets (Engine A — equities, crypto spot
    CEX). The adapter targets this scope for MVP. When market_simulator adds more engines, extend
    `contract.rs` and `export.rs` to match.
- **Acceptance:** `backtest_adapter.rs` proves: the exported Arrow IPC matches market_simulator's
  data contract schema; a round-trip (export → call → parse results) succeeds against a
  market_simulator test fixture; the adapter rejects a strategy that requires data granularity
  finer than what is archived.
- **Depends on:** Phase 1 (raw archive), Phase 0 (strategy definition + domain types).

### P4-T05 — Backtest REST endpoints
- **Goal:** Submit and fetch backtests over REST; delegate to the market_simulator adapter.
- **Files:** `crates/api/src/routes/backtests.rs` (+ register in `routes/mod.rs`).
- **Context:** `POST /api/backtests` (strategy def + time range → async job → market_simulator
  adapter call), `GET /api/backtests/{id}` (BacktestReport: metrics, PnL, risk) per
  [`../spec/06-ui-and-streaming.md`](../spec/06-ui-and-streaming.md).
- **Acceptance:** a definition + range submitted via REST triggers the adapter, runs against
  market_simulator, and returns a BacktestReport; an unsupported asset class or missing data
  returns a structured error.
- **Depends on:** P4-T04.

### P4-T06 — Multi-instance runtime (one per instrument per user)
- **Goal:** Multiple strategy instances active simultaneously — one per initialized instrument per
  user — each with its own WorldState, all funneling intents through the single risk gate.
- **Files:** extend `crates/strategy-runtime/src/runtime.rs` (instance manager keyed by
  `(user_id, instrument_id)`); wiring in `apps/platform/src/main.rs`.
- **Context:** Per [`../spec/04-strategy-system.md`](../spec/04-strategy-system.md) §asset model:
  a user initializes a strategy on an asset → one runtime instance is created bound to that
  instrument. Multiple users can each have strategies on the same instrument (separate instances).
  The Demand Manager deduplicates overlapping lane demand. Manual orders on the same instrument
  still work — they share the risk gate.
- **Acceptance:** two users each initialize a strategy on BTC-USDT; both instances run
  independently with their own WorldState; intents from both route through the gate; manual orders
  on BTC-USDT still work; Demand Manager keeps one pipeline for the shared lane.
- **Depends on:** P4-T03, Phase 3 (Demand Manager).

### P4-T07 — Strategy + adapter integration tests
- **Goal:** Prove the live strategy path and the backtest adapter contract.
- **Files:** `tests/strategy_end_to_end.rs`, `tests/backtest_adapter.rs`.
- **Context:** `strategy_end_to_end`: definition → initialize on instrument → runtime → intent →
  risk gate → paper fill → position update. `backtest_adapter`: archive export → Arrow IPC
  format validation → market_simulator contract compliance (does not need market_simulator to
  actually run; validates the contract boundary).
- **Acceptance:** both pass against compose infra.
- **Depends on:** P4-T05, P4-T06.

---

## Phase exit criteria

- [ ] `crates/{features,strategy-runtime,market-simulator-adapter}` implemented; `features` is
      pure/versioned with no I/O-crate deps.
- [ ] A frozen-format definition runs live: user clicks an asset → Initialize → runtime instance
      starts bound to that instrument → emits intents only through the risk gate → never reads
      wall-clock or the UI feed.
- [ ] market_simulator adapter: exports 1m OHLCV from the Parquet archive as Arrow IPC matching
      market_simulator's contract; `backtest_adapter.rs` passes.
- [ ] Multiple strategy instances (one per instrument per user) run concurrently; manual trading
      coexists; Demand Manager deduplicates lane demand.
- [ ] `POST/GET /api/backtests` work and delegate to the adapter; `tests/strategy_end_to_end.rs`
      and `tests/backtest_adapter.rs` pass.
