# Phase 0 — The Run atom & immutable run store

**Completion: 0% (0 / 10 tasks)**

**Goal:** Make the smallest reproducible unit — the **Run** — a pure, dumb,
content-addressed function `RunConfig → RunResult`, and persist every execution
immutably (failures included). Wrap the existing `BacktestManager` as the Run
*executor* behind this contract so the engine stays unchanged while the suite
gains caching, reproducibility, and a trustworthy substrate for trial counting.
Establish **INV-1** here: costs, the (Phase-2) trial counter, and the holdout
lock are on by default, and disabling any sets `unsafe=true` permanently.

**Depends on:** `crates/backtest` (sim, manager, store, aggregate), `crates/domain`
(`StrategyDefinition`, `Timeframe`, the 4-timestamp model), `crates/storage`.
**Blocks:** Phases 1–5 (every Study, gate, and reconciliation run is made of Runs).

---

## Design notes

**The Run is dumb (spec ADR-001).** It is a pure function of its full config and
knows nothing about cross-validation, nulls, or trial counting. All intelligence
about *how runs combine* lives one level up (Phase 1+). This keeps the atom
cacheable and trivially parallelizable — which matters because the funnel will
spawn millions.

**`run_id` is the spine of trust.** It is the deterministic hash of the *entire*
`RunConfig`. Two configs that differ in any field produce different `run_id`s;
two identical configs produce the same `run_id` and may be served from cache.
This is what makes the trial counter trustworthy — you cannot accidentally
re-count a cached identical run, and you cannot silently mutate a run.

**Contracts (frozen shape, `crates/backtest/src/run/mod.rs`):**

```rust
pub struct RunConfig {
    pub run_id: RunId,                  // = hash of all fields below (computed, not set)
    pub strategy_ref: StrategyRef,      // pinned by version hash (ADR-0007)
    pub strategy_version: Hash,
    pub params: ParamMap,               // the specific parameter set for THIS run
    pub data_slice: DataSlice,          // universe, [start,end], base 1m, eval res, construction
    pub cost_model_ref: CostModelRef,   // commission/slippage/spread/borrow/latency
    pub fill_model: FillModel,          // next_bar_open | current_close | limit_prob | pessimistic_intrabar
    pub sizing_ref: SizingRef,
    pub seed: u64,
    pub data_snapshot: Hash,            // point-in-time data version → reproducibility
    pub unsafe_flags: UnsafeFlags,      // INV-1: which default protection (if any) was disabled
}

pub struct RunResult {
    pub run_id: RunId,                  // matches the producing config
    pub status: RunStatus,              // Ok | Failed | RejectedIntegrity
    pub equity_curve: Series<f64>,
    pub positions: Series<PositionMap>,
    pub trades: Vec<Trade>,             // entry/exit, MAE, MFE, holding period, costs paid
    pub metrics: MetricSet,
    pub integrity_flags: Vec<Flag>,     // leakage/lookahead/cost-sanity findings (Gate 0 writes here)
    pub compute_cost: ComputeCost,      // { wall_ms, cpu_ms } for funnel budgeting
    pub produced_at: DateTime<Utc>,
    pub produced_by: Hash,              // engine version
}
```

`DataSlice` carries `base_resolution = 1m` always, `eval_resolution ∈
{1m,5m,10m,15m,30m,1h,1d}`, and `construction = close_stamped` (the forming-bar-safe
assembly `aggregate_bars` already produces). `MetricSet` is standardized: every
Run produces the same shape (return / risk / activity blocks) plus the **honesty
hooks** `trial_count_at_eval` and `is_oos_gap`, both `null` at the Run level and
populated only by Studies (§1.2). All metric/score fields are `f64`; `costs paid`
on a `Trade` is `Decimal` (D-10).

**The executor adapter.** `BacktestManager` keeps its `BacktestRequest`→
`BacktestSnapshot` path; Set J adds `RunExecutor::execute(RunConfig) -> RunResult`
that translates a `RunConfig` into the manager's inputs, runs the simulator once,
and maps the output into `RunResult`. No simulator change. The existing job UI
keeps working through a thin `RunConfig`-from-`BacktestRequest` adapter.

---

## Tasks

### ☐ J-0.1 Author ADR-0019 (Run/Study/Experiment object model + sealed distributions) — S
Write `docs/adr/0019-run-study-experiment-object-model.md` (Context / Decision /
Rationale / Consequences / Alternatives). Fold in the spec's ADR-001 (the Run is a
pure dumb function) and ADR-002 (distributions are sealed). Record the `run_id`
content-hash rule, immutability, and that failures are logged and counted. Mark
Accepted; link from `docs/adr/README.md` and Set J MASTER §9.
**Acceptance:** ADR-0019 exists, indexed, and cited by Phase 0 + Phase 1 tasks.

### ☐ J-0.2 `RunConfig` / `DataSlice` / enums — M
Add `crates/backtest/src/run/config.rs` with `RunConfig`, `DataSlice`,
`FillModel`, `Construction { CloseStamped }`, `EvalResolution`, and the `*Ref`
newtypes. All fields `serde`-round-trippable. `base_resolution` is a fixed `1m`
marker type (it is *always* 1m). No `run_id` is accepted from the caller — it is
computed (J-0.3).
**Acceptance:** `cargo test -p backtest` round-trips a config; constructing one
with a caller-supplied `run_id` is impossible by type (builder computes it).

### ☐ J-0.3 Deterministic `run_id` content hash — M
Implement `RunConfig::compute_id() -> RunId` = a stable hash (SHA-256 over a
canonical, field-tagged encoding) of *every* config field except `run_id`
itself. Order-independent for maps (`params`), order-sensitive for sequences.
Property test: any single-field mutation changes the id; re-serializing an
identical config yields the identical id; `params` key reordering does **not**.
**Acceptance:** unit + property tests green; two byte-different-but-semantically-identical
configs collide, two semantically-different configs do not.

