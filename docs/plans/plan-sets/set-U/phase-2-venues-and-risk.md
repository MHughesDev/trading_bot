# Phase 2 — Venues and the risk gate

**Completion: 83% (5 / 6 tasks).** U-2.1–U-2.5 DONE 2026-10-02; U-2.6 (one fee schedule) added and open.

**Requirement IDs:** SU-VEN-01…05.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §9.2, §9.3; ADR-0044, ADR-0046.
**Depends on:** Phase 0 (intents and order vocabulary).
**Blocks:** Phases 3, 4.
**Migration range:** none.

Backtest and paper should *work similarly but not exactly the same*. This phase makes that literal: one venue core, two drivers. And it puts the risk gate in front of the backtest, where it has never been.

---

### U-2.1 Extract the venue core — L
`DONE` 2026-10-02 — **by making the existing engine environment-neutral rather than
extracting a copy of it.** `PaperTradingEngine` *is* the venue core: it reads time only
through an injected `execution::clock::VenueClock` (`WallClock` for paper, `SimClock` for
a backtest), takes order ids from an injected `OrderIds` (`RandomIds` / `SequentialIds`),
and learns prices only from `on_mark` / `on_bar_range`. A second struct would have been
a second place for paper and backtest to drift apart, which is the failure the owner
asked to remove. Evidence: every pre-existing paper-engine test passes unchanged in
behaviour (the only edits to existing tests add the new `OrderIntent` fields);
`tests/venue_core_is_environment_neutral.rs` scans the core's production code for any
clock read (`Utc::now`, `Instant::now`, …) or price source (`reqwest`, `tokio::`,
`sqlx`, `async fn`) and finds none, and pins `WallClock` as the single wall-clock read.
**Acceptance amended:** "fees on both legs from one `FeeBook` call site" is replaced by
*both venues price every fill through the same `SimulatorSet` call*
(`PaperTradingEngine::attempt`). Reconciling `SimulatorSet`'s taker-only `fee_rate` with
`FeeBook`'s maker/taker/half-spread schedule is real work with its own acceptance, so it
is U-2.6 rather than a clause quietly dropped from this task.

Lift working orders, positions, cash, fees (`FeeBook`), OCO groups and per-asset-class accounts out of `PaperTradingEngine` into a `VenueCore` with no notion of where prices come from. `PaperTradingEngine` becomes a thin driver over it, with no behaviour change.

**Files:** `crates/execution/src/venue_core/`, `crates/execution/src/paper/engine.rs`.
**Acceptance:** every existing paper-engine test passes unchanged; the core has no clock read and no mark source (source scan); fees on both legs come from one `FeeBook` call site.
**Traces:** ADR-0044; COMP-002 §3.7.

### U-2.2 SimulatedVenue and the bar fill model — L
`DONE` 2026-10-02 — `execution::sim_venue::SimulatedVenue`: the paper engine on a
`SimClock` with sequential ids. Per bar: open marked (gapped orders fill at the open) →
orders submitted since the last bar arrive and are priced at the open → `on_bar_range`
(stops first against the adverse extreme, then limits at their price) → close marked.
`FillPolicy::CurrentClose` executes at submit and `is_optimistic()`; refusals on arrival
are first-class (`BarReport::rejected`). Evidence, `tests/sim_venue.rs`: a market order at
bar t fills at t+1's open plus spread, slippage and a non-zero commission; a limit touched
only by t+3's low fills on t+3 at the limit; a bar spanning a bracket's stop and target
exits at the stop and cancels the target; **the oracle** — a long/flat strategy that knows
every next bar's close, 600 1m bars at σ≈10 bps — loses money through this venue and wins
through the same venue with costs zeroed, so the test is load-bearing; two identical runs
produce identical ids, prices, fees and equity. `backtest` `run::config::two_runs_with_
different_fill_policies_get_different_ids` pins the policy into the `run_id`. A mutation
(fill at submit under `NextBarOpen`) fails two of these tests.

A venue driven by historical bars on a simulated clock: market orders fill at the next bar's open plus half-spread and slippage; limit and stop orders trigger on the following bars' high/low path; when a stop and a target fall in one bar the stop wins; `CurrentClose` only as a declared, flagged policy. The execution policy is part of `RunConfig` and its hash.

**Files:** `crates/execution/src/sim_venue.rs`.
**Acceptance:** a market order submitted at bar t fills at bar t+1's open plus costs; a limit at a price touched only by bar t+3's low fills on bar t+3; a bar spanning both stop and target exits at the stop; the oracle test passes against this venue; two runs with different fill policies get different `run_id`s.
**Traces:** FEAT-007 §9.2; FEAT-005 §3; ADR-0044.

