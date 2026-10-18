# Phase 0 — Reconciliation: what Set U left, what is still owed

**Completion: 50% (2 / 4 tasks), plus 2 `NOT STARTED`.**

**Requirement IDs:** BT-01, BT-02, BT-03, BT-05
**Spec:** [FEAT-005](../../../specs/FEAT-005-backtest-engine-integration.md) §3, §6, §9
**Depends on:** nothing inside this set.
**Migration range:** none.

FEAT-005 was written on 2026-09-11 against a `market_simulator` facade that exposed one order type. Set U
(2026-10-01) then built a native `SimulatedVenue` on the paper venue's core (ADR-0044), the full order
vocabulary, one fee schedule, decide-at-close / fill-at-next-open, the Layer 2 `StrategyInstance` driver and the
model point-in-time check. Before anything is built, this phase says which BT- requirements are already met by
that, which are met in a different place than the spec said, and which are open. It is short on purpose: the
defect this repo breeds is building what exists.

---

### M-0.1 Audit the BT- requirements against what Set U built — S
`DONE` 2026-10-04 — the table is the deliverable. It was built from the Set U plan files and a grep of `crates/backtest`, not from a green run of every cited test.

Each BT- requirement was checked against the Set U plan files and the tree on 2026-10-04.

| BT | Requirement | State | Where |
|---|---|---|---|
| BT-01 | facade exposes the order types, fee/fill presets, latency | **superseded** by a native venue; the facade is not widened (ADR-M-01) | U-2.2, U-2.3, U-2.6 |
| BT-02 | fill timing verified by test; default decide-at-close / fill-next-open | **met** in Set U; the manifest line is open (M-0.4) | U-2.2, `crates/backtest/tests/fill_delay.rs` |
| BT-03 | Layer 2 drives `StrategyInstance`; `evaluate_signals` removed | driver **met**; removal is `BLOCKED` in U-8.3 | U-3.2, U-8.3 |
| BT-04 | `position_series` run kind | **open** | Phase 1 |
| BT-05 | every `RunConfig` field honoured | **unverified** | M-0.2 |
| BT-06 | dated per-instrument cost models, EDGE + square-root | fee side met (FeeBook); spread and impact open | Phase 2 |
| BT-07 | model backtests read walk-forward series; Gate 0 rejects overlap | **met** | U-5.4, `gates/model_leakage.rs` |
| BT-08 | batch runs | **open** | Phase 3 |
| BT-09 | vectorised tier 0, calibrated | **open** | Phase 3 |
| BT-10 | rich outputs, MAE/MFE populated | MAE/MFE **met** in `env/mod.rs`; ClickHouse tables open | Phase 2 |
| BT-11 | replay determinism per `RunConfig` hash | **unverified** as a CI test | M-2.8 |

**Files:** this file; evidence is `docs/plans/plan-sets/set-U/phase-2-venues-and-risk.md`, `phase-3-backtest-environment.md`, `phase-5-models.md`
**Acceptance:** every BT- row above names a state and a place; a requirement is never both `met` and listed as a task below.
**Traces:** FEAT-005 §9; BS-007 09 §9; ADR-0044

### M-0.2 Prove every RunConfig field changes the run (BT-05) — M
`PARTIAL` 2026-10-04 (track R-MQ) - `crates/backtest/tests/runconfig_fields_are_honoured.rs`: field table compared with the struct source (a new field without a row fails), one proof per field, covering the position-series executor, whose `PositionRunExecutor::check_config` refuses by field name. The SimRunExecutor and v3 paths are unverified (need PG/ClickHouse). Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

`RunConfig` declares `cost_model_ref`, `fill_model`, `sizing_ref`, `seed`, `data_snapshot`, `strategy_version`,
`params`, `data_slice` and `fill_delay_bars`. A field the executor ignores is a field whose hash differs while its
result does not, which is the defect FEAT-005 §2 found on 2026-09-10. Write one test per field: change only that
field, run the same fixture, and assert the output differs (or, for `strategy_version`, that a mismatch fails the run
as `version_mismatch`). A field that is *intended* to be inert for a run kind says so in a table in the test.

**Files:** `crates/backtest/tests/runconfig_fields_are_honoured.rs` (new)
**Acceptance:** for each field, a test that changes only it and observes a different trade list, equity curve or refusal; a field that cannot be observed is listed with the reason; the test fails if a new `RunConfig` field is added without a row (struct field list compared to the table).
**Traces:** BT-05; FEAT-005 §6, B10

### M-0.3 Decide the market_simulator contract (ADR-M-02) — S
`DONE`

Set U made `market_simulator` the *reference*, not the engine. This task records what that means for Set M:
the pinned rev in the workspace `Cargo.toml` is bumped only by M-4.3, nothing in this set widens its facade, and the
`market_simulator` contract is a differential test (same bars, same orders, both venues, stated tolerance) rather
than a dependency of any Layer 1 path.

**Files:** this file §3, ADR-M-02
**Acceptance:** ADR-M-02 names the accepted and the rejected alternative; M-4.3 is the only task that touches the pinned rev.
**Traces:** ADR-0044; FEAT-005 §9

### M-0.4 Every run manifest states its execution policy — S
`PARTIAL` 2026-10-04 (track R-MQ) - `crates/backtest/src/execution_policy.rs` (ExecutionPolicy block, optimistic_close flag, `require_block`); the Layer 1 worker states the block in every result. Missing: the report assembly in crates/api (outside territory) must call `require_block`. Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

FEAT-005 §3.5 and acceptance criterion 1: the execution policy (`FillModel`, latency bars, fee schedule id) is in the run
manifest and in every report's execution-assumptions block. `fill_delay_bars` is already hashed; the others are
derivable but not stated. A `CurrentClose` run additionally carries `execution_policy=optimistic_close` in its verdicts.

**Files:** `crates/backtest/src/receipt.rs` (reads only), a manifest field in `crates/api` report assembly
**Acceptance:** a stored run's manifest names its fill model, fill delay and fee schedule; a `CurrentClose` run is labelled `optimistic_close` in the gate verdict it feeds; a test fails when a report is rendered without the block.
**Traces:** BT-02; FEAT-005 §3.5, B1
