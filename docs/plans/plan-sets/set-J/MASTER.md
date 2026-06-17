# Backtest Suite — Honest-Evaluation Core — Set J

**Completion: 34% (20 / 59 primary tasks)**

**Status:** IN PROGRESS — Phases 0 & 1 shipped (object model + sealed Study
layer, in `crates/backtest/src/run/` and `crates/backtest/src/study/`; 97 tests
green; ADR-0019 Accepted). Phases 2–5 (Experiment/counter/vault, Null Library,
staged gates, reconciliation/UX) remain. 6 phases (0–5) total. Object model
(Run/Study/Experiment) + Null Library + staged-gate funnel, layered on the
existing `crates/backtest` Run engine. End-state design.
**Created:** 2026-06-17
**Scope class:** End-state architecture (NOT an MVP cut — every subsystem is
specified at full fidelity; phases are build-ordering, not feature-gating).

---

## 1. Overview

The repo already has a **Run engine**: `crates/backtest` runs a strategy
definition against historical bars through the `market_simulator` SDK and
returns an equity curve, trades, and metrics (`backtest::sim`,
`backtest::manager::BacktestManager`, `backtest::store::BarStore`,
`backtest::aggregate::aggregate_bars`). It can execute *one* backtest honestly.
What it cannot do is stop a researcher from fooling themselves: there is no
trial counting, no sealed distributions, no held-out vault, no null model, and
no gate sequence. A user can run 500 variations, remember the 3 that worked, and
report those as if they were the only attempts.

Set J builds the **honest-evaluation core** on top of that engine — the
[Backtest Suite Core Spec](../../../specs/BACKTEST_SUITE_CORE_SPEC.md). It adds
two load-bearing subsystems:

1. **The object model — Run / Study / Experiment** — with an immutable,
   content-addressed run store, *sealed* distributions, an automatic monotonic
   **trial counter**, and a one-shot **holdout vault**.
2. **The Null Library and the staged-gate funnel** — a selectable, logged,
   parameterized null object, and a cost-ordered Gate 0→4 sequence that cannot
   be skipped, ending at the vault.

**What this set delivers** (mapped to the spec):

- A **Run atom** (`RunConfig → RunResult`, pure/dumb/cacheable) keyed by a
  deterministic `run_id` hash, written immutably — *failures included* — to a
  run store. The existing `BacktestManager` becomes the Run executor behind this
  contract. **(Spec §1.1, ADR-001)**
- A **Study layer** that orchestrates many Runs along one varying dimension and
  reports a **sealed distribution** — no `best_run`, no `argmax`, no sortable
  member list (INV-2). Ten study types. Each Study emits a `trial_delta`.
  **(§1.2, ADR-002)**
- An **Experiment** that owns the **global trial counter** (monotonic,
  irreversible) and the **holdout vault** (one logged access, self-sealing),
  plus a one-directional lifecycle state machine. **(§1.3)**
- A **Null Library** — a first-class `Null` object with a 7-entry catalog,
  explicit `preserves`/`destroys` hypothesis statements, and a
  recommended-but-never-defaulted, always-logged selection. **(§2.1)**
- The **staged-gate funnel** — Gate 0 integrity (the close-stamped leak scan),
  Gate 1 single-path sanity, Gate 2 robustness (CPCV/synthetic/neighborhood),
  Gate 3 significance (one primary null test, selection-bias-corrected by the
  live trial counter, with DSR + PBO as corroborators), Gate 4 the vault.
  **(§2.2)**
- The **reconciliation loop** (live vs backtest distribution; auto-transition to
  `decaying`) and a **workbench UX** that renders significance, its null, and the
  trial count *inseparably* (INV-3). **(§2.2 tail, §2.3)**

**Why now.** A backtester without these structural protections is not just
incomplete — it is *actively misleading*, because it produces confident numbers
that the math does not support. The three governing invariants (below) are the
whole point: a researcher acting normally — iterating, chasing the promising
direction — is automatically prevented from fooling themselves because the
architecture counts, seals, gates, and locks on their behalf.

