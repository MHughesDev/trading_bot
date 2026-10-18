# Phase 2 — Costs, RunConfig and outputs

**Completion: 25% (2 / 8 tasks), plus 4 `PARTIAL`, 2 `NOT STARTED`.** Re-counted 2026-10-05 (track VERIFY2) from the per-task markers; it read 0 / 8.

**Requirement IDs:** BT-05, BT-06, BT-10, BT-11
**Spec:** [FEAT-005](../../../specs/FEAT-005-backtest-engine-integration.md) §5–§8
**Depends on:** Phase 0.
**Migration range:** none (ClickHouse `11_backtest_outputs_v2.sql`).

Costs are on by default and come from a dated per-instrument model: venue fees from the `FeeBook` (ADR-P6), a spread
estimated from bars (EDGE) or quoted, and square-root impact. Outputs are rich enough to explain every trade.
Set U's `FeeBook::resolve` already handles the fee leg, so the work here is spread, impact, borrow and latency, the
`cost_model_ref` plumbing, and the output tables.

---

### M-2.1 The dated per-instrument CostModel artifact — L
`DONE` 2026-10-05 (track R-MQ) - `crates/backtest/src/cost/mod.rs`: dated model, refuses a missing leg, point-in-time check, sqrt impact charged by `evaluate_with_impact`. Wired into the Layer 1 manifest (`cost_model`). Borrow is stated and priced by `borrow_cost` but not charged by the evaluator. Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

`cost:<instrument>@<date>`: fees from the `FeeBook`, spread by `edge | quoted | fixed`, impact `sqrt` with Y, sigma and ADV
series refs, borrow, latency in bars. Costs on by default; `UnsafeFlags.costs_disabled` marks a run unsafe (exists).
A model missing a leg refuses to build rather than assuming zero.

**Files:** `crates/backtest/src/cost/` (new module), `crates/backtest/src/lib.rs` (one `pub mod` line)
**Acceptance:** a model with no spread or impact leg is refused with the leg named; the same trades cost strictly more under a larger Y; a model dated after the run's window is refused (point-in-time).
**Traces:** BT-06; FEAT-005 §5

### M-2.2 EDGE spread, ADV and sigma from the instrument profile — M
`PARTIAL` 2026-10-04 (track R-MQ) - `SpreadLeg::edge(None)` refuses; an EDGE leg cites a profile row and its window. The instrument_profile job's own output is not yet read into a model (python side outside territory). Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

The `instrument_profile` job (`InstrumentProfileWorker`, track CAMPAIGN 2026-10-03) computes a deterministic profile; this task makes
it the source of the cost model's spread, ADV and sigma. EDGE itself is an open question (OQ-15) and the tool says so.

**Files:** `apps/model-trainer/app/instrument_profile.py` (outside this track), `crates/backtest/src/cost/`
**Acceptance:** the cost model's spread leg cites a profile row and its window; with no profile row the model refuses.
**Traces:** BT-06; FEAT-005 §5; AGENT-003 §7

### M-2.3 cost_model_ref is honoured and CostSweep members differ — M
`NOT STARTED` 2026-10-04 (track R-MQ) - needs `sim_executor.rs` to resolve `cost_model_ref`; the Layer 1 path honours it.

`SimRunExecutor` resolves `cost_model_ref` and applies it. A `CostSweep` study over three cost models yields three distinct
member distributions (FEAT-005 acceptance 2).

**Files:** `crates/backtest/src/sim_executor.rs` (Set U territory: report the hook), `crates/backtest/src/cost/`
**Acceptance:** three cost models produce three different distributions on one fixture; an unresolvable `cost_model_ref` fails the run.
**Traces:** BT-05, BT-06; B2

### M-2.4 sizing_ref, seed and data_snapshot are honoured — M
`NOT STARTED` 2026-10-04 (track R-MQ) - belongs with M-2.3 and Set U files; M-0.2's table says which fields are honoured.

The remaining fields from M-0.2's table that turn out to be ignored. `data_snapshot` reads bars with
`ingested_time <= snapshot` (DATA-005 §4); `seed` seeds probabilistic fills.

**Files:** `crates/backtest` (Set U files: report hooks)
**Acceptance:** M-0.2's test passes with no field marked inert except by declaration.
**Traces:** BT-05; FEAT-005 §6

### M-2.5 Outputs v2: fills, positions, decisions — L
`PARTIAL` 2026-10-04 (track R-MQ) - `clickhouse/11_backtest_outputs_v2.sql` and `backtest/src/outputs_v2.rs` (rows, ReplacingMergeTree, written by the Layer 1 worker, idempotent by run_id). Layer 2 environment writer is Set U territory. Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

`clickhouse/11_backtest_outputs_v2.sql` (the next free number; 09 and 10 are taken): `backtest_run_fills`, `backtest_run_positions`,
`backtest_run_decisions` (slot values, signals, intents), plus `exit_reason`, `entry_order_type` and per-leg costs on trades.
Writers behind the existing store.

**Files:** `clickhouse/11_backtest_outputs_v2.sql` (new), `crates/backtest/src/store.rs`
**Acceptance:** a run writes one decisions row per decision bar; costs on a trade equal the sum of its legs; a re-run writes the same rows (idempotent by `run_id`).
**Traces:** BT-10; FEAT-005 §8

### M-2.6 Annualisation from the instrument calendar — S
`PARTIAL` 2026-10-04 (track R-MQ) - `backtest/src/annualisation.rs` (calendar-aware periods, fine-resolution warning, expectancy default). `run/metrics.rs` does not call it yet; the Layer 1 result carries it. Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

24/7 crypto is 525,600 periods a year at 1m. Objectives at 5m or finer default to expectancy or profit factor; a report never
annualises a 1m Sortino or Calmar without a warning.

**Files:** `crates/backtest/src/run/metrics.rs` (reports the hook)
**Acceptance:** 1m and 1h runs of the same series annualise by their own period counts; a 1m Sortino carries a warning.
**Traces:** BT-10; FEAT-005 §8

### M-2.7 explain_trade — M
`PARTIAL` 2026-10-04 (track R-MQ) - GET /api/backtest/runs/{run_id}/trades/{idx}/explain (`routes/explain_trade.rs`, tenant-checked through the ledger) and MCP `explain_trade`. Only Layer 1 runs write the outputs it reads. Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

`explain_trade(run_id, trade_idx)` reads the decisions table for the entry bar and returns slot values, regime, path, MAE/MFE and exit reason.

**Files:** `crates/mcp-server/src/tools` (new file), `crates/api` route (outside this track)
**Acceptance:** for a stored run, the entry bar's slot values and the exit reason are returned; an index out of range is a typed error.
**Traces:** BT-10; FEAT-005 §8

### M-2.8 Replay determinism is a CI test — M
`DONE` 2026-10-05 (track R-MQ) - `crates/backtest/tests/replay_is_bit_identical.rs` on the position-series executor. Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

The same `RunConfig` hash gives bit-identical trades, fills and equity (FEAT-005 B5). The oracle test already exists and gates
engine changes; this is the stricter claim.

**Files:** `crates/backtest/tests/replay_is_bit_identical.rs` (new)
**Acceptance:** two runs of one hash compare equal byte for byte; mutating the seed changes the hash and, where fills are probabilistic, the result.
**Traces:** BT-11; B5
