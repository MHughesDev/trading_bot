# FEAT-002: Backtesting System

**Status:** Implemented
**Version:** 1.0
**ADR(s):** ADR-0014, ADR-0007 (frozen strategy v1), ADR-0008 (available-time
ordering), ADR-0012 (canonical bar storage)
**Crate:** `crates/backtest` · **API:** `crates/api/src/routes/backtests.rs`

## 1. Purpose

Defines the backtesting system: replaying a v1.0 strategy definition over
historical bars through the `market_simulator` SDK and reporting fills, PnL, and
return statistics. The platform owns the data, the strategy interpretation, and
the job lifecycle; the simulator owns only execution.

## 2. Scope & Non-Goals

**In scope:**
- Job lifecycle and state machine (`Queued → CheckingData → [CollectingData] →
  LoadingData → Simulating → Completed`, with `Failed`/`Cancelled`).
- Data-requirement derivation from the strategy (timeframe lane + indicator
  warm-up).
- ClickHouse coverage analysis (per-day counts, gaps, trading-schedule and
  holiday calendars).
- Automated historical backfill (Binance for crypto, Alpaca for equities/ETFs)
  with timeouts and bounded retry/backoff.
- The strategy-definition → per-bar callback bridge (`sim.rs`) and its
  rising-edge signal semantics.
- Per-user scoping, pagination, concurrency limiting, and best-effort
  persistence of runs (`backtest_runs`).

**Not in scope (deliberate):**
- The simulator's internal venue/matching models — owned by `market_simulator`
  behind the SDK (ADR-0014).
- Tick/quote replay — bars-only in v1.0 (see §6, deferred).
- Sizing modes beyond `Fixed`, and v1.5 universe nodes — parse-only / deferred
  (see §6).

## 3. Boundary (ADR-0014)

The platform owns everything except execution: strategy definitions (v1.0 JSON,
interpreted by the same `strategy-runtime` evaluator that runs live), indicator
computation (the same pure `features` crate), and bars (ClickHouse
`market_bars`). The simulator is a pure processing engine consumed through the
frozen `nautilus-backtest::sdk` surface; it receives bars + a callback in memory
and returns a result document, persisting nothing. This makes live/replay parity
structural.

## 4. Job lifecycle

```
Queued
  └─ run-permit acquired (≤ MAX_CONCURRENT_RUNS) ───────────────┐
CheckingData   derive requirements; measure ClickHouse coverage │
  └─ gaps && auto_collect ─▶ CollectingData (paged REST backfill)│
LoadingData    load deduplicated bars (argMax over revision)     │
Simulating     run the SDK on a blocking task; poll progress     │
Completed / Failed / Cancelled                                   │
```

A `SimulationControl` block carries progress and cancellation between the async
manager and the blocking simulation. Cancellation is honoured at every phase
boundary, including while a job waits for a run permit.

**Restart behavior (deliberate):** runs do **not** survive a platform restart.
On boot, rows still marked active are surfaced as `Failed` with
"interrupted by platform restart". Resuming `Queued` jobs is deferred until
there is demand (ADR-0014 §5).

## 5. Correctness & safety invariants

- **No hand-built SQL.** Bar inserts use the `clickhouse` crate's typed
  RowBinary path; reads use bound parameters. `numeric()` remains as a
  defense-in-depth assertion on the collector boundary.
- **No panics on malformed market data.** All nautilus value construction
  (`Price`/`Quantity`/`Money`) goes through the checked `from_str` path and
  returns errors rather than panicking inside `spawn_blocking`.
- **Real precision.** Price/size precision come from the instrument's tick/lot
  metadata when known (supporting 0-dp through 9-dp instruments), falling back
  to data-inferred precision only when metadata is absent.
- **Exact market mapping.** Collector symbols are never silently re-quoted
  (`BTC-USD` is not proxied to `BTCUSDT`); unsupported asset-class/timeframe
  combinations are rejected at create time (422), not mid-job.
- **Per-user isolation.** Runs are scoped to their creator; list/get/stop/delete
  filter by the authenticated identity and never leak another user's runs.
- **Bounded resource use.** A semaphore caps concurrent heavy phases; the list
  endpoint pages; collectors have connect/request timeouts and bounded
  exponential backoff.

## 6. Deferred (documented surface)

- **Tick/quote replay** (bars-only today) — would feed `QuoteTick`/`TradeTick`
  from `market_trades` through an SDK intake helper.
- **Sizing modes** beyond `Fixed` (`PercentOfBalance`, `RiskUnit` are
  parse-only) and **v1.5 universe nodes** (Rank/Filter/TakeTopN/DataSource).
- **Stored-feature replay** — feeding versioned `market_features` values at
  their `available_time` instead of recomputing indicators during replay.

## 7. Adversarial tests (Invariant 8)

Each mechanism carries at least one test that asserts it does the right thing on
hostile or boundary input, co-located with the code:

| Mechanism | Adversarial / boundary test | Location |
|-----------|-----------------------------|----------|
| Typed insert / numeric guard | rejects `1); DROP TABLE …`, `1e5`, empty | `store.rs::numeric_accepts_decimals_and_rejects_injection` |
| Value construction | garbage input errors instead of panicking | `sim.rs::value_constructors_reject_malformed_input_without_panicking` |
| Precision | 0-dp and 8-dp increments format correctly | `sim.rs::increment_str_supports_zero_dp_and_crypto_precision` |
| Symbol mapping | `BTC-USD` is **not** rewritten to `BTCUSDT` | `collect.rs::binance_symbol_mapping` |
| Auto-collect support | unsupported class is a clear error | `collect.rs::unsupported_asset_class_is_a_clear_error` |
| Gap merge over-reach | a present day between two gaps is not swallowed | `gaps.rs::continuous_market_does_not_swallow_a_present_day` |
| Holiday calendar | a weekday market holiday isn't counted/collected | `gaps.rs::session_market_skips_holidays` |
| Signal semantics | one rising-edge crossover ⇒ exactly one order | `sim.rs::ema_cross_over_rising_bars_places_one_order` |

## 8. Traceability

- ADR-0014 (boundary, pinned SDK, restart decision).
- Concern map: `plans/set-D/MASTER.md` (#1–#29 → phase · task).
- DATA-004 (strategy definition format), DATA / ADR-0012 (bar storage).