---

## 2. Scope

### In scope

The two subsystems of the [Backtest Suite Core Spec](../../../specs/BACKTEST_SUITE_CORE_SPEC.md):

| Spec area | Backtest engine today | Set J delivers |
|-----------|----------------------|----------------|
| §0 Invariants | none | INV-1 skeptical defaults + `unsafe` flag; INV-2 sealed distributions; INV-3 significance carries null + trial count, inseparably |
| §1.1 Run | `BacktestRequest`→`BacktestSnapshot` (mutable job) | `RunConfig`→`RunResult`, content-addressed `run_id`, immutable run store, cache hits, failures logged |
| §1.2 Study | absent | 10 study types, `VarySpec`, sealed `StudyResult` distribution, `trial_delta`, in-Study pre-declared selection rule |
| §1.3 Experiment | absent | global trial counter (monotonic/irreversible), holdout vault (one-shot/logged), lifecycle state machine, `primary_test` |
| §2.1 Null Library | absent | `Null` contract, 7-entry catalog, `preserves`/`destroys`, recommended-not-defaulted selection, logged |
| §2.2 Staged gates | one ad-hoc run | Gate 0→4 funnel; each gate's entry requires the prior gate's pass verdict |
| §2.2 Gate 3 stats | absent | permutation p-value + selection-bias correction + Deflated Sharpe Ratio + Probability of Backtest Overfitting (one verdict, two corroborators) |
| §2.2 Reconciliation | absent | live-vs-backtest distribution comparison; auto `decaying` transition |
| §2.3 Honesty UX | raw metrics | renders significance ⊕ null ⊕ trial count or nothing |

### Out of scope (this set)

- **Data ingestion** — 1-min bar collection and higher-timeframe construction.
  Treated as a defined, validated input (`backtest::store`,
  `backtest::aggregate`, the collectors). Set J *consumes* the close-stamped bar
  construction; it does not build it.
- **Strategy definition / authoring.** The Strategy module owns
  `domain::strategy_def::StrategyDefinition`; Set J pins it by version hash and
  treats it as input (ADR-0007).
- **Live order placement, execution, routing, fills, reconciliation against a
  broker.** The single risk gate (ADR-0005) and the execution stack are
  untouched. Set J's reconciliation loop compares *backtest distribution vs
  realized performance series* — it never places or sizes an order.
- **The probabilistic forecasting model suite** (Set H/I). A strategy *may*
  consume a published model as a feature; Set J does not train, score, or
  evaluate models. Forecast quality (CRPS/coverage) is Set I's instrument; Set
  J's instrument is **strategy edge significance** (permutation p / DSR / PBO).
- **Capital allocation / portfolio construction** across validated strategies.
- **A parallel stats stack outside Rust.** Significance math (permutation,
  DSR, PBO) is straightforward numeric compute and stays in `crates/backtest`
  (D-9); no Python sidecar is introduced.

---

## 3. Locked Decisions (2026-06-17)

