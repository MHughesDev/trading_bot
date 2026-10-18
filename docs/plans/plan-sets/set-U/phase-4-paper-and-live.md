# Phase 4 — Paper and live

**Completion: 100% (7 / 7 tasks).** U-4.1, U-4.3–U-4.6 DONE 2026-10-02; U-4.2 and U-4.7 DONE 2026-10-04.

**Requirement IDs:** SU-LIVE-01…06.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §9.5, §11; ADR-0038, ADR-0039, ADR-0042, ADR-0047.
**Depends on:** Phases 1 and 2.
**Blocks:** Phase 8; Set T Phase 1 (deployment record).
**Migration range:** 0142–0144.

The same engine, a different clock and a different venue. This phase is also where paper deployments are finally *recorded* — the missing writer that has kept forward evidence at zero.

---

### U-4.1 LiveRunner — L
`DONE` 2026-10-02 — `apps/platform/src/live_runner.rs`. Stage 2 now also publishes each
completed minute to `live_bars` (a broadcast hub); `live_deployments` runs one `LiveRunner`
task per declared v3 deployment, warmed from `BarStore` for every declared input — plus the
minutes already elapsed in each longer timeframe's open bucket, so the first live hourly bar
is whole — and folds minutes into each input's timeframe on UTC boundaries. The runner is
the async twin of `strategy_engine::cycle`: event → models → decision → act → journal →
checkpoint → refusals back. Orders go through the process-wide risk context, the
`RiskGate` and the `ExecutionEngine` (journaled before submission; keys
`v5(deployment, tag)`, so a restart's resend is a no-op); before each decision the runner
`sync_order`s its working orders so resting fills reach the engine first. Minutes already
held (warm-up, duplicate delivery) are not folded twice. Evidence:
`an_ema_strategy_fires_on_paper_from_live_bars_alone` — `ema(close,7) > ema(close,21)` on
trades fed through the pipeline's own `Minute1Aggregator`, five per minute, with no feature
value from anywhere: it places orders that fill on the paper book; one bar-close decision
per completed minute; engine steps = minutes + order events, under a third of the trades.
Not exercised: the supervisor and hub against a running platform (Docker was down and the
platform starts live collectors).

Per deployment: compile, warm up from `BarStore`, subscribe to the live bus for every declared (instrument, timeframe), step the engine on decision ticks only (ADR-0042), route through the risk gate and execution engine to the venue, feed fills and order updates back.

**Files:** `apps/platform/src/live_runner.rs`.
**Acceptance:** an `ema(close, 7) > ema(close, 21)` strategy deployed to paper fires from live bars alone — no injected feature values (the defect FEAT-007 §1 #2 names); the engine is stepped once per closed bar, not per tick (counted).
**Traces:** FEAT-007 §9.5; ADR-0039; ADR-0042.

### U-4.2 Deployment record — M
`DONE` 2026-10-04 — the two clauses left open, each checked against a reader that now exists:
"starting capital is read by reconciliation" and "Set T's `paper_deployment` readers find rows".
`live_deployments::tests::a_recorded_forward_deployment_is_found_by_its_readers` (fresh PostgreSQL,
every migration): `POST /api/deployments` with forward evidence through the platform's own router
writes the record and the `mlops.paper_deployment` row; the supervisor's resume reads the record
back with its capital (2500); `forward_spec` finds the `paper_deployment` row through the record
(trial, strategy id = deployment id, bar seconds, thresholds exact); reconciliation's
`ForwardRecorders::deploy` is built from that capital (a zero capital is refused, so the value is
consumed). Further readers of the record, from Set T: the monitor enumerates running
`strategy_deployment` rows each cadence (`monitor/tests/measurement_reaches_the_gate.rs`, as
`app_role`), and the deployment path's overview and proposal gate read it
(`api/tests/deployment_path.rs`). The first clause — every running deployment has a row and every
row a running or closed deployment — is the 2026-10-02 evidence below.

Built 2026-10-02 — migration 0142 `strategy_deployment` (revision, the graph itself,
instrument, `environment = 'paper'`, `venue = 'platform_paper'`, starting capital, model pins,
author; fixed at start by trigger, closed once; one running row per user/instrument/strategy
by partial unique index; `forward_deployment_id` for Set T's record to sit in front);
`storage::postgres::deployments`; `POST/GET/DELETE /api/deployments` write the row **before**
declaring the deployment and close it on withdrawal or on a failed declaration; the
supervisor re-declares every running row at boot and closes the row of a deployment that
cannot start or halts. Evidence: `storage/tests/pg_strategy_deployment.rs` against a scratch
PostgreSQL 17 with 0142–0144 applied — recorded at start, duplicate running binding refused,
stop closes once, edits refused, capital readable.
Until 2026-10-04 two clauses were unmet: "starting capital is read by reconciliation" and "Set T's `paper_deployment`
readers find rows" — `deployments::starting_capital` exists, but no reconciliation code reads
it and no Rust code reads `mlops.paper_deployment` at all (T-0.4's recorded gap). Closing
this is Set T T-1.1/T-1.3: write `paper_deployment` alongside this row when a deployment is
forward evidence, and build `ForwardRecorder` from `starting_capital` at boot.

Starting a deployment writes the deployment row (revision, target, environment, starting capital, pinned model versions, who started it); stopping closes it. This is the writer `mlops.paper_deployment` never had, shaped so Set T's proposal path can sit in front of it.

**Files:** `crates/storage/src/postgres/deployments.rs`, `crates/api/src/routes/deployments.rs`, migration 0142.
**Acceptance:** every running deployment has a row and every row a running or closed deployment; starting capital is recorded and read by reconciliation; Set T's `paper_deployment` readers find rows.
**Traces:** FEAT-007 §1 defect 10, §9.5; Set T T-1.1, T-1.3.

### U-4.3 Asynchronous model resolution — M
`DONE` 2026-10-02 — every request of one tick is asked at once (`join_all`), each under its
contract's `timeout_ms`; an unanswered request is left unanswered and the engine records
`missing(model_timeout)`. Responses are ordered as the engine asked, so a trace is
deterministic whichever model returned first. Live serving is Phase 5: production uses
`NoLiveModels` (every request `unavailable`, every dependent rule held). Evidence:
`a_model_past_its_timeout_holds_on_time` (a 400 ms model under a 50 ms contract: the rule
holds with `ModelTimeout{timeout_ms: 50}` in under 300 ms, journaled as `model_timeout`);
`two_models_in_one_layer_are_asked_at_once` (two 200 ms models in under 380 ms; a mutation
asking them in turn fails it).

The LiveRunner resolves `NeedModels` concurrently through the model provider with each contract's timeout; a timeout resumes with `missing(model_timeout)`. The hot path never blocks on a model.

**Files:** `apps/platform/src/live_runner.rs`.
**Acceptance:** a model that takes longer than its timeout yields a `hold` with `model_timeout` in the journal, on time; two models in one layer are requested concurrently (timed).
**Traces:** FEAT-007 §8.2, §8.4; ADR-0043.

### U-4.4 Checkpoint and restore — M
`DONE` 2026-10-02 — `strategy_engine::checkpoint`: a checkpoint is the planner state (armed
brackets, the tag sequence), the tick and the last bar per input; indicators and stateful
operators are *rebuilt* by replaying stored bars as warm-up, not stored twice. The runner
checkpoints after every decision with its working orders (migration 0143, latest wins).
Evidence: `checkpoint_restore.rs` — an engine restored at bar 90 decides byte-for-byte as one
that never stopped through bar 160, orders and tags included (a mutation dropping the planner
restore fails it); a checkpoint from another revision is refused.
`a_restart_mid_position_resumes_without_a_duplicate_entry` — a runner killed while holding a
position and restarted from its checkpoint places exactly the orders, tag for tag, of a runner
that never stopped, and the journal numbering continues; a different plan is refused at start.

Engine state (temporal operators, warm-up position, OCO bookkeeping) checkpoints after each decision; a restart restores and resumes instead of re-entering.

**Files:** `crates/strategy-engine/src/checkpoint.rs`, migration 0143.
**Acceptance:** kill the process mid-position, restart, and the next decision emits no duplicate entry (a test); a checkpoint from a different plan hash is refused.
**Traces:** FEAT-007 §9.5; ADR-0041.

### U-4.5 The decision journal — M
`DONE` 2026-10-02 — migration 0144 `strategy_decision_journal`: one row per decision per
deployment (`(deployment_id, seq)` key), with the event, account and instrument spec the
engine saw (`inputs` + its digest), the full trace, the gate's verdict per order and the
`missing` reasons; append-only by trigger; `prune_older_than` run daily by the supervisor at
the 90-day retention. A failed write halts the deployment (`LiveError::Journal`, the record is
closed with the reason) — never swallowed. Evidence: one record per decision, numbered
without gaps, each carrying inputs and verdicts (`an_ema_strategy_fires…`);
`a_journal_that_cannot_be_written_halts_the_deployment`; append-only, unique per seq and
pruned by age in `pg_strategy_deployment.rs`. "Enough to replay the step" is asserted by
Phase 8's harness, which does not exist yet; the record carries the event, view and spec it
will need.

`strategy_decision_journal` stores one record per decision per deployment — input digest, model requests and responses, intents, planned orders, gate verdicts, missing reasons — with retention per the data budget.

**Files:** `crates/storage/src/postgres/decision_journal.rs`, migration 0144.
**Acceptance:** every LiveRunner decision writes exactly one record; a record carries enough to replay the step (asserted by Phase 8's harness); journal write failure is surfaced, never swallowed.
**Traces:** ADR-0047.

### U-4.6 v3 deployments through the one live registry — M
`DONE` 2026-10-02 — `strategy_runtime::live::V3Deployment` lives in its own map with its own
generation; stage 3 reads only `snapshot_for`, which cannot return one, and declaring one does
not even wake stage 3. The runner checks `is_v3_declared` before every bar. Evidence:
`a_v3_deployment_is_invisible_to_stage_3`; `a_withdrawn_deployment_stops_within_one_decision_tick`
(withdrawn, the next bar ends the runner, reported as withdrawn rather than halted).

### U-4.7 A paper account per deployment — M
`DONE` 2026-10-04 — `PaperTradingEngine::open_deployment_account` gives a deployment its own
`PaperAccount` (own cash, positions, ledger and day's realized P&L), held in slots that are filled
once and never emptied so its lock borrows from the engine exactly as an asset-class account's does,
with the same poison recovery. An order routes to its deployment's account when its `strategy_id`
names one opened on that asset class (frozen on the order record at submit), else to the class's;
`positions_for` / `snapshot_for` / `daily_loss_usd_for` read the same way, and settlement and funding
reach every account holding the instrument. The live runner's view and position read its own
account; `live_deployments` opens it (a selection's members keep sizing against their sleeves); the
risk context gates a deployment's order on its own position and day's loss. Durable through
`paper_deployment_account_state` (0188: the 0153 state shape and version; a row the binary cannot
read refuses boot; app_role without DELETE). Evidence: engine tests — two deployments on one
instrument each see only their own position and cash while the class account trades nothing; a
manual order and an unopened deployment still use the class account; reduce-only reads the
deployment's own position; settlement reaches it; export/restore round trip, never reseeded;
`paper_deployment_accounts_durable` on scratch PostgreSQL 17 (a fresh engine restores each account's
position and exact cash); platform `a_deployment_with_its_own_account_sizes_and_trades_in_it` (equity
starts at its capital, fills land in its account, none in the shared one); execution 23 suites,
platform 63, api 528. Lands with the Set E paper-persistence work it extends.

Found building U-4.1. The paper engine keeps **one account per asset class**, so every
deployment on that class shares cash, equity and — on one instrument — position with every
other deployment and with manual paper trading. A deployment's `state.position` is the
account's position in its instrument, and its recorded starting capital is not the balance it
trades. Give each deployment its own paper sub-account, funded with its starting capital, and
read its view from that.

**Files:** `crates/execution/src/paper/`, `apps/platform/src/live_runner.rs`.
**Acceptance:** two deployments on one instrument each see only their own position and
equity; a deployment's equity starts at its recorded starting capital; manual paper trading
is unaffected.
**Traces:** FEAT-007 §9.5; Set T T-0.4 (comparable realised series).

ADR-0038's registry declares v3 deployments; the LiveRunner is what runs them. v1 strategies keep hot-path stage 3 until Phase 8.

**Files:** `crates/strategy-runtime/src/live.rs`, `apps/platform/src/hot_path.rs`.
**Acceptance:** a v3 deployment is never also evaluated by stage 3 (asserted); stopping a deployment stops its runner within one decision tick.
**Traces:** ADR-0038; ADR-0039.
