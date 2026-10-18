# Phase 7 — The monitor

**Completion: 100% (13 / 13 tasks)** — 2026-10-04: S-7.10 done (value and limit on the wire and on the page).

> **2026-10-03, track MONITOR.** The measurement layer this page called missing now
> exists: feed staleness (0165 gauge → `assess_feeds`), behaviour inputs from the
> strategy's own fills and journal, daily returns for the P&L family, per-venue
> reconciliation verdicts that a resume is held to, the promotion gate as a schema
> trigger, and a push channel. Migrations **0165, 0166** (this track's range
> 0165–0169). Live-PG tests: `crates/monitor/tests/measurement_reaches_the_gate.rs`.
>
> **2026-09-27.** S-7.11 closed: it was `BLOCKED:UPSTREAM` on S-3.18, S-3.18
> landed, and both of its acceptance clauses are asserted by the whole-tree guard
> that came with it. **Nothing in this phase is blocked any more**, and with it the
> last `BLOCKED:UPSTREAM` in Set S is gone.

The fail-closed core — the separate process, the lease, the scoped durable switch, the human-only
reset, and dwell — is complete and drilled against a live database. What is partial is almost
entirely one thing: **the monitor decides correctly but does not yet measure.** There is no
collector loop reading fills, bar ages or venue errors, so the execution, behaviour, data and
venue families have a tested decision layer with no inputs. Read the per-task notes before
treating any of those as working.

**Requirement IDs:** MS-01…MS-14.
**Spec:** [COMP-007](../../../specs/COMP-007-live-monitor-and-kill-switch.md), extending
[COMP-002 §3](../../../specs/COMP-002-execution-and-risk-gate.md).
**Depends on:** Phase 0 only. Fully independent of the agent work — start it on day one.
**Blocks:** the monitor page (S-6.21) and the read-only `check_control_limits` tool (S-3.18).
**Migration range:** 0074–0078.
**Migrations shipped:** 0074, 0075, 0076, 0077, 0078 (merged 2026-09-15, integration wave 4).
**Parallel with:** everything.

`crates/risk/src/kill_switch.rs` is an `AtomicBool` with `trip()` and `reset()`, persisted by
"the caller". Nothing trips it automatically, nothing scopes it, nothing records why, and it
lives in the same process as everything it would stop. The operational finding this phase is
built on: **a kill switch must run in a different process from the strategy, or the failure
that requires killing also disables the kill.** The second: P&L is a lagging signal — fill
quality, signal frequency and holding-period drift flag a broken strategy earlier than its
equity curve does.

Auth is real here, which is what makes the human-only rule enforceable:
`crates/api/src/auth/session.rs` resolves a bearer token against `sessions` with an expiry
check and returns its scopes, and `BearerToken::permits` gates on them. (The
`COMPLACENCY_LOG` entry calling this CRITICAL is stale — S-0.6 corrects it.)

---

### The halt stopped erasing itself — and the sweep was refused, 2026-09-21

`check_position_divergence_at_venue` now raises through
`MonitorStore::carry_out_mismatch`, which writes the durable
`monitor.kill_switch` row **first** and mirrors `trip_scope` second. **The bare
`trip_scope` is gone from `crates/reconciliation` entirely**, and a test asserts
that — not a comment asking callers not to use it.

`a_reconciliation_halt_is_still_a_halt_after_the_mirror_refresh` builds a
`LeaseWatcher` exactly as `AppState` does, against the same database, and calls
`.refresh().await` **between** the halt and the assertion. Reverted to the
pre-fix body it fails, and it fails *after* passing the "immediately halted"
line — which is the whole point: the broken version halts, and only the real
refresh catches that the halt was in memory. The gate assertion goes through
`RiskGate::check` **with a mark price**, because a `None` there makes every
assertion true via `MarketDataUnavailable` — the trap that had six COMP-007
sections resting on it.

`compare_position` was split out as pure arithmetic that **raises nothing**, so
"you cannot reach a halt without naming a venue" is structural. `SweepError` has
`Unreadable` and `NotHalted` and **neither can be spelled as a clean result**. A
sweep halts a venue **once** however many instruments disagree, so one incident
is one `monitor.events` row.

### The sweep was refused, and the reason is the rule

The audit was right that `apps/platform` declares `reconciliation` and never
uses it — **proof: four public signatures changed and `cargo check -p platform`
still passes.** But wiring the 30-second loop was refused, and correctly:

`main.rs` builds the only `ExecutionEngine` on `paper_engine.multi_asset_broker()`,
and `PaperBroker::query_positions()` returns `self.engine.positions(...)` — **the
same `Arc<PaperTradingEngine>` the internal book is read from**. Every real venue
adapter (`Alpaca`, `Kalshi`, `Oanda`, `Tradier`, `Tradovate`, `ZeroX`) is
constructed **only** in `crates/execution/tests/venue_adapters.rs`. There is no
live broker in the running process.

So the sweep would compare a value with itself and log *"position reconciliation
OK"* every thirty seconds for a check that **cannot fail** — a reconciliation
verdict nothing measured, feeding an `all_venues_reconciled` proxy that would
keep reading clean. An absent measurement rendered as a passing one, on the
control that decides whether our book and a venue agree about what is owned. The
reason is recorded in `positions.rs`'s module doc, **which previously claimed
the loop existed**.

### Still open, and each needs something outside that track's reach