| # | Decision | Locked Choice |
|---|----------|---------------|
| D-1 | Position | **Honesty layer on top of the existing Run engine.** Reuse `BacktestManager`/`sim`/`BarStore`/`aggregate_bars` as the Run executor; add Study/Experiment orchestration, the Null Library, the gate funnel, the trial counter, and the vault. The existing `BacktestRequest`/`BacktestSnapshot` job path becomes one *adapter* into a `RunConfig`→`RunResult` execution. Net-new surface is minimized. |
| D-2 | Invariants are structural | The three governing invariants are enforced by architecture, not docs. **INV-1:** costs + global trial counter + holdout lock are *on by default*; disabling any sets `unsafe=true` permanently. **INV-2:** distributions expose no best-member API. **INV-3:** a significance result is invalid unless it carries its null and its trial-count-at-eval; the report renders all three or none. |
| D-3 | Run is pure & dumb | `RunConfig → RunResult` is a pure function (spec ADR-001). `run_id = hash(entire RunConfig)`; identical configs collide and may serve from cache; any field change is a new id. The Run knows nothing of CV, nulls, or trial counting. Cacheable + parallelizable because the funnel spawns millions. |
| D-4 | Distributions sealed | No `best_run`, no `argmax`, no sorted/ranked member list anywhere in `StudyResult` (spec ADR-002). `member_run_ids` is provenance only. "Carry one config forward" uses a **pre-declared in-Study selection rule** (e.g. "median-stable region centroid"), never a user reaching for the peak. |
| D-5 | Trial counter | The `Experiment.trial_counter` is **automatic, monotonic, irreversible**. It increments by `study.trial_delta` on *every* Study — sweeps, failed runs, abandoned directions. No decrement, no reset. Starting over = a new Experiment (you cannot launder trial count by renaming). |
| D-6 | Holdout vault | A locked tail slice no Study can address while `candidate`/`validated`. Reaching it requires passing Gates 0–3. It grants **exactly one** evaluation; `spent` flips true and never back; every access is logged forever (who + when). A second attempt is refused at the API level. |
| D-7 | Null is chosen, logged, never defaulted | `Null` is a first-class, parameterized, logged object the user selects. The suite *recommends* a null per declared strategy type, surfaced as a **prompt**, logged as a user decision, overridable with a logged reason. No invisible default (an invisible default is an invisible assumption). |
| D-8 | Funnel ordering enforced | A gate's entry requires the prior gate's **pass verdict**, which only exists if its Studies actually ran. The funnel cannot be skipped or reordered; the vault (Gate 4) is addressable only from a Gate-3-passed Experiment. |
| D-9 | Stats placement | **Significance math lives in Rust** (`crates/backtest`): permutation-null generation, selection-bias correction, Deflated Sharpe Ratio, Probability of Backtest Overfitting. These are deterministic numeric computations; no ML, no Python sidecar. Consistent with the backtest crate already being a self-contained Rust orchestrator (ADR-0014). |
| D-10 | Money vs statistics | **Costs are `Decimal`** (ADR-0002) and *on by default* (INV-1); the cost model floor is real money. **Metrics, distributions, p-values, and scores are `f64`** (statistical, not money) — matching the existing metric precedent. `check-money-f64` stays green: the suite constructs no `Price`/`Size` it does not already construct through the simulator. |
| D-11 | Storage | **Postgres** owns Experiment/Study/Run metadata, the trial counter, the vault access log, lifecycle state, and the null registry (system of record). **ClickHouse** owns equity curves, trade lists, and metric-distribution series (high-volume, append-only). **Object store** holds large run artifacts. Reuse `crates/storage`. The existing `market_bars` CH table is the read source; the Run store never mutates it. |
| D-12 | Numbering | Reserve **ADRs 0019–0021**, Postgres migrations **0026+**, ClickHouse DDL **05+**. Workbench UI is greenfield, mirroring existing chrome (`--tb-*` tokens, `api/*.ts` patterns, the WS lane). *(Verify next-free numbers before locking — last used: ADR 0018, migration 0025, CH DDL 04.)* |

---

## 4. Architecture

