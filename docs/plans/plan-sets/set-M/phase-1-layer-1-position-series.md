# Phase 1 — Layer 1: research strategies as position series

**Completion: 12% (1 / 8 tasks), plus 7 `PARTIAL`.** Re-counted 2026-10-05 (track VERIFY2) from the per-task markers; it read 0 / 8.

**Requirement IDs:** ST-01, ST-02, ST-03, ST-04, BT-04, EV-05
**Spec:** [FEAT-004](../../../specs/FEAT-004-strategy-representation-v2.md) §2; [FEAT-005](../../../specs/FEAT-005-backtest-engine-integration.md) §4.2
**Depends on:** Phase 0.
**Blocks:** Set R-a RA-3.5 (the analysis-code truncation test).
**Migration range:** none.

FEAT-004 §2 gives the agent unlimited expressiveness for *research*: a Python function that returns a target-position
series, run by the platform under an explicit execution policy and cost model. The platform never accepts
client-computed P&L (ST-02). The Gate-0 **truncation test** (ST-03) is what makes that safe: re-run the function on
data ending at K stratified cut points and require the earlier positions to be identical, so a `bfill`, a full-sample
normalisation or a centred window is caught naming the first timestamp that differs.

**Built by this set on 2026-10-04 (pure parts, no production caller yet):** `crates/backtest/src/position_series.rs`
(series type and its validity rules, the truncation comparison, the position-to-order planner and a deterministic
bar evaluator that fills at the next bar's open under a fee and half-spread cost) and
`apps/skill-runner/research_strategy.py` (the `research_strategy` decorator, `Param`, `target_vol_scale`, a PIT-clipped
`Data` object and the truncation harness). What is missing is the job, the artifact and the gate wiring (M-1.5, M-1.7),
which live in `crates/jobs`, `crates/api` and `crates/backtest/src/gates`, outside this track's territory.

---

### M-1.1 PositionSeries and its validity rules — S
`PARTIAL` 2026-10-04 — `position_series::{PositionSeries, PositionPoint, PositionSeriesError}` with `validate` and `effective`; `deny_unknown_fields` on both types, so a client P&L is a deserialisation error. Tests (backtest lib, `position_series`): `each_validity_rule_refuses_and_names_the_first_offender`, `nan_means_no_change_and_carries_the_previous_target`, `a_pnl_field_is_an_error_not_an_ignored_value`. **Missing:** no production caller; the research runner (M-1.5) is what reaches it.

Decision times, target positions, `position_units` (`fraction` in [-1, 1] or `units`), `allow_short`. NaN means "no
change", values must be finite and inside the bounds, the index must be strictly increasing and a subset of the
decision times. Pure Rust, no I/O.

**Files:** `crates/backtest/src/position_series.rs` (new)
**Acceptance:** a series with a non-finite value, an out-of-bound value, a short position while `allow_short` is false, a repeated or decreasing timestamp or a timestamp that is not a decision time is refused with a code naming the first offender; NaN carries the previous position forward.
**Traces:** ST-01; FEAT-004 §2.1

### M-1.2 The position-to-order planner and the series evaluator — M
`PARTIAL` 2026-10-04 — `position_series::evaluate` (decide on close, fill at the next open, fee and half-spread per fill, lot rounding, orders only when the target changes, ruin stops trading). Tests: `every_fill_is_at_a_bars_open_and_never_the_decision_bars_close`, `the_same_series_and_bars_give_byte_identical_results` (equity compared bit for bit), `doubling_the_fee_strictly_lowers_final_equity_on_a_series_that_trades`, `a_flat_series_pays_nothing_and_holds_its_equity`, `a_buy_and_hold_with_no_costs_earns_the_price_return_on_the_invested_equity`. **Missing:** it is not a `RunKind` and nothing dispatches it; it does not read the `FeeBook` or a `CostModel` (M-2.1), taking `Costs{fee_bps, half_spread_bps}` directly; it has not been held to the oracle test. ADR-M-03.

At each decision time `delta = target - current`; the order is sized `delta x equity / price`, filled at the next bar's
open under a declared policy, charged the fee and a half-spread, and the series never carries a P&L field. The evaluator
is also the shape of the vectorised tier 0 (M-3.2): bar-close signals, next-open fills, fee plus half-spread, no intrabar exits.

**Files:** `crates/backtest/src/position_series.rs`
**Acceptance:** the same series and bars give byte-identical fills and equity on two runs; every fill is at a bar's open and never at the decision bar's close; a series that carries a `pnl` or `equity` field is rejected at deserialisation (`deny_unknown_fields`); doubling the fee strictly lowers final equity on a series that trades; a flat series pays nothing.
**Traces:** ST-02, BT-04; FEAT-005 §4.2, B9

### M-1.3 The Gate-0 truncation test — M
`PARTIAL` 2026-10-04 — `position_series::{truncation_points, check_truncation, truncation_test}` and, in Python, `research_strategy.truncation_test`. Both use SplitMix64 with one slicing rule and **choose the same cuts** (`cuts_match_the_python_harness` pins `[5, 133, 368, 478, 515, 630, 802, 966]` for seed 7 in both languages). Tests: `a_difference_by_one_ulp_before_the_cut_fails_and_names_the_timestamp`, `a_causal_function_passes_and_a_peeking_one_fails_naming_where` (a `shift(-1)` and a full-sample normalisation fail), Python `test_a_bfill_variant_fails_naming_the_first_differing_timestamp`. Mutation-checked: loosening the Python comparison to a 1e-9 tolerance fails `test_a_difference_by_one_ulp_is_a_failure`. `TruncationVerdict::gate_flag()` returns the `crate::run::Flag` Gate 0 attaches (`leakage.position_truncation_mismatch` naming the first differing timestamp, and `leakage.position_truncation_not_run` for a test that never ran, so an unchecked series is not a passed one): `only_a_pass_raises_no_gate_flag`. Mutation-checked: replacing the exact comparison with a 1e-9 tolerance fails `a_difference_by_one_ulp_before_the_cut_fails_and_names_the_timestamp`. **Missing:** no Gate-0 caller attaches it (M-1.7).

K truncation points (default 8), stratified across the window and seeded. For each, the function is re-run on data ending at
`t_k`; the verdict passes iff `positions_trunc[t <= t_k] == positions_full[t <= t_k]` exactly, with NaN equal to NaN. A failure
records the first differing timestamp and which inputs differed. The comparison and the point selection are Rust;
the re-run is the Python harness in M-1.4.

**Files:** `crates/backtest/src/position_series.rs`
**Acceptance:** `truncation_points(seed, n, window)` is deterministic, stratified (one point per equal slice) and never the final bar; a series that differs in the pre-cut region by one ULP fails and names that timestamp; NaN against NaN passes; a series that differs only after the cut passes; a `bfill` fixture fails and a causal one passes (mutation-checked: loosening the comparison to a tolerance makes the one-ULP test fail).
**Traces:** ST-03; FEAT-004 §2.3, S1

### M-1.4 The research_strategy SDK and runner-side harness — M
`PARTIAL` 2026-10-04 — `apps/skill-runner/research_strategy.py`: `research_strategy`, `Param`, `target_vol_scale`, a `Data` object clipped at the cutoff with a closed, causal `feature()` grammar (never `eval`), seeded `rng`, `run_positions` validating the output, `truncation_test`, `lint_tuned_literals`. 16 pytest cases in `apps/skill-runner/tests/test_research_strategy.py`. **Missing:** `Data.feature` is a numpy/pandas stand-in for the DATA-006 batch engine; `data.prediction` raises until the research runner exists; the `tbot.strategy` re-export belongs to `apps/tbot` (not this track's).

`research_strategy(instruments, timeframe, params, position_units, allow_short)`, `Param.int/float`, `target_vol_scale`, a `Data`
object whose bars are clipped at the call's cutoff, `feature(expr)` for a closed list of windowed expressions, a seeded `rng`,
and `truncation_test(fn, data, params, k, seed)`. It runs inside the existing skill runner (no network, no clock). The
`tbot.strategy` re-export in `apps/tbot` is a follow-up for that package's owner.

**Files:** `apps/skill-runner/research_strategy.py` (new), `apps/skill-runner/tests/test_research_strategy.py` (new)
**Acceptance:** the FEAT-004 §2.1 example runs and returns a series inside bounds; a `bfill` variant fails `truncation_test` naming the first differing timestamp and the differing input; `data.close` never exposes a bar after the cutoff; two runs with one seed are identical; a tuned-looking numeric literal raises a lint warning.
**Traces:** ST-01, ST-03; FEAT-004 §2.1, §2.3, S1

### M-1.5 The research_run job, the position_series artifact and RunKind::PositionSeries — L
`PARTIAL` 2026-10-04 (track R-MQ) - `JobKind::Layer1Run` (uncounted, ADR-M-08), `jobs::kinds::layer1::Layer1Manifest` (deny_unknown_fields, no P&L), `api::layer1_worker::Layer1Worker` (bars via BarStore, sandboxed harness `layer1_driver.py`, Rust truncation compare, code and series stored as artifacts, one counted trial through `Backtest` and `PgTrialLedger`), `backtest::position_run`, registered in apps/platform main.rs. Python harness verified: tests/test_layer1_driver.py 7 pass, test_research_strategy.py 16 pass. Rust unverified. Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

`bt.run` with a Layer 1 file submits a `research_run` job whose output is a `position_series` artifact; that triggers a
`backtest` job with `RunKind::PositionSeries { series_handle }`, linked to the Experiment, and the backtest is the counted
trial. The code snapshot hash is part of `strategy_version`. **Check first** whether `ResearchRun` (Set S S-10.6) already
carries arbitrary code: if it does, this is a manifest kind and not a second job type.

**Files:** `crates/jobs`, `crates/api` (outside this track), `crates/backtest/src/run`
**Acceptance:** a Layer 1 file produces one counted trial; the artifact's content hash and the code hash are in the run's `strategy_version`; a client that supplies P&L is refused; the run is registered in the trial ledger before it computes (INV-16).
**Traces:** BT-04, ST-02; FEAT-004 §2.2; FEAT-005 §4.2

### M-1.6 Layer 1 sweeps use the platform sampler — M
`PARTIAL` 2026-10-04 (track R-MQ) - `research::layer1_sweep` (seeded sampler, sealed report with no member-level field, report_from) and the worker's `sweep` path (N members = N trials; seed derived by the platform). MCP `run_research_strategy` has `sweep_runs`. Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

The FEAT-003 sweep engine samples params (Random/TPE) and submits member `research_run` + `backtest` pairs; INV-2 applies
unchanged and the agent never sees an argmax. Reuse `crates/research::sweep`; add a Layer 1 member kind.

**Files:** `crates/research/src` (additive), `crates/mcp-server`
**Acceptance:** a Layer 1 sweep of N params adds exactly N to the trial counter; no tool result contains the best member; the sampler seed is recorded.
**Traces:** ST-04; FEAT-004 §2.4, S2

### M-1.7 Gate 0 runs the truncation test on every position_series experiment — M
`DONE` 2026-10-05 (track R-MQ) - `gates/position_leakage.rs`; `GateRunner::gate0` refuses a `layer1:` experiment with no verdict, `gate0_position_series` takes it; the executor rejects a run with a missing or failed verdict. Missing: `suite.rs`'s funnel builds its own Gate 0 inputs and was not touched. Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

Wire M-1.3's verdict into Gate 0 as a `Flag` with a stable code (`leakage.position_truncation_mismatch`) beside
`leakage.model_trained_past_decision`. The test is a `research_run` sub-job; its cost counts toward compute, not trials.

**Files:** `crates/backtest/src/gates/model_leakage.rs` neighbours (a new `gates/position_leakage.rs`), `crates/api/src/gate_worker.rs` (outside this track)
**Acceptance:** a position_series experiment cannot advance past Gate 0 without a truncation verdict; the failing flag carries the first differing timestamp; the sub-job spends no trial.
**Traces:** ST-03, EV-05; FEAT-004 §2.3

### M-1.8 A tool and CLI entry for Layer 1 — S
`PARTIAL` 2026-10-04 (track R-MQ) - `crates/mcp-server/src/tools/layer1.rs` (`run_research_strategy`, thin client of POST /api/jobs, no P&L field, params required, costs required); registered in spec.rs and lib.rs. The `tbot` CLI half is outside territory. Verified 2026-10-05: compiled and run green (backtest lib 563, gate0_position_series 3, runconfig_fields_are_honoured 4, replay_is_bit_identical 4, vector doctest 1, jobs lib 79, research lib 150, api layer1 6 incl. the real sandbox harness).

One MCP tool and `tbot` subcommand that submit a Layer 1 file, plus its entry in the authoring guide.

**Files:** `crates/mcp-server/src/tools` (new file), `apps/tbot` (outside this track)
**Acceptance:** the tool is in the catalogue within the 1,200-byte descriptor budget and the 16-per-namespace cap; it refuses a file that declares no params.
**Traces:** ST-01; FEAT-004 §2.2