- **The global reconciliation halt has the same erasure defect.**
  `check_position_divergence` (the venue-less form, reached by
  `DivergenceMonitor::on_position` → `ForwardRecorder::reconcile_position`) calls
  `KillSwitch::trip()`, and `replace_halted` also does `self.global.store(global)`
  from the table — so a global trip with no `scope='global'` row is erased within
  one cadence too. Closing it needs a global `Fact::ReconciliationMismatch`, or
  venue attribution the recorder does not have: `DeploymentSpec.venue` is
  `PaperVenue::Platform` → `"platform_paper"` (was `AlpacaPaper` until ADR-0048, 2026-10-02), which is **not** the `venue_id`
  the gate sees from instrument metadata, so halting on it would refuse nothing.
  Documented on the function; behaviour left alone rather than a venue name
  invented.
- **No table holds a real per-venue reconciliation verdict**, so
  `all_venues_reconciled = !any venue halted` stays a proxy. Recording "venue X
  was checked at T and agreed / disagreed / could not be read" needs a migration.
- **`monitor.lease` is a single global row** (`lease_id` is hard-coded to
  `LEASE_ID`; the drills vary the *holder*). **Any test that takes the lease
  opens the platform's order gate for one cadence, process-wide.** The new test
  deletes its row on the way out; the monitor drills do not delete theirs.
- **`crates/strategy-runtime/Cargo.toml` declares `reconciliation` and uses
  nothing from it** — a second dead dependency of the same shape.
- `crates/dataplane/src/census.rs:315` trips `clippy::manual_checked_ops`, which
  fails a literal `-D warnings` run before it reaches any other crate.


### Five of eleven fail-closed tests had never met a database, 2026-09-21

**190 lib + 6 + 14 + 4 tests**, clippy `-D warnings` clean. But the counts are
not the finding.

**COMP-007 §8.1, §8.2, §8.3, §8.5, §8.6 and §8.7 all name
`tests/fail_closed_drill.rs` as "Verified by".** Those tests return early
without `MONITOR_TEST_DATABASE_URL`. Run against live Postgres for the first
time, **five of the eleven failed.** Two causes, both now fixed:

- The harness's `ctx()` passed `None` for the mark price, so `RiskGate::check`
  refused with `MarketDataUnavailable` **before reaching the kill switch at
  all**. Every `is_ok()` assertion was false and every `Err(..)` assertion was
  true for the wrong reason — including *"the other venues keep trading"* and
  *"a live lease opens the gate"*. Six safety sections were resting on it.
- `a_second_reset_within_five_minutes_is_refused_by_the_store` re-tripped with
  the **same** `MonitorEvent`, so the second insert died on `events_pkey` rather
  than on the five-minute rule. A second incident is a second event.

**A test that skips silently is not a test, and a spec section that cites one is
not verified.** That is the reusable part.

### Three phase notes claimed built what was not

**S-7.13 said windows and their platform-only suppression were "built, tested
and wired into CI".** `MaintenanceWindow::suppresses` and `suppressing_window`
had **no caller outside `window.rs`'s own tests**; `MonitorEvent::suppressed_by`
had **no caller at all**, though migration `0075` gave `monitor.events` a
`suppressed_by_window` column to hold the answer; and no platform-health trip
existed for a window to suppress. **A window suppressed nothing, so every deploy
still halted trading** — the exact failure §4.3 was written to prevent.

**Same task: "drill mode ... built".** `DrillOutcome::record` took its three
latencies **as arguments**, and its only caller was `POST /api/monitor/drill`,
which read them from the request body. *"A drill whose propagation exceeds one
cadence fails the deploy"* was arithmetic over numbers the caller typed.
`rehearsal.rs` now measures detection at trip commit, propagation through the
watched process's own `replace_halted` → `is_active_scope` path, and
notification as the record becoming readable on `monitor.events` — the only
channel that exists — recorded in `notes` so the number cannot be misread as the
desk-banner latency nobody built. *"Refused nothing real"* is **measured** by
diffing the halted set, not assumed; a rehearsal that cannot observe propagation
returns an error rather than a recorded verdict.

**S-7.6 said the resume block was "built and drilled", naming
`an_injected_mismatch_halts_one_venue_and_blocks_resume_until_clean`.** That
test never attempts to resume the halted venue — it asserts only that a
**global** reset is refused. `ResetRequest::validate` applies
`all_venues_reconciled` to `Scope::Global` alone, so **a venue halted because it
and our book disagree about what is owned can be resumed today** with a typed
string, a reason and five minutes' patience, with no proof the mismatch cleared.
§8.3 is not enforced. Closing it needs a migration and an API change, both
outside that track's territory.

**S-7.5 was over-claimed in its reason, and the narrower truth is the useful
one.** The note says last-bar age "lives in ClickHouse and `crates/monitor` has
no client for it and must not grow one". There **is** a Postgres table with
exactly the right shape — `dataplane.data_coverage_census`. It is unusable for a
different reason: **0 rows**, its only writer `take_census` has no caller
anywhere outside its own module, and it is an append-only *census series* keyed
on `as_of`, not a live gauge — so `now() - last_bar` over-states staleness
whenever bars arrived after the snapshot, **which is the false-halt direction**.
Same for `dataplane.instrument.last_seen`: migration `0046` grants
`UPDATE (last_seen)` to `app_role`, no Rust anywhere writes it, and all ten rows
are NULL.

### What the live database said that the plan did not expect

