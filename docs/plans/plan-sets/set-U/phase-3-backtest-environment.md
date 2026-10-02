# Phase 3 — The backtest environment

**Completion: 0% (0 / 5 tasks)**

**Requirement IDs:** SU-BT-01…05.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §9.1, §9.4; ADR-0039, ADR-0046.
**Depends on:** Phases 1 and 2.
**Blocks:** Phases 6, 8.
**Migration range:** none.

The engine meets history. When this phase closes, a v3 strategy backtests through exactly the code that will run it on paper, gated, journaled, and in the shape Set J already judges.

---

### U-3.1 Environment ports — M
`NOT STARTED`

Traits `Clock`, `MarketData`, `ModelProvider`, `Venue`, `Journal`, and an `Environment` that owns one of each plus the risk gate and execution engine. The engine depends on none of them.

**Files:** `crates/strategy-engine/src/env.rs` (traits only), `crates/backtest/src/env/`.
**Acceptance:** `strategy-engine` has no dependency on `execution`, `risk`, `storage` or `model-registry` (a Cargo-graph test); a mock environment drives a full decision cycle in a unit test.
**Traces:** FEAT-007 §9.1; ADR-0039.

### U-3.2 BacktestRunner — L
`NOT STARTED`

Resolve the revision, register the trial (INV-16), materialise params, compile, load warm-up plus window for every declared (instrument, timeframe) point-in-time, merge them into one time-ordered stream, step the engine on decision ticks, resolve model requests, route orders through the risk gate to the SimulatedVenue, feed fills and order updates back as engine events.

**Files:** `crates/backtest/src/runner.rs`.
**Acceptance:** an indicator strategy and a bracket strategy each backtest end to end; a strategy reading BTC 1d while trading ETH 1h sees only BTC daily bars closed by the decision instant; every run appears in the trial ledger before it computes.
**Traces:** FEAT-007 §9.4; ADR-0039; INV-16.

### U-3.3 Results Set J can judge — M
`NOT STARTED`

Write the trade list, daily return series, equity curve, gate rejections, model pins and fold usage, execution policy, and the decision journal into the run artifact and the existing results tables, so experiments, nulls, the funnel and `/experiments/{id}/returns` consume v3 runs unchanged.

**Files:** `crates/backtest/src/run/result.rs`, `crates/backtest/src/runner.rs`.
**Acceptance:** a v3 run is accepted by the Set J funnel and studies without a code change there; the manifest names fill policy, model versions and folds, and the count of `missing`-driven holds by reason.
**Traces:** FEAT-007 §9.4; ADR-0019; ADR-0021.

### U-3.4 Backtests of v3 graphs through the job service and API — M
`NOT STARTED`

`/api/backtests` and the backtest job worker accept a v3 revision and run it through `BacktestRunner`; v1 revisions keep the old path until Phase 8.

**Files:** `crates/api/src/routes/backtests.rs`, `crates/backtest/src/manager.rs`.
**Acceptance:** a v3 backtest submitted over HTTP runs and reports; a v1 submission is unaffected (existing tests).
**Traces:** FEAT-007 §9.4, §12.

### U-3.5 Preview is a short backtest — M
`NOT STARTED`

`/api/agent/strategy/preview` and the desk preview run `BacktestRunner` over a short recent window with an intent-only venue option, returning decisions, intents and holds with reasons. The separate preview loop is no longer used for v3.

**Files:** `crates/api/src/routes/agent_preview.rs`, `crates/backtest/src/preview.rs`.
**Acceptance:** a v3 preview and a v3 backtest over the same window produce the same decisions (a test compares traces); preview reports the first bar each signal could have fired and why it held before.
**Traces:** FEAT-007 §9.4, §10; ADR-0039.