### U-2.3 The full order vocabulary — M
`DONE` 2026-10-02 — `domain::order::OrderType` gains `Stop` and `TrailingStop`;
`OrderIntent` gains `stop_price`, `trail`, `reduce_only`, `post_only`, `oco_group`
(serde-defaulted, so old journals and messages still parse) and `check_shape()`. **A
latent bug closed on the way:** `StopLimit` had no field for its trigger anywhere, so
every stop-limit in the platform was a plain limit — the paper engine rested it as one
and the Tradier/Tradovate adapters *sent* it as one. The venue core now triggers stops
on the mark (or the bar's adverse extreme), trails from the best price seen (ratcheting
only after a bar, never within it), caps reduce-only at the opposing position and
cancels the rest, refuses a post-only limit that would take, and cancels OCO siblings.
Alpaca sends `stop_price`/`trail_price` and refuses post-only; Tradier and Tradovate
refuse stop types by name. Evidence: engine unit tests (`a_stop_rests_until_…`,
`a_stop_limit_becomes_a_limit_…`, `a_trailing_stop_…` ×2, `reduce_only_…`,
`post_only_…`, `one_cancels_other_…`, `a_bar_spanning_both_bracket_legs_…`) and, in
`tests/sim_venue.rs`, `every_order_type_fills_on_the_same_price_in_paper_and_in_
simulation` — one script, both venues, the same fill step for all five types — plus the
reduce-only and post-only criteria run against both venues.

`domain::order` gains stop-market and trailing-stop types, reduce-only and post-only; the venue core executes them; brokers that cannot express one refuse it by name.

**Files:** `crates/domain/src/order.rs`, `crates/execution/src/venue_core/`.
**Acceptance:** each order type fills correctly in SimulatedVenue and PaperVenue against scripted prices; a reduce-only order never increases exposure; a post-only limit that would take is rejected, not filled.
**Traces:** FEAT-007 §6; ADR-0041.

### U-2.4 The risk gate in a simulated context — M
`DONE` 2026-10-02 — `risk::context` gains `SimulatedInputs`, `SimulatedRiskState` (order
rate counted on the simulated clock; a per-day halt latch) and `GateContext::simulated`;
`execution::sim_risk::SimulatedRisk` runs the production `RiskGate::check` with a
context built from the simulated account, its own kill switch, and a log of every
refusal. Evidence, `tests/sim_risk.rs`: a strategy buying past the position cap holds at
the cap with each of four refusals logged in simulated time; a daily-loss breach halts
the run for the rest of the simulated day even after the loss recovers, and the next day
trades (a mutation removing the latch fails it); rate limits count simulated seconds;
the run's kill switch stops only that run; an instrument with no metadata is refused as
live refuses it. **A live-gate defect found and fixed:** the position cap added the
order's size whatever its side, so a strategy at its cap could not sell to exit;
`limits::check_position_after` judges the position the order would leave. That is the
one change to the live gate's behaviour; `cargo test -p risk` 76/76.

`RiskGate::check` runs against an environment-supplied context; a backtest supplies simulated equity, positions, clock and a per-run kill switch. Rejections are first-class events with the check that failed.

**Files:** `crates/risk/src/context.rs`, `crates/risk/src/gate.rs`.
**Acceptance:** a backtest whose strategy exceeds the position cap has those orders rejected and reported per trade; a daily-loss breach halts the simulated strategy for the rest of the simulated day; the live gate's behaviour is unchanged (existing tests).
**Traces:** ADR-0046; COMP-002 §3.1, §3.2.

### U-2.5 PaperVenue on the core — S
`DONE` 2026-10-02 — `execution::Venue` (sync port, orders addressed by idempotency key so
a backtest order has a handle before it reaches the venue); `paper::PaperVenue` over the
shared engine (its submit is `PaperTradingEngine::submit`, the path the hot-path tests
exercise) and `SimulatedVenue` both implement it. Evidence:
`an_environment_is_built_with_either_venue_without_a_type_change` holds a
`Box<dyn Venue>` of each, and the order-type, reduce-only and post-only tests drive both
through `&dyn Venue`. Per ADR-0048 this is now *the* paper venue — no broker's paper
account is one.

### U-2.6 One fee schedule for both venues — M
`NOT STARTED`

`SimulatorSet` charges one taker `fee_rate` per asset class; the backtest's `FeeBook`
(`backtest::costs::FeeSchedule`) carries maker and taker rates, half-spread, venue, tier
and an effective date. Now that backtests execute on `SimulatorSet`, the two must become
one: a resting limit filled by the bar range is a maker fill and should be charged as
one; a crossing order is a taker fill. Source the simulators' fees from the dated
`FeeSchedule` so a fee-tier change is one edit that both venues see.

**Files:** `crates/execution/src/paper/{clob,engine}.rs`, `crates/backtest/src/costs.rs`.
**Acceptance:** a resting limit filled by `on_bar_range` is charged the maker rate and a
market order the taker rate, in both venues; a `FeeSchedule` change moves both venues'
fees in one place; the oracle test still fails the always-in oracle at 1m.
**Traces:** ADR-0044; EXTERNAL-REVIEW-2026-09 §3.4.

Name the paper driver `PaperVenue` behind the same `Venue` port as `SimulatedVenue`, so environments hold one trait.

**Files:** `crates/execution/src/paper/venue.rs`.
**Acceptance:** an environment can be built with either venue without a type change (a compile test); the paper fill path is the one exercised by the existing hot-path tests.
**Traces:** ADR-0044.
