# Phase 7 — The monitor

**Completion: 0% (0 / 13 tasks)**

**Requirement IDs:** MS-01…MS-14.
**Spec:** [COMP-007](../../../specs/COMP-007-live-monitor-and-kill-switch.md), extending
[COMP-002 §3](../../../specs/COMP-002-execution-and-risk-gate.md).
**Depends on:** Phase 0 only. Fully independent of the agent work — start it on day one.
**Blocks:** the monitor page (S-6.21) and the read-only `check_control_limits` tool (S-3.18).
**Migration range:** 0074–0078.
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

## Tasks

### S-7.1 `crates/monitor` — a separate process — M
`NOT STARTED`

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
`NOT STARTED`

The monitor renews a `monitor_lease` row every cadence. The runtime's order path checks it;
no valid lease → every new order is refused with `monitor_unavailable`. This is how "if the
monitor dies, the platform stops" is enforced without the monitor being in the process.

**Files:** `migrations/0074_monitor_lease.sql`, `crates/execution/`, `crates/risk/`.
**Acceptance:** killing the monitor causes order refusal within one cadence; killing the
**API** does the same from the runtime's side; restarting does not resume orders until the
lease is valid again.
**Traces:** COMP-007 §4 ("fail closed"); §8.1, §8.2.

### S-7.3 The scoped, persisted kill switch — M
`NOT STARTED`

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
`NOT STARTED`

Reset requires the human's typed confirmation of the scope name, a reason, and — for global —
that every venue reconciles clean first. One reset per five minutes per scope, so a flapping
fault cannot be reset into. The POST routes require a `web:full`-class scope that agent
service tokens do not carry.

**Files:** `crates/api/src/routes/monitor.rs`, `crates/api/src/auth/scopes.rs`.
**Acceptance:** a global reset without a typed scope and reason is refused; a second reset
within five minutes is refused; **no agent token can call any monitor POST**, asserted
adversarially with a minted service token.
**Traces:** COMP-007 §5, §6; §8.7, §8.6.

### S-7.5 Data and venue families — M
`NOT STARTED`

Last-bar age per instrument, gap count, impossible bars (reuse `probe_data_quality`);
connection state, error rate, rate-limit headroom. Staleness > 3× timeframe warns; > 10×
halts the strategy.

**Acceptance:** a stale feed halts the strategy that depends on it and only that one; the event
names the instrument and the age.
**Traces:** COMP-007 §3.

### S-7.6 Reconciliation halts the venue — M
`NOT STARTED`

Any mismatch from `crates/reconciliation` halts that venue's order flow and only that
venue's; resume is refused until reconciliation passes.

**Acceptance:** an injected mismatch halts one venue, leaves the others trading, and blocks
resume until clean.
**Traces:** COMP-007 §4; §8.3.

### S-7.7 Execution quality, behaviour, and automatic scale-down — L
`NOT STARTED`

Fill rate, realised slippage vs the fill model's prediction, latency p95, partial-fill share;
signals per hour, holding period p50, orders per minute, cancel ratio, position count.
Baselines from shadow/paper ± 3σ EWMA and the venue rate-limit ceiling. A breach **scales the
strategy to 25%** automatically, because scale-down is reversible and therefore does not need
a human in the moment.

**Acceptance:** a fill rate 3σ below the paper baseline scales to 25% automatically and the
event names the metric, the values and the limit.
**Traces:** COMP-007 §3, §4 P4; §8.4.

### S-7.8 The P&L family — L
`NOT STARTED`

Rolling 30/60/90-day Sharpe vs backtest, CUSUM on daily returns, drawdown against the
backtest's 95th-percentile Monte Carlo drawdown (from `trade_reshuffle_bootstrap`, S-3.12),
time under water. A CUSUM or MC-drawdown breach **halts the strategy** and applies its
`HaltPolicy` (`crates/domain/src/instrument.rs:198` — hold / flatten / reduce).

**Acceptance:** a CUSUM breach halts and applies the policy; the trip record exists **before**
the first refused order.
**Traces:** COMP-007 §3, §4; §8.5.

### S-7.9 Limits are declared before the drawdown — M
`NOT STARTED`

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
`NOT STARTED`

Every state change writes `monitor_events` and publishes on NATS; the desk subscribes
(S-6.21) and the notification rules (approval, question, run finished, halted, kill switch,
control-limit breach) consume it.

**Acceptance:** every state change in §4 emits exactly one event carrying metric, value, limit
and scope; the desk banner reflects a trip within one event.
**Traces:** COMP-007 §6; AGENT-012 §13.

### S-7.11 The agent's read-only view — S
`NOT STARTED`

`portfolio.check_control_limits` reads these tables and nothing else. The monitor is a
service the agent may **query** and never **skip** — there is no agent-callable halt, scale
or reset.

**Depends on:** S-3.18.
**Acceptance:** the tool has no write path; the catalogue carries no monitor-mutating tool at
any risk class.
**Traces:** COMP-007 §6; AGENT-006 §5.7.

### S-7.12 Dwell, hysteresis, and scoring the monitor itself — M
`NOT STARTED`

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
`NOT STARTED`

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
