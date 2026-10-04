# Phase 3 — The backtest environment

**Completion: 100% (5 / 5 tasks).** U-3.1, U-3.2, U-3.3, U-3.5 DONE 2026-10-02; U-3.4 DONE 2026-10-04 (its HTTP criterion run in process against the platform's own router and state).

**Requirement IDs:** SU-BT-01…05.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §9.1, §9.4; ADR-0039, ADR-0046.
**Depends on:** Phases 1 and 2.
**Blocks:** Phases 6, 8.
**Migration range:** none.

The engine meets history. When this phase closes, a v3 strategy backtests through exactly the code that will run it on paper, gated, journaled, and in the shape Set J already judges.

---

### U-3.1 Environment ports — M
`DONE` 2026-10-02 — `strategy_engine::env`: `Clock`, `MarketData`, `ModelProvider`,
`VenuePort` (the plan's `Venue`, named apart from `execution::Venue` because it speaks
planner tags, not venue order ids) and `Journal`, plus **`cycle`** — the one loop that
answers model requests, journals the decision, hands its actions to the venue and feeds
the venue's synchronous refusals back through the engine. It lives beside the engine, not
in each environment, so paper (Phase 4) drives the same function the backtest does. The
backtest environment is `backtest::env` (`MergedBars`, `SimOrders` over `SimulatedVenue` +
`SimulatedRisk`, `IntentOnly`, `TradeBook`). Evidence: `strategy-engine/tests/env_cycle.rs`
— a mock environment drives a bar through a model request, a sized order, a journaled
decision and a refusal fed back; the Phase 1 Cargo.lock guard still finds no path from
`strategy-engine` to execution, risk, storage, model-registry, tokio, sqlx or reqwest.

Traits `Clock`, `MarketData`, `ModelProvider`, `Venue`, `Journal`, and an `Environment` that owns one of each plus the risk gate and execution engine. The engine depends on none of them.

**Files:** `crates/strategy-engine/src/env.rs` (traits only), `crates/backtest/src/env/`.
**Acceptance:** `strategy-engine` has no dependency on `execution`, `risk`, `storage` or `model-registry` (a Cargo-graph test); a mock environment drives a full decision cycle in a unit test.
**Traces:** FEAT-007 §9.1; ADR-0039.

### U-3.2 BacktestRunner — L
`DONE` 2026-10-02 — `backtest::runner`: `run` (pure: compiled plan + loaded bars →
`V3Outcome`) and `V3RunExecutor` (resolves the exact revision from `strategy_revision`,
else the head; materialises params through `compile`; loads warm-up plus window for every
declared (instrument, timeframe); refuses an instrument with no metadata, as the live gate
does). Bars of every input merge by close, other inputs before the primary at the same
instant; each primary bar runs through the venue first so its fills reach the engine before
its close is decided; every order passes `SimulatedRisk`. **Model requests are answered by
`NoModels` until Phase 5** — a plan that calls a model backtests with those nodes `missing`
and their rules held, counted in the manifest, never guessed. Scheduled triggers are refused
by name until a calendar exists to drive them. Evidence, `backtest/tests/v3_runner.rs`: an
EMA-cross strategy backtests end to end (440 decisions, every round trip net of fees); a
bracket strategy with no exit rule exits only through its stop or target, legs placed on
the entry's fill; an ETH-1h strategy reading BTC-1d sees, at every decision, exactly the last
daily bar closed by that instant, including the hour that closes on the day boundary (a
mutation putting the primary first at equal instants fails it);
`every_v3_run_is_in_the_trial_ledger_before_it_computes` — the executor is reachable only
with a `TrialTicket`, and the run's trial is registered first and settled with its `run_id`.

Resolve the revision, register the trial (INV-16), materialise params, compile, load warm-up plus window for every declared (instrument, timeframe) point-in-time, merge them into one time-ordered stream, step the engine on decision ticks, resolve model requests, route orders through the risk gate to the SimulatedVenue, feed fills and order updates back as engine events.

**Files:** `crates/backtest/src/runner.rs`.
**Acceptance:** an indicator strategy and a bracket strategy each backtest end to end; a strategy reading BTC 1d while trading ETH 1h sees only BTC daily bars closed by the decision instant; every run appears in the trial ledger before it computes.
**Traces:** FEAT-007 §9.4; ADR-0039; INV-16.

### U-3.3 Results Set J can judge — M
`DONE` 2026-10-02 — `RunResult.manifest: Option<EngineManifest>` (serde-defaulted, so every
stored run still parses): engine, revision, fill policy and whether it is optimistic, venue,
risk limits, model pins, folds (empty until Phase 5), decisions, holds and unknowns by
`missing` reason, planner refusals by reason, every gate rejection with its check, a
SHA-256 journal digest over every trace, and the journal entries that acted. Trades, equity
and net exposure go through the existing `map_sim_result`, so metrics are the standard
`MetricSet`. Evidence: `a_set_j_study_accepts_v3_runs_unchanged` runs a three-member
parameter sweep through `StudyEngine::run` with no change to the study code (three trials,
three surviving members, three distinct journal digests);
`the_fill_policy_is_part_of_the_run_and_flagged_when_optimistic` — `current_close` changes
the `run_id`, marks the manifest and adds a `fills.optimistic` integrity flag.

Write the trade list, daily return series, equity curve, gate rejections, model pins and fold usage, execution policy, and the decision journal into the run artifact and the existing results tables, so experiments, nulls, the funnel and `/experiments/{id}/returns` consume v3 runs unchanged.

**Files:** `crates/backtest/src/run/result.rs`, `crates/backtest/src/runner.rs`.
**Acceptance:** a v3 run is accepted by the Set J funnel and studies without a code change there; the manifest names fill policy, model versions and folds, and the count of `missing`-driven holds by reason.
**Traces:** FEAT-007 §9.4; ADR-0019; ADR-0021.

### U-3.4 Backtests of v3 graphs through the job service and API — M
`DONE` 2026-10-04 — the HTTP criterion, run without starting the platform:
`crates/api/tests/v3_backtest_over_http.rs` builds the platform's own `AppState` and router in
process exactly as `main.rs` does — nothing spawned, no collector, no pipeline — over a scratch
PostgreSQL with every migration applied and a scratch ClickHouse database holding ten days of
stored BTC-USD hourly bars. `POST /api/backtests` with an inline EMA-cross graph answers 201
(`engine: strategy-engine/v3`); the job runs through the backtest manager's check/load phases and
`runner::run`; `GET /api/backtests/{id}` reads the tile back `completed` with the run's result —
239 decisions, 6 orders, 3 trades. The same file proves the door: a graph that does not compile
and a timeframe that is not the graph's answer 422, an unsigned caller 401, another person's
token cannot read the tile (404). v1 submissions keep their existing tests.

Built 2026-10-02 — `POST /api/backtests`
takes an inline `graph` or a stored slug whose document is v3, compiles it (422 with the
compiler's diagnostics), refuses a timeframe that is not the graph's, and hands the manager
a `JobStrategy::V3`; the manager's `drive_v3` runs the same check/collect/load phases for
every input and then `runner::run`, storing the `RunResult` as the job's result document.
Experiments and Studies reach v3 too: `AppState` now builds `FormatRoutedExecutor`, which
reads the stored document's format and sends v3 to `V3RunExecutor`, v1 to `SimRunExecutor`
— without it the new executor had no production caller. v1 is untouched: `cargo test -p
backtest` 437/437 lib and every integration suite, `-p api --lib` 421/421.
Until 2026-10-04 "a v3 backtest submitted over HTTP runs and reports" was unrun: it seemed to
need the platform up, and it was not started — Docker was down, and starting the platform starts the live market-data
collectors for initialised assets, which this session is not to do. What is tested without
it: `JobStrategy` round-trips both formats as the stored document, a request carrying a
graph parses, and everything after the HTTP layer runs in `v3_runner.rs`.

`/api/backtests` and the backtest job worker accept a v3 revision and run it through `BacktestRunner`; v1 revisions keep the old path until Phase 8.

**Files:** `crates/api/src/routes/backtests.rs`, `crates/backtest/src/manager.rs`.
**Acceptance:** a v3 backtest submitted over HTTP runs and reports; a v1 submission is unaffected (existing tests).
**Traces:** FEAT-007 §9.4, §12.

### U-3.5 Preview is a short backtest — M
`DONE` 2026-10-02 — `backtest::preview::preview_v3` runs the backtest runner's own loop
over the window and reports decisions only — per rule: first fired, fired and held counts,
and what stopped it before it first fired (`not_matched`, `unknown:<reason>`,
`held:<reason>`); the decisions that acted, held or were refused; and the journal digest.
No equity, trades or P&L, and no trial. `PreviewVenue::Simulated` (default, identical to a
backtest) or `IntentOnly` (orders recorded, never filled). `/api/agent/strategy/preview`
routes a v3 document there under the same window rule (cutoff clips, holdout refuses) and
point-in-time read as v1; v1 keeps its preview. Evidence:
`a_preview_and_a_backtest_over_the_same_window_decide_identically` (traces equal, digests
equal, no money fields); `a_preview_names_the_first_bar_each_rule_fired_and_why_it_held_before`
(first firing matches the trace; `unknown:warming_up` and `not_matched` before it);
`an_intent_only_preview_records_orders_and_never_fills_them`.

`/api/agent/strategy/preview` and the desk preview run `BacktestRunner` over a short recent window with an intent-only venue option, returning decisions, intents and holds with reasons. The separate preview loop is no longer used for v3.

**Files:** `crates/api/src/routes/agent_preview.rs`, `crates/backtest/src/preview.rs`.
**Acceptance:** a v3 preview and a v3 backtest over the same window produce the same decisions (a test compares traces); preview reports the first bar each signal could have fired and why it held before.
**Traces:** FEAT-007 §9.4, §10; ADR-0039.
