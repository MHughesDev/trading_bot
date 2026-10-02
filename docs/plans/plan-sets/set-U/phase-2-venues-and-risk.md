# Phase 2 — Venues and the risk gate

**Completion: 0% (0 / 5 tasks)**

**Requirement IDs:** SU-VEN-01…05.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §9.2, §9.3; ADR-0044, ADR-0046.
**Depends on:** Phase 0 (intents and order vocabulary).
**Blocks:** Phases 3, 4.
**Migration range:** none.

Backtest and paper should *work similarly but not exactly the same*. This phase makes that literal: one venue core, two drivers. And it puts the risk gate in front of the backtest, where it has never been.

---

### U-2.1 Extract the venue core — L
`NOT STARTED`

Lift working orders, positions, cash, fees (`FeeBook`), OCO groups and per-asset-class accounts out of `PaperTradingEngine` into a `VenueCore` with no notion of where prices come from. `PaperTradingEngine` becomes a thin driver over it, with no behaviour change.

**Files:** `crates/execution/src/venue_core/`, `crates/execution/src/paper/engine.rs`.
**Acceptance:** every existing paper-engine test passes unchanged; the core has no clock read and no mark source (source scan); fees on both legs come from one `FeeBook` call site.
**Traces:** ADR-0044; COMP-002 §3.7.

### U-2.2 SimulatedVenue and the bar fill model — L
`NOT STARTED`

A venue driven by historical bars on a simulated clock: market orders fill at the next bar's open plus half-spread and slippage; limit and stop orders trigger on the following bars' high/low path; when a stop and a target fall in one bar the stop wins; `CurrentClose` only as a declared, flagged policy. The execution policy is part of `RunConfig` and its hash.

**Files:** `crates/execution/src/sim_venue.rs`.
**Acceptance:** a market order submitted at bar t fills at bar t+1's open plus costs; a limit at a price touched only by bar t+3's low fills on bar t+3; a bar spanning both stop and target exits at the stop; the oracle test passes against this venue; two runs with different fill policies get different `run_id`s.
**Traces:** FEAT-007 §9.2; FEAT-005 §3; ADR-0044.

### U-2.3 The full order vocabulary — M
`NOT STARTED`

`domain::order` gains stop-market and trailing-stop types, reduce-only and post-only; the venue core executes them; brokers that cannot express one refuse it by name.

**Files:** `crates/domain/src/order.rs`, `crates/execution/src/venue_core/`.
**Acceptance:** each order type fills correctly in SimulatedVenue and PaperVenue against scripted prices; a reduce-only order never increases exposure; a post-only limit that would take is rejected, not filled.
**Traces:** FEAT-007 §6; ADR-0041.

### U-2.4 The risk gate in a simulated context — M
`NOT STARTED`

`RiskGate::check` runs against an environment-supplied context; a backtest supplies simulated equity, positions, clock and a per-run kill switch. Rejections are first-class events with the check that failed.

**Files:** `crates/risk/src/context.rs`, `crates/risk/src/gate.rs`.
**Acceptance:** a backtest whose strategy exceeds the position cap has those orders rejected and reported per trade; a daily-loss breach halts the simulated strategy for the rest of the simulated day; the live gate's behaviour is unchanged (existing tests).
**Traces:** ADR-0046; COMP-002 §3.1, §3.2.

### U-2.5 PaperVenue on the core — S
`NOT STARTED`

Name the paper driver `PaperVenue` behind the same `Venue` port as `SimulatedVenue`, so environments hold one trait.

**Files:** `crates/execution/src/paper/venue.rs`.
**Acceptance:** an environment can be built with either venue without a type change (a compile test); the paper fill path is the one exercised by the existing hot-path tests.
**Traces:** ADR-0044.