- **`monitor.heartbeat` grows one row per monitor boot, for ever.** Keyed
  `(component, instance)` with `instance` a per-boot id, and nothing evicts.
  **26 stale `tbot-monitor` rows were found.** Under a row-wise reading of §4's
  *"missed 3 heartbeats → global halt"*, that is **26 standing reasons to halt
  the entire book**, armed by ordinary restarts. `platform::assess` therefore
  judges a component lost only when *every* instance of it is stale, and never
  judges the monitor's own component — both held by tests, not comments. The
  eviction itself needs a migration and is not done.
- **No `tbot-monitor` container is running** — `docker ps` shows postgres, nats,
  redis and clickhouse only, and the platform runs as a native binary. With
  `monitor.lease` at 0 rows the API's order gate is refusing every order with
  `monitor_unavailable` **right now**. Fail-closed working as designed, and
  **nothing in this file says the default local stack has no monitor in it**.
- **The drill suite leaks rows when a test panics**, and a leaked `venue:*` halt
  has a real consequence: `routes/monitor.rs` derives
  `all_venues_reconciled = !halted.iter().any(Venue)`, so **one abandoned test
  row refuses every global reset**. The track cleaned up everything its runs
  left (7 switch rows, 11 events, 26 heartbeats, 1 lease) and verified the
  database was back to empty with `trading_enabled = t`. Cleanup that survives a
  panic across all six tests is not done.
- **That derivation is a proxy, not the fact.** A venue halted for a connection
  loss reads as "not reconciled"; a venue that failed reconciliation and was
  never halted reads as "clean". The real per-venue verdict has nowhere to live.
- `monitor.drill.notes` existed and `record_drill` never wrote it.
  `mlops.divergence_halt` is `FORCE ROW LEVEL SECURITY` per-tenant and holds
  per-strategy Set-T halts — it is **not** the S-7.6 record.
- **A hazard found and fixed mid-build:** a rehearsal runs on the loop that
  renews the lease, so an unbounded one would lapse the lease and **halt the
  platform for real while rehearsing how the platform halts**.
  `Rehearsal::within` is bounded to half the renewal interval, asserted across
  four cadences.

### Problems in the spec, neither reading implemented

§4.2 lists three latencies and the schema makes all three `NOT NULL`, but the
notification channel it describes — *"the desk banner appears marked DRILL"* —
does not exist. A zero there reads as *"the notification landed instantly"*,
which is the dangerous direction, and `Measurement` has no `Default` precisely
so this cannot happen elsewhere. The channel that exists was measured and the
drill says which one it was, rather than a reading being picked.

§4's *"platform heartbeat loss → global halt"* **has no subject**: nothing in the
spec says which components must beat, and today only `tbot-monitor` does — into
a table that never forgets. Left open; the code judges only components that
beat, and never the monitor itself.


## Tasks

### S-7.1 `crates/monitor` — a separate process — M
`DONE` — verified by `monitor::independence::{the_monitor_does_not_depend_on_the_api,
the_monitor_ships_its_own_binary, the_monitor_binary_opens_its_own_database_connection}`.
Joined the workspace as a path dependency of `crates/api` rather than by editing the root
manifest (ADR-S7-01); `tbot-monitor` runs as its own container in `docker-compose.yml`.

A new workspace member and a `tbot-monitor` service: its own Postgres connection, its own
NATS subscriptions (fills, orders, bars, heartbeats), its own heartbeat, and **no dependency
on the API process**. Adding a crate means `static_invariants` starts scanning it — expect
AT-01/AT-10/AT-15/AT-19 to apply.

**Files:** `crates/monitor/`, `Cargo.toml` (workspace members — a known cross-track conflict
point), `docker-compose.yml`.
**Acceptance:** the monitor runs with the API stopped and keeps recording; `just lint` clean
under `pedantic`.
**Traces:** COMP-007 §1 P1, §6.

### S-7.2 The lease — fail closed — M
`DONE` — verified by `monitor::fail_closed_drill::killing_the_monitor_stops_order_flow_within_one_cadence`
(a real lease in a real database, allowed to lapse),
`monitor::fail_closed_drill::a_database_the_runtime_cannot_read_closes_the_gate`, and the whole
of `risk::lease_fails_closed`, including a source scan proving no code exists that disarms it.

The monitor renews a `monitor_lease` row every cadence. The runtime's order path checks it;
no valid lease → every new order is refused with `monitor_unavailable`. This is how "if the
monitor dies, the platform stops" is enforced without the monitor being in the process.

**Files:** `migrations/0074_monitor_lease.sql`, `crates/execution/`, `crates/risk/`.
**Acceptance:** killing the monitor causes order refusal within one cadence; killing the
**API** does the same from the runtime's side; restarting does not resume orders until the
lease is valid again.
**Traces:** COMP-007 §4 ("fail closed"); §8.1, §8.2.

### S-7.3 The scoped, persisted kill switch — M
`DONE` — verified by `monitor::fail_closed_drill::a_trip_survives_a_restart_and_the_boot_path_agrees`,
`monitor::fail_closed_drill::{the_database_refuses_an_agent_tripped_or_agent_reset_row,
no_agent_tripped_row_exists_anywhere}` and `risk::kill_switch::tests::*`. The hot-path check is a
plain atomic load plus a lock-free map lookup; `tripped_by`/`reset_by` carry CHECK constraints
that make an `agent:` row unwritable.