```
┌──────────────────────── FRONTEND (React workbench) ───────────────────────────┐
│  Experiment console (lifecycle + trial counter, always visible) ·             │
│  Study distribution viewer (median / IQR / worst-5% — NO best-member affordance)│
│  Gate funnel board (0→4, locked until prior pass) · Null picker (recommended + │
│  preserves/destroys rendered) · Significance card (p ⊕ null ⊕ trial-count,     │
│  inseparable, INV-3) · Vault panel (one-shot, with access log)                 │
└───────┬───────────────────────────────────────────────────────────────────────┘
        │ REST /api/backtest/experiments|studies|runs|nulls  + WS lane (run/study progress)
        ▼
┌──────────────────────── RUST  (crates/backtest, extended) ────────────────────┐
│  Run engine (reuse): sim · BacktestManager · BarStore · aggregate_bars         │
│  Set J (new):                                                                  │
│    · RunConfig/RunResult/MetricSet contracts + run_id content hash             │
│    · RunStore (immutable, content-addressed, cache hits, failures logged)      │
│    · StudyEngine (10 types) → sealed StudyResult distribution + trial_delta    │
│    · in-Study pre-declared selection rule (no argmax leak)                     │
│    · ExperimentManager: monotonic trial counter + holdout vault + lifecycle    │
│    · Null Library: Null contract + 7 generators + preserves/destroys           │
│    · GateRunner: Gate 0 integrity (close-stamp leak scan) → 1 → 2 → 3 → 4       │
│    · stats: permutation p · selection-bias correction · DSR · PBO              │
│    · ReconciliationStudy (live vs backtest distribution) → decaying transition │
└───────┬───────────────────────────────────────────────────────────────────────┘
        │ reads market_bars (PIT, available_time order) ; never mutates it
        ▼
Stores:  Postgres (experiments, studies, runs-meta, trial counter, vault log,
nulls, lifecycle) · ClickHouse (equity curves, trades, metric distributions) ·
Object store (large run artifacts) · NATS (run/study progress events) ·
Redis (run_id → cached RunResult hot map)
```

**Responsibility split.** The **Run engine** stays a pure
`RunConfig → RunResult` function — cacheable and massively parallel. **All
intelligence about how runs combine** (CV, nulls, trial counting, gating,
sealing) lives one level up in the Study/Experiment/Gate layer. A Study bug
cannot be caught at the Run level (ADR-001 consequence), so every Study type and
every gate carries an adversarial test (§7.9).

---

## 5. Where Set J sits in the build order

The spec prescribes building the object model bottom-up, then the null library,
then the gates that consume both:

```
spec section                              Set J phase
────────────────────────────────────────────────────────────────────
§1.1 Run (the atom)              ──────▶ Phase 0   ┐
§1.2 Study (sealed distribution) ──────▶ Phase 1   ├─ object model
§1.3 Experiment (counter+vault)  ──────▶ Phase 2   ┘
§2.1 Null Library                ──────▶ Phase 3
§2.2 Staged gates 0→4            ──────▶ Phase 4
§2.2 Reconciliation + §2.3 UX    ──────▶ Phase 5
```

---

## 6. Phase Summary

Build order is top-to-bottom. Phase 5 (frontend) can begin against each contract
as soon as it is frozen, in parallel with the phases above it.

| Phase | File | Label | Tasks | Goal |
|-------|------|-------|-------|------|
| 0 | [phase-0-run-atom-and-store.md](phase-0-run-atom-and-store.md) | The Run atom & immutable run store | 10 | `RunConfig`/`RunResult`/`MetricSet`, content-addressed `run_id`, immutable RunStore with cache hits + logged failures, `BacktestManager` as the Run executor, INV-1 skeptical defaults + `unsafe` flag |
| 1 | [phase-1-study-and-sealed-distributions.md](phase-1-study-and-sealed-distributions.md) | The Study layer & sealed distributions | 10 | `StudyConfig`/`VarySpec`/`StudyResult`, 10 study types, sealed distribution (INV-2, no best-member), `trial_delta`, in-Study pre-declared selection rule |
| 2 | [phase-2-experiment-counter-and-vault.md](phase-2-experiment-counter-and-vault.md) | Experiment, trial counter & holdout vault | 9 | `Experiment` aggregate, monotonic/irreversible trial counter, one-shot self-sealing holdout vault + access log, one-directional lifecycle state machine, `primary_test` declaration |
| 3 | [phase-3-null-library.md](phase-3-null-library.md) | The Null Library | 9 | `Null` contract, 7-entry catalog with `preserves`/`destroys`, generators, recommended-not-defaulted + logged selection, attach-to-result plumbing |
| 4 | [phase-4-staged-gate-funnel.md](phase-4-staged-gate-funnel.md) | The staged-gate funnel (0→4) | 12 | Gate 0 integrity (close-stamp leak scan, cost sanity), Gate 1 single-path, Gate 2 robustness, Gate 3 significance (permutation p + selection-bias + DSR + PBO), Gate 4 vault; ordering enforced |
| 5 | [phase-5-reconciliation-and-workbench.md](phase-5-reconciliation-and-workbench.md) | Reconciliation loop & honesty workbench UX | 9 | Live-vs-backtest reconciliation Study + auto `decaying` transition, suite calibration view, INV-3 significance card, distribution viewer, gate-funnel board, null picker, vault panel |

