# Engine truthfulness — Set M

**Completion: 51% left (7 / 29 tasks). 7 done, 17 partial, 2 not started, 2 blocked upstream, 1 blocked on a human.** Re-counted 2026-10-05 (track VERIFY2) from the per-task markers; the line read 2 / 29 before.

**Status:** PLANNED (2026-10-04, track F-RESERVED) — this file was the reserved letter in [ROADMAP §5](../../ROADMAP.md#5-set-succession-what-the-reserved-letters-still-mean) until today.
**Stage:** [4](../../ROADMAP.md#4-the-stages) — capability depth; Phase 1 can start now
**Derived from:** [FEAT-005](../../../specs/FEAT-005-backtest-engine-integration.md) (with its 2026-09-15 amendment), [FEAT-004](../../../specs/FEAT-004-strategy-representation-v2.md) §2 and §7, BS-007 09_BACKTEST, ROADMAP §5, OPEN-TASKS Part C3
**Requirement IDs:** BT-01…BT-11, ST-01…ST-04, ST-10, EV-05 (position truncation).
**Migration range:** 0205 (one of 0205–0206 used; 0206 reserved).
**Scope class:** End-state architecture.

---

## 1. Why this set exists

Set U made every environment drive one engine and gave it a native venue. What it did not do is make the *numbers* honest beyond the fee: spread and
impact are not modelled, the `RunConfig` fields were never proved to change anything, the rich outputs the agent needs to explain a trade are not
stored, there is no batch or screening tier, and there is no way for the agent to express a strategy that is not a graph.

That last gap matters most. FEAT-004 §2's Layer 1 gives research unlimited expressiveness, and its safety rests on one test (truncation at Gate 0) that
did not exist anywhere in the repository when this set was planned. A position series computed with a full-sample statistic looks like alpha and is look-ahead.

## 2. Scope

| Area | Today | This set delivers |
|---|---|---|
| Layer 1 research strategies | nothing | a position-series type, a deterministic evaluator, a truncation test, a Python SDK and harness; then the job, artifact and gate wiring |
| Costs | fee leg only (`FeeBook`) | a dated per-instrument model with EDGE spread, square-root impact, borrow and latency |
| `RunConfig` | fields declared, not proved | one test per field; the ones that were ignored honoured |
| Outputs | MAE/MFE in memory | fills, positions and decisions tables; `explain_trade` |
| Throughput | one run at a time | batch runs and a calibrated tier 0 that can never judge |
| Research to deploy | no link between layers | a reconciliation record, comparator and a Gate 3 precondition |

### Out of scope

- Features: Set S Phase 5 owns the DATA-006 feature engine and the one-engine rule.
- The order vocabulary, fill timing, fee schedule and Layer 2 driver: Set U.
- Exits as typed fields at `definition_version` 1.3: already driven by `ExitTracker`; the grammar stays frozen.
- Live trading, Set T's deployment path, multi-instrument SLv2 (OQ-040).

### Excluded because something else already owns it

| What | Owner | Evidence |
|---|---|---|
| BT-01 facade, BT-02 fill timing, BT-03 driver, BT-07 model leakage | Set U | U-2.2, U-2.3, U-2.6, U-3.2, U-5.4 |
| FEAT-005 §3–§4 and §9 (the unbuilt halves) | Set U | set-U MASTER header |
| v1 to v2 translation and moves | Set U / Set S | U-7.1, S-5 |
| MAE/MFE populated | Set U | `crates/backtest/src/env/mod.rs` |
| Per-fold models and the prediction series | Set U | U-5.5, U-5.8 |

## 3. Decisions made while planning (ADR rows)

Each row records the alternative that was rejected. They are this set's own ADR index; promote one to `docs/adr/` when a task relies on it.

| ID | Decision | Rejected alternative | Why |
|---|---|---|---|
| ADR-M-01 | The `market_simulator` facade is not widened; BT-01 is met by the native `SimulatedVenue`. | Extend `sdk.rs` with the full order vocabulary and pin a new rev (FEAT-005 §9). | Set U built the vocabulary natively on the paper core (ADR-0044); a second implementation would have to be kept in step by hand. |
| ADR-M-02 | `market_simulator` is a differential reference only; the pinned rev changes in exactly one task (M-4.3). | Keep it as a second backtest engine users can choose. | Two engines produce two numbers for one strategy, which is the defect Set U exists to remove. |
| ADR-M-03 | Layer 1 position series are evaluated by this crate's own bar evaluator, not by Nautilus. | Convert position changes to Nautilus orders (FEAT-005 §4.2 as written). | The evaluator must be deterministic and cheap enough to run K truncation re-runs and to double as tier 0; it is held to the oracle test. |
| ADR-M-04 | A position series carries no P&L field; `deny_unknown_fields` rejects one at the type. | Accept an optional client P&L and ignore it. | ST-02 says client-computed P&L must not be accepted; ignoring is accepting with extra steps. |
| ADR-M-05 | Truncation points are stratified one-per-slice and seeded; the final bar is never a cut. | Uniform random cuts. | Random cuts cluster and leave whole regimes untested; a cut at the last bar compares a series with itself. |
| ADR-M-06 | The reconciliation record keys to `strategy_revision`, not the proposed `strategy_versions`. | Create `strategy_versions` as FEAT-004 §4 describes. | Q-G4-01 left the design open; `strategy_revision` is built, append-only and written by `POST /api/strategies`. A second lineage table would diverge. |
| ADR-M-07 | The ClickHouse outputs DDL is `11_backtest_outputs_v2.sql`. | Keep the spec's `07_backtest_outputs_v2.sql`. | 07, 08, 09 and 10 are taken. |
| ADR-M-08 | `JobKind::Layer1Run` is a new, **uncounted** Research-class kind; the one counted trial is registered inside it through `backtest::run::Backtest` and the Postgres ledger, after the code has produced a series. The Gate-0 truncation cuts are seeded from the experiment and the code hash, never from the manifest. | (a) Add a *counted* kind (breaks `evaluation_counted_kinds_are_exactly_the_spec_list`, which COMP-005 §2 pins). (b) Two chained jobs, `research_run` then `backtest` (`research_run` is already the campaign's agent sub-run worker, and a second job doubles the moving parts for an output nobody else reads). (c) Let the manifest carry the cut seed (the agent could then pick cuts that miss its own look-ahead). | COMP-005 orders an uncounted research step before the counted evaluation; a trial registered in the ledger before the evaluation computes is INV-16 exactly as for every other run, and the truncation re-runs are compute, not looks. |
| ADR-M-09 | Decision-to-order latency defaults by venue class: 0 extra bars for crypto spot, crypto perp, equity broker and synthetic; 1 for prediction markets; an unclassed venue refuses. | A single global latency of 0 (or of 1). | Decide-at-close, fill-at-next-open already imposes one bar. At 1m or coarser, API latency is far inside it. Prediction-market books are thin enough that an order resting a bar is stale. Guessing a class for an unknown venue would be inventing the number. |
| ADR-M-10 | The Layer 1 sweep uses the seeded uniform sampler over the declared params; its seed is derived by the platform from the experiment and the code hash; the report is sealed (a type with no member-level field). | TPE (`research::TpeSampler`). | TPE is built over a `SearchSpace` made from a stored v1 definition, and a Layer 1 function has no such definition; the declared `Param`s are its whole space. A caller-chosen seed would let the agent grind for a flattering draw. |
| ADR-M-11 | A position-series executor **refuses by field name** any `RunConfig` field it cannot honour (fill model, fill delay, null world, target) or that disagrees with the artifact (params, seed, cost model, snapshot, slice, version) instead of ignoring it. | Honour `fill_delay_bars` and `CurrentClose` in the evaluator. | BT-05 is about no silent ignoring. Implementing extra fill timing is its own task; refusing is the honest state until then. |

## 4. Phase summary

| Phase | Title | Tasks | Migrations | State |
|---|---|---|---|---|
| 0 | [Reconciliation: what Set U left, what is still owed](phase-0-reconciliation.md) | 4 | none | 2 / 4 done |
| 1 | [Layer 1: research strategies as position series](phase-1-layer-1-position-series.md) | 8 | none | 0 / 8 done |
| 2 | [Costs, RunConfig and outputs](phase-2-costs-and-outputs.md) | 8 | none (ClickHouse `11_backtest_outputs_v2.sql`) | 0 / 8 done |
| 3 | [Throughput tiers and tick replay](phase-3-throughput-and-replay.md) | 5 | none | 0 / 5 done |
| 4 | [Research-to-deploy reconciliation and the reference venue](phase-4-research-to-deploy.md) | 4 | 0205 | 0 / 4 done |

### Dependency graph

```
Phase 0 ──▶ Phase 1 (Layer 1) ──▶ Phase 4 (reconcile) ──▶ Set R-a RA-3.5
   │
   └──▶ Phase 2 (costs, outputs) ──▶ Phase 3 (batch, tier 0, replay)
```

### Reading the statuses

The pure halves of M-1.1 to M-1.4 were built in this track and carry their own evidence. Every task that needs `crates/jobs`, `crates/api`,
`crates/backtest/src/{runner,sim_executor,models}.rs` or the ClickHouse DDL is **NOT STARTED**: it needs another territory's hook, and the task says which.