Replace the in-memory flag with `kill_switch(scope: global | venue:<id> | strategy:<id>,
state, tripped_by, reason, metric?, value?, limit?, tripped_at, reset_by?, reset_reason?,
reset_at?)`. The runtime keeps an in-memory mirror refreshed on lease renewal and NOTIFY, so
`is_active(scope)` stays an atomic read on the hot path. `tripped_by` has an `agent:` shape
that must never appear — the column exists to prove it stays null.

**Files:** `migrations/0075_kill_switch.sql`, `crates/risk/src/kill_switch.rs`.
**Acceptance:** the hot-path check remains a non-blocking atomic read; a trip is durable
across a restart; `tripped_by` is never `agent:*` in any test or migration.
**Traces:** COMP-007 §5; §8.6.

### S-7.4 The reset is human-only, typed, and rate-limited — M
`DONE` — verified by `monitor::control::tests::*` (typed scope, reason, five-minute rate limit,
global-needs-reconciliation, and `nothing_that_is_not_a_named_human_can_reset`),
`monitor::fail_closed_drill::a_second_reset_within_five_minutes_is_refused_by_the_store`, and
`api::monitor_authority::*` — which proves §8.6 **per token scope**, mints a real service token,
and additionally scans `routes/monitor.rs` so a route added later cannot skip the check.
The pre-existing `/api/trading/kill|resume` routes, which took any bearer token at all, are now
gated by the same scope.

Reset requires the human's typed confirmation of the scope name, a reason, and — for global —
that every venue reconciles clean first. One reset per five minutes per scope, so a flapping
fault cannot be reset into. The POST routes require a `web:full`-class scope that agent
service tokens do not carry.

**Files:** `crates/api/src/routes/monitor.rs`, `crates/api/src/auth/scopes.rs`.
**Acceptance:** a global reset without a typed scope and reason is refused; a second reset
within five minutes is refused; **no agent token can call any monitor POST**, asserted
adversarially with a minted service token.
**Traces:** COMP-007 §5, §6; §8.7, §8.6.


---

## The monitor track, 2026-09-21 — two defects that would have cost money

**Read this before building anything else on this crate.**

### The collector loop measured nothing, and now does

`MonitorService::run` renewed the lease, beat, and slept. `collect.rs` and
`pnl.rs` are new; `Cadence::assess` is pure (readings → events + transition) and
`collect_cadence` / `evaluate_strategy` / `apply` are wired into `run()`, so it is
not built-and-unreached. COMP-007 §8.4 is true from orders in to scale-down out:
ten orders, a measured fill rate of 0.4 against a declared floor of 0.8, and the
event names metric, value, limit and scope.

`Measurement` is `Measured { value, observations } | Unobserved(reason)` with **no
`Default` and no `From<_> for f64`**, so a family with no observations cannot
report a zero — which on this surface is the dangerous direction: zero reads as
*measured, and fine*.

### (a) A collector-driven halt would have refused no orders

`apply` wrote `monitor.events` and `monitor.strategy_state`. The order gate's
mirror is refreshed from `MonitorStore::halted_scopes`, which reads
`monitor.kill_switch` **and nothing else**. A strategy would have sat at `halted`
on the monitor page, with an event row explaining why, **and gone on trading**.
`apply` now routes a `Halted` decision through `MonitorStore::trip` — event and
switch row, one transaction, record first — guarded by
`a_halt_decision_carries_the_event_that_trips_the_switch`.

### (b) Automatic scale-down never crossed the process boundary

`refresh` renewed the lease and the halted scopes and never the scales. The only
existing caller of `KillSwitch::scale_strategy` is the **manual API route**, which
shares a process with the mirror it updates — so the manual path worked and the
automatic one silently did nothing. `refresh` now pushes each `scale_pct` into the
mirror, keeping the previous values on a read failure, which can only be too
restrictive.

**The general rule, and it is the thing to remember:** the manual route and the
monitor process update two different things. Anything meant to reach the order
gate must go through a table `refresh` reads, or it will pass every test and do
nothing in production. That is exactly how (a) and (b) came about.

### Still true, and the most dangerous thing on this page

**The dwell-exempt path has no production caller.** `Fact::LeaseLost` is
constructed nowhere in the repository outside one assertion in `family.rs`;
`Fact::ReconciliationMismatch` only inside `evaluate.rs`'s own tests;
`Fact::HardDataStaleness` only by `evaluate::staleness`, which itself has no
non-test caller; and `Evaluator::on_fact` is called by nothing outside tests.

So COMP-007 §4.1's *"a reconciliation mismatch, a lost lease and hard staleness
are facts, not estimates, and act immediately"* is true in the type system, true
in nine tests, and **does nothing at all in a running platform**. This is the
repository's dominant defect shape on the one surface where it costs money.

### One spec contradiction, unresolved by design

§3 says a hard-stale feed halts; §4 says a data breach scales down. The code
follows §4 and the test asserting it is *named* after §3. Written up as Q-S7-01;
nothing was changed.

### What the monitor cannot measure, and why — each is a column, not a component

`slippage_bps_p50/p95` (nothing stores the fill model's prediction — the metric
§3 lists *first* under execution quality), `signals_per_hour` (no per-strategy
signal count; it is the metric that catches a strategy that has silently stopped
trading), `position_count` (`positions` has no `strategy_id`, and joining through
`orders` attributes a shared instrument to every strategy that ever traded it —
an over-count against an upper limit is a false scale-down), and the P&L family's
daily return series and baseline, which have no storage. `mlops.forward_observation`
holds the returns but is `FORCE ROW LEVEL SECURITY` on a tenant the monitor's pool
does not set, and **a silently-empty RLS read is indistinguishable from a strategy
with no returns**.