---

## 7. Cross-cutting principles

1. **The three invariants are non-negotiable and structural.** INV-1 (skeptical
   defaults), INV-2 (sealed distributions), INV-3 (significance never naked) bind
   every component. Each gets dedicated adversarial tests; a task that weakens an
   invariant is not done, it is wrong.
2. **The Run is dumb; the Study is smart.** The atom is a pure function so it
   caches by `run_id` and parallelizes for free. Combination intelligence lives
   exactly one level up. No CV/null/counter logic leaks into the Run.
3. **Nothing is ever deleted.** Every executed Run — `ok`, `failed`,
   `rejected_integrity` — is written immutably and **counts toward the trial
   counter**. The failures are part of the honesty.
4. **The counter removes the human from the counting loop.** Studies increment it
   automatically, before any result is seen; Gate 3's significance math reads it
   directly. You physically cannot under-report trials.
5. **The vault is sacred.** One logged access, self-sealing, addressable only
   from a Gate-3-passed Experiment. A failed vault run kills the idea *for this
   holdout*; continuing requires genuinely new data and a new Experiment.
6. **The null is a hypothesis, rendered.** `preserves`/`destroys` are not
   documentation — they are the stated hypothesis, shown in every report. The
   null is chosen and logged, never defaulted.
7. **One verdict, two corroborators — not seventeen votes.** Gate 3 has a single
   primary p-value; DSR and PBO corroborate. Disagreement is a flag to
   investigate, not a result to shop between.
8. **Leakage-safety is mechanical.** Gate 0 checks, on every config, that every
   higher-timeframe signal is stamped at the constituent bar's *close*, never its
   open (`available_time` ordering, ADR-0008) — the single most likely real-world
   leak in this stack.
9. **Adversarial test per mechanism.** Every decided mechanism gets a test, and a
   task is not done until its test is green (master-plan invariant §2.8).

---

## 8. Security & permissions

Auth remains the platform's per-user `BearerToken` placeholder
(`crates/api/src/auth/session.rs`, M-17); experiments, studies, runs, and vault
access logs are user-scoped by `created_by`, matching the existing backtest and
Set H/I managers. Set J adds **no new privileged path**: it cannot place orders,
size positions, or touch capital — it produces a *verdict* a human reads. The
vault access log records `{when, run_id, by}` for every touch, forever; this is
an audit surface, not a permission surface. The single risk gate (ADR-0005) is
untouched.

---

## 9. Derived From / Traceability

| Source | Relationship |
|--------|--------------|
| `docs/specs/BACKTEST_SUITE_CORE_SPEC.md` | **Primary requirement** — every Set J task traces to §0–§2 or an appendix ADR |
| `crates/backtest` (as-built) | The Run engine Set J extends (sim, manager, store, aggregate) |
| `ADR-0001` | Rust modular monolith — Set J stays in-process in `crates/backtest` |
| `ADR-0002` | Decimal money newtypes — defines the cost(Decimal)/metric(f64) boundary (D-10) |
| `ADR-0005` | Single risk gate — unchanged; the suite never trades |
| `ADR-0007` | Frozen strategy definition v1.0 — pinned by version hash as Run input |
| `ADR-0008` / `ADR-0009` | `available_time` ordering + ground-truth archive — the Gate-0 leakage discipline |
| `ADR-0012` | Canonical bar storage — the close-stamped bars Gate 0 validates and Runs read |
| `ADR-0014` | Backtesting via market_simulator SDK — the engine behind the Run executor |
| **ADR-0019 (new, this set)** | Run/Study/Experiment object model + sealed distributions — authored in Phase 0 (folds spec ADR-001 + ADR-002) |
| **ADR-0020 (new, this set)** | The Null Library & null-selection discipline — authored in Phase 3 |
| **ADR-0021 (new, this set)** | Staged-gate funnel, trial counter & holdout vault — authored in Phase 4 |

