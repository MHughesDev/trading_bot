# Backtest Suite — Make It Real & Unify — Set K

**Completion: 0% (plan only)**

**Status:** PLANNED. Set J shipped the honest-evaluation *core* (Run/Study/
Experiment + Null Library + staged-gate funnel + DSR/PBO + reconciliation) as
tested pure compute, plus a REST/WS surface and three React surfaces. But the
suite manager runs a **deterministic synthetic executor over in-memory stores**,
and the testing UX is **split across three pages**. Set K closes the two
explicitly-deferred Set-J live legs (the `market_simulator` `SimRunExecutor` and
the Postgres/ClickHouse stores), adds **parallel execution**, and folds the three
surfaces into **one unified workspace**.

**Created:** 2026-06-18
**Builds on:** Set J (`docs/plans/plan-sets/set-J/MASTER.md`)
**Scope class:** Completion + consolidation — wiring tested primitives to reality,
not new statistical machinery.

**Locked direction (2026-06-18):**
- **Foundation:** build on the Set-J honest-eval core (do not rewrite the stats).
- **UX:** one unified workspace — fold `/backtesting`, `/proving-ground`,
  `/workbench` into a single surface and retire the legacy pages.

---

## 1. Why Set K

The statistical philosophy from Masters (*Permutation and Randomization Tests for
Trading System Development*), López de Prado (*Advances in Financial ML* — CPCV,
Deflated Sharpe), Aronson (detrending / data-mining bias), and Bailey et al.
(Probability of Backtest Overfitting) is **already implemented and tested** in
`crates/backtest`:

| Principle | Where it lives today |
|---|---|
| Permutation / randomization nulls | `nulls/generators.rs` — 7 generators, each with a preserve/destroy property test |
| Selection-bias correction | `gates/` Gate 3 — Šidák correction driven by the irreversible trial counter |
| Sealed return/drawdown distributions | `study/result.rs` — median / IQR / worst-5% / spread, **no** best-member API (INV-2) |
| CPCV / purged evaluation | `study/` CPCV kind with disjoint-partition tests |
| Deflated Sharpe Ratio | `stats/` — PSR + expected-max-Gaussian |
| Probability of Backtest Overfitting | `stats/` — PBO via CSCV |
| Falsification by construction | `experiment/` one-shot holdout vault + `gates/` staged Gate 0→4 funnel |

**The gap is not the math — it is that the math runs on fake inputs in volatile
memory, fragmented across three pages.** Two of the four gaps are exactly the
user's stated asks: *run multiple tests at once* and *view tests + stats in one
place*.

### The two parallel worlds today

| Surface | Real execution? | Honest-eval? | Persisted? |
|---|---|---|---|
| `/backtesting` (`BackTestingPage`) | ✅ real `market_simulator` runs on real bars | ❌ none | ✅ (legacy `/api/backtests`) |
| `/proving-ground` + `/workbench` (Set J) | ❌ synthetic hash-curve executor | ✅ full apparatus | ❌ in-memory `RwLock<HashMap>` |

Set K makes the right column real, persistent, parallel, and singular.

---

## 2. Scope

### In scope
- **A — Real execution:** the `SimRunExecutor` live leg (Set J J-0.6). Wire the
  honest-eval core to the actual `market_simulator` so Runs are real backtests.
- **B — Persistence:** the Postgres/ClickHouse store live legs (Set J J-0.7/J-1.9).
  Schema already exists (migrations 0026–0030, ClickHouse `05`).
- **C — Parallel execution:** concurrent Runs within a Study and concurrent
  Studies/Experiments, with a bounded scheduler and a `run_id` result cache.
- **D — Unified workspace:** one surface for launch + live fleet + sealed results
  + significance + gates + vault + calibration. Retire the legacy pages.

### Out of scope
- New statistical methods (the Set-J core is the contract; no new null kinds,
  gates, or significance math unless a real-data gap forces one — see A-3).
- Data ingestion, strategy authoring, live order placement (unchanged from Set J §2).
- The model suite (Set H/I) and capital allocation.

---

## 3. Phases

Build order is A → B → C → D, but B and C can overlap once A lands, and D's
frontend consolidation can begin against each contract as it freezes.

### Phase A — Make it real (the `SimRunExecutor`)

The honest-eval core takes a `RunExecutor` (`crates/backtest/src/run/executor.rs`).
Today `SuiteManager::new()` injects `ClosureExecutor(synthetic_execute)`. Phase A
replaces it with a real one.