### A caveat on the test counts

111 → 165. The 11 `fail_closed_drill` tests return early without
`MONITOR_TEST_DATABASE_URL` — they were green-but-vacuous before and after, and
**none of the SQL added here has been executed against a live database.**

### S-7.5 Data and venue families — M
`DONE` 2026-10-03 (was `PARTIAL`) — **the measurement layer now exists and the
production loop raises `Fact::HardDataStaleness` from it.** Migration 0165 adds
`monitor.feed_freshness`, a gauge (one row per live feed, overwritten) written by
the platform process that *observes* each completed bar
(`apps/platform/src/health_jobs.rs::spawn_feed_freshness`, subscribed to
`live_bars` — started from `health_jobs::spawn`, which `main` already calls).
`MonitorService::assess_feeds`, called from `run()`, judges each row
(`monitor::freshness::judge`), builds the dependency map from the running
`strategy_deployment` rows, and routes a hard-stale feed through `facts::decide` →
`MonitorStore::carry_out_fact` (event + `monitor.kill_switch` row, record first).
`Fact::HardDataStaleness` is now `Wiring::RaisedBy` and off `KNOWN_UNWIRED`
(`facts_are_wired` 7/7). A shut equity session and a silent producer are reported
as `SessionClosed` / `ProducerSilent`, never as stale and never as fresh.
Verified by `measurement_reaches_the_gate::{a_hard_stale_feed_halts_the_deployment_on_it_through_the_gate_and_no_other,
a_shut_session_halts_nothing_however_old_the_bar}` (live PG 17, as `app_role` for the
producer's writes and the watcher's refresh), `monitor::freshness::tests::*` (6) and
the platform guards `health_jobs::feed_tests::*` (3, incl. the venue table pinned
against `pipeline_manager.rs`). Mutation-checked: removing the `carry_out_fact`
call in `assess_feeds` fails the first test at the halted-scope assertion.
**Not in the acceptance and not built:** venue connection state, error rate and
rate-limit headroom, gap counts and impossible bars (`probe_data_quality`). Under
ADR-0048 every order fills in-app, so there is no venue error stream to read; gap
counts need a ClickHouse reader and stay with the census. ADR-S7-12 (new): the
gauge is keyed on the feed's *data* venue (kraken/alpaca), not the deployment's
`platform_paper` execution venue, and the edge is `strategy_deployment.instrument_id`
— rejected: keying on `instruments.venue_id`, which pins BTC-USD to coinbase while
the live feed is kraken.

Last-bar age per instrument, gap count, impossible bars (reuse `probe_data_quality`);
connection state, error rate, rate-limit headroom. Staleness > 3× timeframe warns; > 10×
halts the strategy.

**Acceptance:** a stale feed halts the strategy that depends on it and only that one; the event
names the instrument and the age.
**Traces:** COMP-007 §3.

### S-7.6 Reconciliation halts the venue — M
`DONE` 2026-10-03 (was `PARTIAL`) — **the resume block is now a fact, not a
proxy.** Migration 0165 adds `monitor.venue_reconciliation` (append-only verdicts:
`agreed` / `disagreed` / `unreadable`, detail required when not clean) and the SQL
rule `monitor.venue_reconciled(venue)`: never checked, or last check agreed.
`MonitorStore::carry_out_mismatch` records `disagreed` before it halts;
`reconciliation::positions::{reconcile_all, reconcile_with_broker}` record
`agreed` / `unreadable`. `MonitorStore::reset` now **reads both preconditions
itself and discards the caller's**: a venue reset is refused with
`ResetRefusal::VenueNotReconciled` until a later check agrees, and a global reset
is refused while any venue's last word is not `agreed` — replacing the
"no venue is halted" proxy (which read a connection-loss halt as unreconciled and
an unhalted failure as clean). Verified by
`measurement_reaches_the_gate::{a_venue_halted_for_a_mismatch_resumes_only_after_a_reconciliation_agrees,
verdicts_are_append_only}` (as `app_role`), `monitor::control::tests::a_venue_whose_last_reconciliation_disagreed_cannot_be_resumed`,
plus the existing `an_injected_mismatch_halts_one_venue_and_blocks_resume_until_clean`.
Test cleanup in `fail_closed_drill.rs` and `a_venue_mismatch_survives_the_mirror_refresh.rs`
now ends every test venue on `agreed`, because an append-only `disagreed` left
behind would refuse every later global reset on that database.
**Still unwired, and correctly:** `Fact::ReconciliationMismatch` has no production
raiser — under ADR-0048 every fill is the in-app engine's, so a sweep would compare
the engine with itself. It stays on `KNOWN_UNWIRED` with the exact caller that
clears it (a 30 s sweep over the first live broker adapter, gate H-4).

Any mismatch from `crates/reconciliation` halts that venue's order flow and only that
venue's; resume is refused until reconciliation passes.

**Acceptance:** an injected mismatch halts one venue, leaves the others trading, and blocks
resume until clean.
**Traces:** COMP-007 §4; §8.3.