---

## 10. Glossary

| Term | Meaning |
|------|---------|
| **Run** | The atom: one strategy × params × data slice × cost model × seed → one equity curve + metrics. Immutable, content-addressed by `run_id`, cacheable. |
| **`run_id`** | Deterministic hash of the entire `RunConfig`. Identical configs collide (cache hit); any field change is a new id. |
| **Study** | A deliberate set of Runs along one varying dimension (`VarySpec`), reporting a **sealed** distribution. |
| **Sealed distribution** | A `StudyResult` exposing median/IQR/worst-5%/spread but **no** best-member, argmax, or ranked list (INV-2). |
| **`trial_delta`** | How many Runs a Study added to the global trial counter. |
| **Experiment** | The container for one strategy idea; owns the trial counter and the holdout vault. |
| **Trial counter** | Automatic, monotonic, irreversible count of every Run across all Studies; input to selection-bias correction and DSR. |
| **Holdout vault** | A locked data tail granting exactly one logged evaluation; self-sealing (`spent`). |
| **Null** | A first-class object that generates a null-world dataset; declares what it `preserves` and `destroys`. |
| **Gate** | A funnel stage (0 integrity, 1 single-path, 2 robustness, 3 significance, 4 vault); ordered by cost-per-discriminating-power. |
| **Selection-bias correction** | Adjusting the significance verdict for the live trial count (INV-3). |
| **DSR** | Deflated Sharpe Ratio — Sharpe adjusted for trial count + non-normality. Corroborator in Gate 3. |
| **PBO** | Probability of Backtest Overfitting — corroborator in Gate 3. |
| **Reconciliation Study** | The only Study allowed in `live` state: realized performance vs the backtested distribution. |
| **`unsafe`** | A permanent flag set when any INV-1 default (costs / counter / holdout) is disabled. |

---

## 11. Progress Log

| Date | Phase | Task | Note |
|------|-------|------|------|
| 2026-06-17 | — | plan | Set J created. 12 decisions locked. 59 tasks across 6 phases (0–5). Honest-evaluation core (Run/Study/Experiment + Null Library + staged gates) on top of the `crates/backtest` Run engine; core spec recorded at `docs/specs/BACKTEST_SUITE_CORE_SPEC.md`. End-state design. |
| 2026-06-17 | 0 | J-0.1–J-0.10 | **Phase 0 complete.** `crates/backtest/src/run/`: content-addressed `RunConfig`/`RunId` (SHA-256, map-order-insensitive), standardized `MetricSet` with null honesty hooks, `Trade`/`RunResult`, `RunExecutor` trait + `map_sim_result` core, idempotent immutable `RunStore` (+ `migrations/0026` & `clickhouse/05`), cache-aware `Backtest::run` (`RunOrigin`), INV-1 `UnsafeFlags`, `ENGINE_VERSION` provenance. ADR-0019 Accepted. 94 lib tests green. Deferred live legs: SimRunExecutor bit-for-bit (J-0.6), Pg/CH-backed stores (J-0.7). |
| 2026-06-17 | 1 | J-1.1–J-1.10 | **Phase 1 complete.** `crates/backtest/src/study/`: `StudyConfig`/`VarySpec`/`StudyKind` + kind↔vary validation, sealed `StudyResult`/`Distribution` (no best-member API), `StudyEngine` fan-out with `trial_delta` counting all members (cache hits + failures), pre-declared `SelectionRule`→`carried_forward` (never argmax), all 10 study kinds (sweep/neighborhood/walk-forward/CPCV/nested/permutation/synthetic/cost-sweep/regime/trade-MC), `combinations`/`cpcv_assignments` with disjoint-partition property test, `StudyStore` (+ `migrations/0027`), and the INV-2 adversarial suite (`tests/sealed_distributions.rs`). 3 integration tests green. Deferred live leg: Pg-backed study persistence (J-1.9). |
