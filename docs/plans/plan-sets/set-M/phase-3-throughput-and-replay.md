# Phase 3 — Throughput tiers and tick replay

**Completion: 40% (2 / 5 tasks), plus 1 `PARTIAL`, 2 `BLOCKED:UPSTREAM`.** Re-counted 2026-10-05 (track VERIFY2) from the per-task markers; it read 0 / 5.

**Requirement IDs:** BT-08, BT-09
**Spec:** [FEAT-005](../../../specs/FEAT-005-backtest-engine-integration.md) §7, §9
**Depends on:** Phases 1 and 2.
**Migration range:** none.

Batch runs make sweeps cheap; a vectorised tier 0 makes screening cheaper still but may never produce a gate verdict; tick and quote
replay need data the platform does not yet hold at depth (Set P).

---

### M-3.1 BatchRun: N parameter sets per data load — L
`PARTIAL` 2026-10-04 (track R-MQ) - `backtest/src/batch.rs`: one load, each member its own run id and counted trial, equal to its single run. The 5x speedup against a real loader is NOT measured. Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

`BatchRun { base, param_sets }` loads bars and features once, plans the union of feature DAGs and evaluates the members in one
worker. Each member is its own `RunConfig`, `run_id` and counted trial (INV-1).

**Files:** `crates/backtest/src/batch.rs` (new)
**Acceptance:** a 200-member batch is at least 5x faster than 200 single runs on one fixture (measured, reported); every member is counted; member results equal their single-run results.
**Traces:** BT-08; B6

### M-3.2 Vectorised tier 0 and its calibration gate — L
`DONE` 2026-10-05 (track R-MQ) - `backtest/src/vector/mod.rs`: `Tier0::enable` needs a calibration row, refuses below 0.8, `ScreenResult` has no conversion to `RunResult` (compile_fail doctest). No production caller yet. Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

The position-series evaluator (M-1.2) generalised to a screening evaluator. It is used only for screening after Spearman rho >= 0.8
against the event engine on a calibration set, and it never produces a gate verdict (a type, not a convention: its result type has no `GateInput` conversion).

**Files:** `crates/backtest/src/vector/` (new)
**Acceptance:** tier 0 is off until a calibration row exists; a calibration below 0.8 keeps it off and says why; no code path feeds a tier-0 result to a gate (compile-time).
**Traces:** BT-09; FEAT-005 §7, B7; FEAT-003 P5

### M-3.3 Trade-tick replay — L
`BLOCKED:UPSTREAM`

Replay trade ticks through the venue so a strategy that decides intrabar is testable. **Blocked on tick history**: `market_bars_v2` holds bars and
the forward collectors write trades only from the day they were started (Set P P-0.1).

**Files:** `crates/backtest/src/env/` (Set U territory), `crates/dataplane`
**Acceptance:** a trade-tick fixture replays to the same bars the bar path builds from it; a window with no ticks is a typed refusal.
**Traces:** FEAT-005 §9

### M-3.4 Quote-tick replay and a quoted-spread cost leg — L
`BLOCKED:UPSTREAM`

As M-3.3 for quotes; the cost model's `spread.method = quoted` reads them. Blocked on L2/quote history (Set P).

**Files:** `crates/backtest/src/cost/`
**Acceptance:** a quoted-spread model on a quote fixture charges the quoted half-spread; absent quotes refuse rather than fall back to EDGE silently.
**Traces:** BT-06; FEAT-005 §5

### M-3.5 Latency defaults per venue class — S
`DONE` 2026-10-05 (track R-MQ) - ADR-M-09 and `cost::latency_for` / `venue_class` (unclassed venue refuses). Not yet read by a run path: the evaluator refuses nonzero latency. Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

FEAT-005 open question 1: 0 or 1 bar of decision-to-order latency by venue class. Decide, record as an ADR and put the default in the cost model.

**Files:** this file §3, `crates/backtest/src/cost/`
**Acceptance:** each venue class resolves to a latency; an unclassed venue refuses.
**Traces:** FEAT-005 §11