### S-7.7 Execution quality, behaviour, and automatic scale-down — L
`DONE` 2026-10-03 (was `PARTIAL`) — every behaviour input except one is now
measured from rows that belong to the strategy and no other
(`monitor::activity`): **signals per hour** from `strategy_decision_journal`
decisions that produced a gate verdict (reported only for a deployment running the
whole window — `None`, never `Some(0)`, otherwise), **holding period p50** and
**position count** from a replay of the strategy's own fills (`fills ⋈ orders` on
the idempotency key, `orders.strategy_id` = the deployment id) — not from
`positions`, which has no strategy column. Running deployments are judged from the
moment their row exists. Verified by
`measurement_reaches_the_gate::a_measured_fill_rate_below_its_floor_scales_the_deployment_to_a_quarter_in_the_gate`
(ten real orders, four filled → `fill_rate` 0.4 vs a declared floor of 0.8 →
three cadences of dwell → `scaled_down`/25 in `monitor.strategy_state` → the
watched process's `refresh` sets the gate's scale to 25; the event names metric,
value and limit), `monitor::activity::tests::*` (4) and the existing
`service::tests::a_sustained_fill_rate_breach_scales_to_a_quarter_and_the_event_names_the_numbers`.
**Slippage against the fill model is unobservable on paper by construction**
(ADR-0044/0048: paper fills *are* the model's output, so realised − predicted ≡ 0);
it stays `Unobserved` until a live venue fills — reporting the zero would be a
perfect score for a measurement of nothing.

Fill rate, realised slippage vs the fill model's prediction, latency p95, partial-fill share;
signals per hour, holding period p50, orders per minute, cancel ratio, position count.
Baselines from shadow/paper ± 3σ EWMA and the venue rate-limit ceiling. A breach **scales the
strategy to 25%** automatically, because scale-down is reversible and therefore does not need
a human in the moment.

**Acceptance:** a fill rate 3σ below the paper baseline scales to 25% automatically and the
event names the metric, the values and the limit.
**Traces:** COMP-007 §3, §4 P4; §8.4.

### S-7.8 The P&L family — L
`DONE` 2026-10-03 (was `PARTIAL`) — **the P&L family has an input.**
`monitor::daily` rebuilds each v3 deployment's daily return series from Postgres
alone: `strategy_deployment.starting_capital`, its own fills (price, sign,
commission), and the close of the last bar it *decided on* each UTC day
(`strategy_decision_journal.inputs.event.bar.close`) — not the journal's equity,
which for a hand-bound deployment is the shared account's. Complete days only; a
position nobody priced stops the series rather than inventing a mark. Migration
0165 adds `monitor.return_baseline` (versioned, append-only, never agent-declared);
`collect_cadence` measures Sharpe 30/60/90, drawdown, time under water and the
CUSUM low statistic against it with a fresh chart each cadence. Verified by
`measurement_reaches_the_gate::a_sustained_cusum_breach_on_measured_daily_returns_halts_the_deployment_at_the_gate`
(a real deployment, a real fill, eight journal days falling 2 %/day → `cusum_low`
past its −5 floor → three cadences → `halted`, a `strategy:` kill-switch row the
gate reads, the record naming `cusum_low`), `monitor::daily::tests::*` (4), and the
existing CUSUM/halt-policy/record-first tests. Rolling Sharpe vs the backtest and the
MC p95 drawdown are *measured* here and *judged* against limits declared with
methods `backtest_3sigma` and `mc_p95_drawdown` — the bound comes from the backtest
(`trade_reshuffle_bootstrap` in `crates/backtest/src/manager.rs`), as §3 says;
nothing here derives one.

Rolling 30/60/90-day Sharpe vs backtest, CUSUM on daily returns, drawdown against the
backtest's 95th-percentile Monte Carlo drawdown (from `trade_reshuffle_bootstrap`, S-3.12),
time under water. A CUSUM or MC-drawdown breach **halts the strategy** and applies its
`HaltPolicy` (`crates/domain/src/instrument.rs:198` — hold / flatten / reduce).

**Acceptance:** a CUSUM breach halts and applies the policy; the trip record exists **before**
the first refused order.
**Traces:** COMP-007 §3, §4; §8.5.

### S-7.9 Limits are declared before the drawdown — M
`DONE` 2026-10-03 (was `PARTIAL`) — **the gate is in the schema, so it has a
caller whatever promotes.** Migration 0165 puts a `BEFORE INSERT OR UPDATE` trigger
on `strategy_deployment` (the only deployment record, 0142):
a row whose `environment` is not `paper` is refused unless
`monitor.has_live_limits(deployment_id)` — current limits in all three
`REQUIRED_FOR_LIVE` families. Today 0142's CHECK already admits only `paper`; the
trigger is what still holds the day that CHECK is widened. Verified by
`measurement_reaches_the_gate::no_deployment_leaves_paper_without_limits_in_three_families`
(drops the CHECK inside a rolled-back transaction, then: live without limits
refused, live with the three families written) plus the existing
`limits::tests::*` and `a_strategy_with_no_declared_limits_cannot_be_promoted_to_live`.
Mid-breach changes are recorded with the breach (`limit_change`, unchanged).
ADR-S7-13 (new): enforce in the schema, not a route — rejected: a check in
`POST /api/deployments`, which is Set U's file and would not cover a second writer.

`monitor_limits(strategy_id, metric, version, lower, upper, method, declared_from,
declared_at, declared_by)`, versioned, where `declared_from` names the backtest or paper run
the baseline came from. Changing a limit **during** a breach takes the two-step pattern and
is logged with the breach it overrode. COMP-002's promotion gate is amended: a strategy with
no declared limits cannot be promoted to `live`.

**Files:** `migrations/0076_monitor_limits.sql`, `crates/api/src/routes/`, COMP-002 §3.
**Acceptance:** promotion to `live` without limits is refused; a mid-breach limit change is
recorded with the breach.
**Traces:** COMP-007 §3, §2 P2; §8.8.

### S-7.10 Events and notifications — M
`DONE` 2026-10-04 (track F-UI) — **value and limit are on the wire and on the page.** `routes/monitor.rs` now selects `observed_value::float8, limit_value::float8` in the stream and serves both as nullable numbers on the stream's `monitor_event` frame and `GET /api/monitor/events` (held by `both_event_surfaces_serve_observed_and_limit_values`). The desk reads them: `parseMonitorFrame` carries `observed_value` / `limit_value` (null when absent, never 0), `MonitorLive` shows **value** and **limit** columns ("not recorded" when null), and the announcement detail reads `0.42 against a limit of 0.3` (`lib/ops/__tests__/monitor.test.ts`, 2 new cases, 18 passing). Not exercised in a browser; NATS publication remains the declared deviation ADR-S7-11. The 2026-10-03 note below named this as the only open acceptance field. 2026-10-03 note: **the desk now subscribes, and one acceptance field was still not on the wire.** `MonitorNotifier` (`frontend/src/components/desk/MonitorNotifier.tsx`, mounted once in `AppLayout`, renders nothing) is the single subscriber to `GET /api/monitor/stream` (registered in `routes/mod.rs`): each frame goes into a shared react-query entry (`MONITOR_LIVE_KEY`), is announced on the live region and — only through `notifyDecision`'s four refusals — posted as a browser notification, and invalidates the monitor, kill-switch and inbox queries. A drill announces marked `DRILL` and **never** notifies; a blip is recorded and silent; a replayed `event_id` announces once; the stream's own state (`stale` with no age before it ever connected) is printed beside the table so a quiet table is never read as a quiet platform. The Monitor page (`/desk/monitor`, `Operations.tsx::Monitor`) gains `MonitorLive`: live events, `/api/monitor/feeds` (age and verdict; unwatched deployments named), `/venues`, `/coverage` ("measured, never judged"), `/defects` with a resolve that needs a sentence, `/drill-runs` with run-now. Every panel reads through `Read<T>`, so a failed route is "unknown, not empty". Evidence: `a-monitor-event-reaches-the-desk-within-one-frame.test.tsx` (8), `the-monitor-measurement-panels-say-what-they-do-not-know.test.tsx` (11), `lib/ops/__tests__/monitor.test.ts` (16). **Still missing, in `routes/monitor.rs` (not this track's file):** the acceptance says an event carries *metric, value, limit and scope*; `monitor.events` has `observed_value` and `limit_value` (0075) but neither the stream's `monitor_event` frame nor `GET /api/monitor/events` selects them, so the page shows metric and scope and cannot show value or limit. Adding the two columns to the stream's SELECT and frame JSON closes it (`parseMonitorFrame` already tolerates extra fields). NATS publication remains ADR-S7-11.