| Task | Description | State |
|---|---|---|
| **A-1** | `run_simulation_detailed` (drive `BacktestEngine`, extract closed positions → trades + reconstructed equity) + `map_detailed_result` (→ standardized `RunResult`). | ✅ **MERGED** (PR #243) |
| **A-2** | `RunConfig → SimulationInputs` resolver — mirror `manager.rs::drive_inner`: resolve the pinned strategy definition (by version hash), instrument metadata via `storage::postgres::instruments::fetch_by_id`, and bars via `BarStore` + `aggregate_bars`. | **next** |
| **A-3** | `apply_params(definition, &ParamMap)` helper — **data gap**: today params only feed the `run_id` hash; there is no way to vary a strategy's actual parameters in a real sim. Required for the `parameter_sweep` and `neighborhood` study kinds. (seeds / window / cost-sweep studies are unaffected.) | **next** |
| **A-4** | `SimRunExecutor: RunExecutor` wrapping A-2 + A-1 + A-3; use `tokio::task::block_in_place` on the multi-thread runtime so **no api-crate change** is needed. Inject into `SuiteManager::new()` in place of `synthetic_execute`; keep `synthetic_execute` as an explicit test backend. | **next** |

**Verification boundary:** A-2/A-3/A-4 are compile- and unit-verifiable in the
sandbox, but a true end-to-end run needs live Postgres + ClickHouse. End-to-end
coverage lands as a **gated integration test** (`#[ignore]` without DB env), same
discipline as the existing backtest manager tests.

### Phase B — Persist it (Postgres + ClickHouse stores)

The schema is already migrated; only the Rust store impls and the manager wiring
are missing.

| Task | Description |
|---|---|
| **B-1** | Postgres-backed `RunStore` / `StudyStore` / `ExperimentStore` / `NullStore` / `GateStore` — system of record for experiments, studies, run metadata, the trial counter, the vault access log, lifecycle, and the null registry. Migrations **0026–0030** already define the tables (and 0028's triggers already enforce counter monotonicity + no-unspend). |
| **B-2** | ClickHouse-backed equity curves / trade lists / metric-distribution series (high-volume, append-only). DDL **`clickhouse/05_backtest_run_series.sql`** already exists. |
| **B-3** | `SuiteManager` reads/writes through the stores instead of `RwLock<HashMap>`; experiments + trial counts **survive restart**. Keep the in-memory stores as the test backend. |

### Phase C — Run multiple at once (parallel execution)

The Run atom is pure and content-addressed by `run_id` (Set J ADR-0019) — it is
*designed* to parallelize and cache. Today the server serializes execution; the
`FleetBoard` already renders N concurrent rows in anticipation.

| Task | Description |
|---|---|
| **C-1** | Parallel Run fan-out **within** a Study — execute a Study's members on a bounded worker pool; the sealed distribution is order-independent so concurrency is safe. |
| **C-2** | Concurrent **Studies / Experiments** — a bounded scheduler replaces serialized execution; respect per-user fairness and a global concurrency cap. |
| **C-3** | `run_id` **result cache** (Redis hot map per Set J D-11, falling back to the Run store) so overlapping sweeps and re-runs dedupe instead of recomputing. |
| **C-4** | Per-run / per-study progress multiplexed over `/ws/backtest-suite` so the FleetBoard shows true parallelism, not one serialized row. |

### Phase D — One unified workspace

Fold the three surfaces into one. The honest-eval components already exist and are
shared between Proving Ground and Workbench (`components/workbench/*`); the work is
information architecture + absorbing the legacy single-run path, not rebuilding panels.

| Task | Description |
|---|---|
| **D-1** | One page, four zones: **Launch** (strategy · instrument · window · cost model · study kind · null picker), **Fleet** (live parallel execution), **Results** (sealed distributions · significance card · gate funnel · vault · reconciliation), **Calibration** (suite meta-view). |
| **D-2** | Absorb `BackTestingPage`'s real single-run capability as a "quick run" (a 1-member study) inside the unified launcher, so ad-hoc runs and rigorous experiments share one engine and one results view. |
| **D-3** | Retire `/backtesting` and `/workbench`; the unified workspace becomes the home. Update `App.tsx` routes + `GlassPillNav.tsx` (one nav entry replacing three). |
| **D-4** | Converge the two API clients — migrate the legacy `api/backtests.ts` callers onto the suite API (`api/experiments.ts`), or keep a thin `quick-run` endpoint that creates a 1-member study. |

---

## 4. Cross-cutting (inherited from Set J — non-negotiable)

1. **The three invariants stay structural.** INV-1 skeptical defaults + `unsafe`
   flag; INV-2 sealed distributions (no best-member); INV-3 significance carries
   its null + trial count or renders nothing. Real execution must not introduce a
   bare-p or argmax path.
2. **The Run stays dumb; the Study stays smart.** Parallelism and caching live in
   the executor/scheduler, not in the Run contract.
3. **Nothing is deleted; every Run counts.** Real failed runs still increment the
   trial counter and are written immutably.
4. **`check-money-f64` stays green** — costs are `Decimal`, metrics/p-values are
   `f64`; the real executor constructs no new `Price`/`Size` outside the simulator.
5. **Adversarial test per mechanism** — every new seam (resolver, param-apply,
   parallel scheduler, store) ships with its test; DB-dependent paths get a gated
   integration test.

---

## 5. Numbering (verify before locking)

- ADRs: Set J used 0019–0021 → next free **0022** (candidate: "Real-execution
  wiring + parallel scheduler + store-backed suite").
- Postgres migrations: Set J used 0026–0030 → next free **0031** (only if a new
  table/column is needed; the existing schema may suffice).
- ClickHouse DDL: Set J used `05` → next free **`06`**.

---

## 6. Immediate next step

**Phase A-2 + A-4** — re-implement the `SimRunExecutor` wiring (the work that was
started but never committed in the PR #243 session). A-1 (`run_simulation_detailed`
+ `map_detailed_result`) is already merged, so the next commit is the resolver +
the executor injection, compile-verified here with the end-to-end run deferred to
a gated integration test.

---

## 7. Progress Log

| Date | Phase | Task | Note |
|------|-------|------|------|
| 2026-06-18 | — | plan | Set K created. Builds on Set J. Four phases: A real execution (`SimRunExecutor`), B persistence (Pg/CH stores), C parallel execution, D unified workspace. Foundation = build-on-Set-J; UX = one unified workspace (legacy pages retired). A-1 already merged (PR #243). |