### ☐ J-0.4 `MetricSet` (standardized shape + honesty hooks) — M
Add `crates/backtest/src/run/metrics.rs` with `MetricSet`: return block (cagr,
total_return, ann_vol, sharpe, sortino, calmar, information_ratio, alpha, beta,
**detrended_sharpe**), risk block (max_drawdown, avg_drawdown,
time_in_drawdown_pct, cvar_95, ulcer_index), activity block (turnover,
exposure_gross/net, hit_rate, profit_factor, n_trades), and honesty hooks
`trial_count_at_eval: Option<i64>` + `is_oos_gap: Option<f64>` (both `None` at Run
level). `detrended_sharpe` is net of market drift + average-position bias
(Aronson). All `f64` (D-10).
**Acceptance:** every field present; `MetricSet::compute(equity, trades, benchmark)`
unit-tested against a hand-computed fixture; honesty hooks default `None`.

### ☐ J-0.5 `Trade` + `RunResult` — M
Add `Trade { entry, exit, side, mae, mfe, holding_period, costs_paid: Decimal }`
and `RunResult` (J design-notes shape) with `RunStatus { Ok, Failed,
RejectedIntegrity }`. `integrity_flags: Vec<Flag>` is empty at execution and
filled by Gate 0 (Phase 4). `costs_paid` is `Decimal`; all metrics `f64`.
**Acceptance:** `cargo test -p backtest` round-trips a `RunResult`;
`check-money-f64` stays green (no new `Price`/`Size` constructed outside the sim).

### ☐ J-0.6 `RunExecutor` over `BacktestManager` — L
Add `crates/backtest/src/run/executor.rs`: `RunExecutor::execute(&self, cfg:
&RunConfig) -> RunResult`. Translate `RunConfig` → the manager's existing inputs
(resolve `strategy_ref`@version, load bars via `BarStore` at `data_snapshot`,
apply `cost_model_ref`/`fill_model`/`sizing_ref`), run the simulator once, map
results into `RunResult`. A panicking/erroring run yields `status = Failed` with
a populated reason — **never** a lost run. No simulator change.
**Acceptance:** an integration test executes a known strategy and asserts the
`RunResult` equity curve matches the legacy `BacktestSnapshot` path bit-for-bit;
a deliberately failing config returns `Failed`, not an `Err` that drops the run.

### ☐ J-0.7 Immutable, content-addressed `RunStore` — L
Add `crates/backtest/src/run/store.rs` + Postgres migration **0026**
(`backtest_runs`: `run_id PK`, full config JSON, full result JSON, status,
engine_version, produced_at, `unsafe`) and ClickHouse DDL **05**
(`backtest_run_series`: equity/positions/trades, append-only). `put(result)` is
idempotent on `run_id`; a second `put` of the same id is a no-op (never an
update). `get(run_id)` serves the cached result. **Failed and rejected runs are
stored too.** Nothing is ever deleted or mutated.
**Acceptance:** storing the same `run_id` twice leaves one row; a `Failed` run is
queryable; an attempted update to an existing row is rejected at the store API.

### ☐ J-0.8 Cache-aware `run()` entry point — M
Add `Backtest::run(cfg: RunConfig) -> RunResult`: compute `run_id`; if present in
`RunStore`, return the cached result (a **cache hit**, no execution, no
re-count); else `RunExecutor::execute` then `RunStore::put`. Expose a
`run_id → RunResult` Redis hot map (D-11) in front of Postgres. This is the
single funnel-facing entry; Studies (Phase 1) call only this.
**Acceptance:** running an identical config twice executes once (second is a cache
hit, asserted via `compute_cost`/an exec counter); differing configs execute
independently.

### ☐ J-0.9 INV-1 skeptical defaults + `unsafe` flag — M
Add `UnsafeFlags { costs_disabled, counter_disabled, holdout_unlocked }`, all
**false by default**. Building a `RunConfig` with a zero/null cost model, or any
path that disables the counter or unlocks the holdout, requires explicitly
setting the corresponding flag, which sets `RunConfig.unsafe = true`. The
`unsafe` bit propagates to `RunResult`, the `RunStore` row, and (Phase 1+) the
Study and Experiment. It never clears.
**Acceptance:** a default config has `unsafe = false` and a non-trivial cost
floor; disabling costs flips `unsafe = true` and the bit survives a store
round-trip; a test asserts `unsafe` cannot be cleared once set.

### ☐ J-0.10 Run provenance & engine-version stamping — S
Stamp `produced_by` (engine version = a hash of the simulator SDK rev + backtest
crate version) and `produced_at` on every `RunResult`, and persist
`compute_cost { wall_ms, cpu_ms }` for funnel budgeting (Phase 4). Add a
`backtest::run::ENGINE_VERSION` constant derived at build time.
**Acceptance:** two runs from the same engine share `produced_by`; a unit test
asserts `compute_cost` is populated and monotonic wall-time is recorded.

---

## Exit criteria

- `Backtest::run(RunConfig) -> RunResult` is the single, cache-aware, immutable
  Run entry point; identical configs collide on `run_id` and serve from cache.
- Every execution — `Ok`/`Failed`/`RejectedIntegrity` — is stored and never
  deleted or mutated.
- `MetricSet` is standardized with `null` honesty hooks at the Run level.
- INV-1 holds: costs + counter + holdout default on; disabling any sets a
  permanent `unsafe` flag.
- ADR-0019 Accepted and indexed. `cargo test -p backtest` green.