Earlier note (track MONITOR) — **the server half of the desk subscription is built;
the client half and the route registration are not.** Every state change still
writes exactly one `monitor.events` row before the change (unchanged). Migration
0165 adds a `pg_notify('monitor_events', event_id)` trigger, `monitor_events` joins
`NOTIFY_CHANNELS` (so the order gate's mirror also wakes on every event), and
`GET /api/monitor/stream` (`routes/monitor.rs::stream`) is a server-sent-events
channel: a `kill_switch` snapshot on connect, on every lease/switch notification and
every 15 s regardless; a `monitor_event` per row. **Missing:** the route line in
`routes/mod.rs` (requested from the orchestrator) and the desk's `EventSource`
subscription in `frontend/src/components/desk/KillSwitchBanner.tsx` (not this
track's territory) — until both land, the banner still polls. NATS publication
remains the declared deviation ADR-S7-11.

Every state change writes `monitor_events` and publishes on NATS; the desk subscribes
(S-6.21) and the notification rules (approval, question, run finished, halted, kill switch,
control-limit breach) consume it.

**Acceptance:** every state change in §4 emits exactly one event carrying metric, value, limit
and scope; the desk banner reflects a trip within one event.
**Traces:** COMP-007 §6; AGENT-012 §13.

### S-7.11 The agent's read-only view — S
`DONE` 2026-09-27 (was `BLOCKED:UPSTREAM`) — **S-3.18 landed, which is what this
entry said would unblock it, and both acceptance clauses are now asserted by one
test.**

`crates/mcp-server/src/tools/control_limits.rs` exists and composes four existing
monitor reads. Its acceptance here is *"the tool has no write path; the catalogue
carries no monitor-mutating tool at any risk class"* — and
`no_tool_anywhere_posts_to_the_monitor` covers both by scanning the **whole**
`src/tools/` tree for a mutating verb near an `/api/monitor` path, plus pinning
`ApiClient`'s verb set so a new verb fails the test rather than slipping past it.
Its first version keyed on the path and failed on its own prose; keying on the verb
is what lets a tool *say* "a human resets it with POST …" without tripping. Proved
load-bearing by adding a wrapped-line `api
.post("/api/monitor/kill-switch/global/
trip")` to another tool file and watching it fail by name.

The read side it uses already existed
(`GET /api/monitor/{kill-switch,events,strategies,precision}` and
`monitor::store::MonitorStore::load_limits`). What is already true and tested: no agent token can
reach any monitor mutation (`api::monitor_authority::*`), the catalogue carries no
monitor-mutating tool at any risk class, and `monitor.control` is on `NEVER_FOR_AGENTS` with a
database trigger refusing it (migration 0075).

`portfolio.check_control_limits` reads these tables and nothing else. The monitor is a
service the agent may **query** and never **skip** — there is no agent-callable halt, scale
or reset.

**Depends on:** S-3.18.
**Acceptance:** the tool has no write path; the catalogue carries no monitor-mutating tool at
any risk class.
**Traces:** COMP-007 §6; AGENT-006 §5.7.

### S-7.12 Dwell, hysteresis, and scoring the monitor itself — M
`DONE` — verified by `monitor::dwell::tests::{a_single_cadence_excursion_is_a_blip_and_nothing_else,
three_consecutive_cadences_act, a_reading_back_inside_restarts_the_count_from_zero,
a_synthetic_oscillating_series_never_flaps_the_book, recovery_needs_twice_the_dwell,
one_good_cadence_during_recovery_does_not_bank_progress_after_a_relapse}` and
`monitor::precision::tests::{a_family_below_half_is_flagged_for_rederivation,
an_unlabelled_family_is_unmeasured_rather_than_perfect_or_terrible,
the_report_covers_every_family_including_the_silent_ones}`. Precision is computed by
`MonitorStore::compute_precision` and served at `GET /api/monitor/precision`; the `flagged`
column is a CHECK over the number rather than a boolean the writer chooses (ADR-S7-09).

A 3σ band on a noisy intraday metric breaches by construction, and a monitor that cries wolf
trains the operator to click through it — the alarm-fatigue failure the HITL research warns
about, except here the thing being ignored is the last line before money.

A metric must sit outside its limit for `min_dwell_cadences` (default 3) **consecutive**
evaluations before the state changes; a single excursion is a `blip`. Recovery needs twice
that, so a metric oscillating on its boundary cannot flap the book. Exempt from dwell:
reconciliation mismatch, lease loss and hard data staleness — facts, not estimates. And the
monitor is **scored like everything else here**: every breach is later labelled, precision per
metric family is published monthly beside the strategy scorecards, and a family below 0.5 has
its limits re-derived rather than left to erode trust.

**Files:** `crates/monitor/`, `migrations/0077_monitor_precision.sql`.
**Acceptance:** a one-cadence excursion produces a `blip` and no state change; a synthetic
oscillating series does not flap; precision per family is computed and a sub-0.5 family is
flagged.
**Traces:** COMP-007 §4.1; §8.9, §8.10.

### S-7.13 Drills, and maintenance windows — M
`DONE` 2026-10-03 (was `PARTIAL`) — against its acceptance clauses: a drill
exercises the full path and refuses nothing real (`Rehearsal`, unchanged); a slow
drill fails the deploy (`a_slow_drill_fails_the_deploy`); a window suppresses a
platform trip and not a reconciliation mismatch (`window::tests::a_window_suppresses_nothing_else`,
`a_platform_fault_inside_a_window_is_recorded_and_halts_nothing`); an open window is
visible and attributed (`an_open_window_is_visible_attributed_and_bounded`). The
two gaps the last note named are closed: the **monthly schedule** runs from
`MonitorService::run` (`run_scheduled_drill`, held by `facts_are_wired`), and a
drill's `is_drill` event now reaches a push channel within one event
(`monitor_events` NOTIFY → `GET /api/monitor/stream`). Rendering it as a DRILL
banner is the desk's (see S-7.10). The CI job still runs on every push rather than
only on `crates/{monitor,risk,execution}` changes — deliberately wider. Set T T-2.4
adds the four fail-closed drills on a weekly schedule on top of this.

Nobody will trip a global halt on a live book to see whether it works, so without a rehearsal
the most important path here is the one path never exercised — first exercised on the day it
matters, by someone under pressure. **Drill mode** runs the whole trip path against a `shadow`
scope: the monitor evaluates, the state machine transitions, events fire, notifications send,
the desk banner appears marked DRILL, and the order gate is asked and answers, refusing
nothing real. It measures detection, propagation and notification latency. Drills run after
every deploy touching `crates/{monitor,risk,execution}` and monthly regardless, and **a drill
whose propagation exceeds one cadence fails the deploy** — that is the entire assertion of the
fail-closed design, and until a drill proves it, the design is a claim.

**Maintenance windows** exist because a rolling deploy looks exactly like a heartbeat loss,
and without them every deploy halts trading and the operator learns to pre-emptively reset the
switch. A bounded, auto-expiring window (default 15 min) suppresses **platform-health trips
only** — never reconciliation, venue errors, data staleness or any strategy breach, which are
facts about the market and the book. An open window is as visible in the desk chrome as a
trip, because a window nobody can see is a hole nobody can audit.

**Files:** `crates/monitor/`, `crates/api/src/routes/monitor.rs`,
`migrations/0078_drills_and_windows.sql`.
**Acceptance:** a drill exercises the full path and refuses nothing real; a slow drill fails
the deploy; a window suppresses a platform trip and **not** a reconciliation mismatch; an open
window is visible and attributed.
**Traces:** COMP-007 §4.2, §4.3.

---

## Exit criteria

- The monitor survives the API dying, and the platform stops trading when the monitor dies.
- A breach scales down automatically; a P&L breach halts; a reconciliation mismatch halts one
  venue.
- Reset is human, typed, reasoned and rate-limited; no agent token can reach any monitor POST.
- No strategy reaches `live` without declared limits.
- The kill-switch state is visible on every route in the desk within one event of a trip.
