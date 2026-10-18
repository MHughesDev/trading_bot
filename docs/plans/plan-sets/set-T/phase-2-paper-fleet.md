# Phase 2 — The paper fleet

**Completion: 75% (3 / 4 tasks: T-2.1, T-2.2, T-2.4), plus 1 `PARTIAL` (T-2.3: the monitor is built and drilled; one process has to be started, see the runbook). T-2.1 closed 2026-10-04 by track T-P (it read 2 / 4 before).**

> **2026-09-27.** T-2.2's detector is built (`fleet.rs`): a systematic simulator
> error is one common-mode finding rather than k strategy failures, and the module
> refuses to mint the reproduction that would back a `platform_defect` because a
> forward divergence cannot be replayed. The dashboard and the filing remain, and
> nothing here can be exercised against real data until H-1.

**Requirement IDs:** FE-DEP-05, FE-REC-06, FE-MON-01, FE-MON-02.
**Spec:** [COMP-007 §8](../../../specs/COMP-007-live-monitor-and-kill-switch.md); SPEC §12.3;
Set S Phase 7 (which builds the monitor this phase operates).
**Depends on:** Phase 1; **Set S Phase 7** (the monitor must exist to be drilled).
**Blocks:** Phase 3.
**Migration range:** 0109–0110.

One deployment proves the pipe. A fleet is what produces Gate 15 evidence at a rate that makes
DONE-3 reachable, and it is also the first time anything on this platform runs unattended
against a broker — which is when the monitor stops being a feature and becomes the thing
standing between a bug and an account.

---

### T-2.1 Many deployments, bounded — M
`DONE` 2026-10-04 (track T-P) — **closed against its acceptance clauses; the one thing the earlier notes held it `PARTIAL` for was not a clause.** Clause 1 (refused, naming the binding ceiling): `deploying_past_a_ceiling_is_refused_and_names_which_one_bound` (PG). Clause 2 (aggregate across instances of a revision, not per instance): ledger `fleet_ceiling::tests` via `capacity::Exposure`, and the same PG test counting running plus approved-unspent deployments. Clause 3 (configuration with no silent permit-everything default): `FleetCeilings` has no `Default`, an undeclared ceiling refuses (`an_undeclared_ceiling_is_a_refusal_not_unlimited`), and the three seeded rows are declared with their ADR (`the_seeded_ceilings_are_declared_with_their_adr`). The ceilings page is the T-1.3 surface (built by track DESK) and is not a T-2.1 clause. **ADR-T2-05:** no per-order aggregate check is added to `crates/risk`. Rejected: an order-time aggregate in `RiskGate::check`. The acceptance names decision time ("refuses the deployment at proposal-decision time"); every deployment is paper with a capital fixed at approval, so the ceiling bounds what can be deployed and an order-time copy would be a second place the same number is enforced (two chokepoints drift; SC-2 keeps one). If capital ever stops being fixed at approval, this reopens with the capital gate (`docs/governance/capital-gate.md`).

Earlier, `PARTIAL` 2026-10-04 (track F-DATA-T) — **the ceilings are now declared, by an ADR.** Migration 0190 seeds `max_concurrent = 10`, `max_aggregate_capital = 100000` and `max_instrument_exposure = 25000` with `declared_by = adr:ADR-T2-04` (append-only; raising one is a new row via `PUT /api/deployment-ceilings`). **ADR-T2-04:** they bound **paper** deployments only (`venue_class` is a CHECK of one value), so a wrong number costs a refused approval and never money; ten is enough running strategies for Gate 15 evidence to accrue at a useful rate and few enough that one person can read every one on the deployments page; 100k is ten deployments at the 10k paper start the first deployments use; 25k per instrument is a quarter of the aggregate so one instrument cannot be the whole fleet. Rejected: leaving them undeclared (every approval refused until someone chose, for a limit that cannot lose money), and generous or unbounded numbers (the silent default 0182 exists to avoid). `ledger::fleet_ceiling` still has no default and an undeclared ceiling still refuses (ledger `an_undeclared_ceiling_is_a_refusal_not_unlimited`; the PG test now truncates the seed to prove it, `deploying_past_a_ceiling_is_refused_and_names_which_one_bound`), and `the_seeded_ceilings_are_declared_with_their_adr` pins the three rows. Still no per-order aggregate check in `crates/risk` (the decision-time check is the stated acceptance); T-2.1 is `PARTIAL` only for that and for the T-1.3 page that shows the ceilings.

Earlier: `PARTIAL` 2026-10-03 — `ledger::fleet_ceiling` (`FleetCeilings` has no default; undeclared refuses; concurrency, aggregate capital, per-instrument exposure aggregated across instances of a revision via `capacity::Exposure`), migration 0182 (`deployment_ceiling`, no seed rows), and `deployment_gate::decide_proposal` checks them at decision time counting running plus approved-unspent deployments and naming the binding ceiling (`deploying_past_a_ceiling_is_refused_and_names_which_one_bound`; ledger `fleet_ceiling::tests` 4). **Missing:** a person must declare the three ceilings before any approval succeeds; no per-order aggregate check was added to `crates/risk` (the decision-time check is the stated acceptance).

Concurrent paper deployments with a declared ceiling: maximum concurrent strategies, maximum
aggregate paper capital, maximum per-instrument exposure aggregated across every instance of a
revision (`capacity::Exposure`, which already exists). Exceeding a ceiling refuses the
deployment at proposal-decision time, with the binding ceiling named.

**Files:** `crates/domain/src/deployment.rs`, `crates/risk` aggregate checks, migration 0109.
**Acceptance:** deploying past a ceiling is refused and names which one bound; aggregate exposure
is computed across instances of the same revision, not per instance; the ceilings are
configuration with no defaults that silently permit everything.
**Traces:** SPEC capacity; COMP-002; ADR-P6 (`capacity::Exposure`).

### T-2.2 Fleet-level reconciliation and a divergence dashboard — M
`DONE` 2026-10-03 (track UI2) — **the dashboard exists.** The fleet panel of `frontend/src/pages/ReconciliationPage.tsx` renders `GET /api/reconciliation/fleet` (registered): the verdict (`common_mode` as the red one with the server's own observed sentence, `idiosyncratic`, `no_divergence`, and `not_computable` rendered as **Not computable**, never as "no divergence"), the declared policy (5 bps per bar tolerance, quorum 3, 30 bars), the aggregate, and per deployment the bars (backfilled rows excluded and counted), mean and cumulative divergence in bps, and slippage excess (`null` is "no traded window", not 0). A filed defect id is shown as an observation awaiting a human replay; the open-defects list with its human-only resolve is on the Monitor page (`/api/monitor/defects`). Evidence: the fleet case of `the-halt-is-visible-and-clears-only-on-purpose.test.tsx` and the fleet cases of `the-pages-state-what-they-do-not-know.test.ts`. It reads zero deployments until T-0.4 deploys a recorder, and the page says that is "not computable".

Earlier note (track MONITOR) — **the dashboard's data and the automatic filing are built
and run; the React page is not.** `reconciliation::fleet_job` reads a tenant's
`mlops.forward_observation` over a declared 30-day window (tenant-scoped, RLS),
judges it with `FleetReport::assess` under a declared policy (ADR-T2-02: tolerance
5 bps/bar, quorum 3, 30 observed bars — asserted by a test; rejected: each
deployment's own threshold, because *k* deployments crossing *k* different lines is
not one fact), and on a common-mode verdict **files a `monitor.platform_defect`**
(migration 0165; one open defect per tenant and direction) — run hourly, unattended,
from `apps/platform/src/health_jobs.rs::fleet_pass`. The finding is filed as an
observation awaiting a human replay, not minted into a `ledger` `EvaluatorDefect`
(that would need a `Reproduction` nobody can run for forward trading). The
dashboard's payload — per deployment, aggregate and verdict — is
`GET /api/reconciliation/fleet` (`fleet_job::report_json`; reading files nothing),
and open defects are `GET /api/monitor/defects` with a human-only resolve. Verified
by `reconciliation::fleet_job::tests::*` (4, incl.
`a_planted_systematic_cost_error_files_one_defect_naming_every_deployment` and the
disagreeing-directions contrast), the existing 13 `fleet` tests, and
`monitor` `a_defect_is_filed_once_per_condition_and_resolved_only_by_a_person`
(as `app_role`). **Missing:** the React dashboard (`frontend`, not this track) and
the route lines in `routes/mod.rs` (requested). And it reads zero rows until T-0.4
deploys a recorder.

**The acceptance clause is closed:** a planted systematic cost error in the
simulator is detected as **one common-mode finding, not k separate strategy
failures** (`a_planted_systematic_cost_error_is_one_common_mode_finding_not_five_
strategy_failures`), with both contrast cases asserted — misses in disagreeing
directions are not a finding against the simulator, and two agreeing misses in a
fleet of five are not either. `FleetReport::assess` produces the per-deployment and
aggregate realised-versus-expected numbers the dashboard's first clause needs.

**It refuses to mint the `Reproduction` itself, and that is the design.** A forward
divergence is an observation of live trading, which is the one thing that cannot be
replayed — so minting a reproduction from generated steps would put this platform's
most expensive claim, an `EvaluatorDefect` that outranks a promotion, behind a
replay nobody ran. The finding supplies the wording and the steps
(`expected_statement`, `observed_statement`, `replay_steps`); whoever actually
re-ran the backtests supplies the artifact and the seed.
`nothing_here_mints_a_reproduction_for_a_replay_nobody_ran` asserts this statically
over the module's own source.

Three further refusals rather than reassuring answers: `FleetInconclusive::
FleetTooSmall` when breaches exist below quorum (calling that "idiosyncratic" is the
comfortable reading of evidence that does not reach it), `NothingMeasurable`, and
`slippage_excess_bps: None` for an untraded window. `FleetPolicy` has no `Default`:
a quorum of 1, and a tolerance of 0 or infinity, are refused at construction.

**What keeps it `PARTIAL`:** the React dashboard (`frontend`), and the automatic
`platform_defect` filing (`crates/api`/`crates/harness`) — the finding is *shaped*
for it but nothing files it. One note for whoever writes the route:
`crates/reconciliation` has no `serde` dependency, so `FleetReport` is not
`Serialize`. Either add it there, or map to a DTO in the API.

**And it cannot be exercised against real data yet.** Every input is a realised
series, which needs deployed strategies — gate H-1. The tests drive planted
fixtures, which is the honest limit of what can be verified before then.

Phase 0's per-strategy detector, aggregated: which deployments are diverging, by how much, and
whether the divergence is idiosyncratic or **common across the fleet**. Common divergence is the
interesting signal — it means the simulator is wrong in a systematic way, which is worth more
than any single strategy's result.

A common-mode divergence files a `platform_defect`, which outranks a promotion in the type and
in the database.

**Files:** `crates/reconciliation/src/fleet.rs`, `frontend/src/pages/`, migration 0110.
**Acceptance:** the dashboard shows realised-vs-expected per deployment and in aggregate;
divergence common to ≥ k deployments files a `platform_defect` automatically; a planted
systematic cost error in the simulator is detected as common-mode and not as k separate strategy
failures.
**Traces:** SPEC §12.3; ADR-P6-10 (`report_evaluator_defect` outranks promotion); HANDOFF §5a.

### T-2.3 The monitor runs in production — M
`PARTIAL` 2026-10-04 (track T-P) — **the operations half is now written; what is left is one process running.** `docs/operations/forward-clock-runbook.md` has a "Starting the monitor" section (compose `docker compose up -d tbot-monitor`, or on the host `cargo run -p monitor --bin tbot-monitor -j 2` with the platform's `DATABASE_URL` and `MONITOR_CADENCE_MS`; how to confirm the lease through `GET /api/monitor/coverage`; how to run `monitor_lost` by hand). `tbot-monitor` is a real binary (`crates/monitor/Cargo.toml`) and a compose service that is deliberately not behind a profile. Every acceptance clause has its test (listed below); the one thing no test can do is be a process on this machine, so the task is `PARTIAL` only for that and not for any code.

`PARTIAL` 2026-10-03 (was `NOT STARTED`) — **everything the code can do is done;
"running here, now" is an operator starting a container.** Against the acceptance:
*runs as an independent process* — the `tbot-monitor` binary and compose service
exist and are asserted independent (`monitor::independence::*`); *killing it stops
order flow within one cadence* — `killing_the_monitor_stops_order_flow_within_one_cadence`,
and now drilled weekly against the **real** lease row (T-2.4 `monitor_lost`);
*limits derive from live deployment rows* — `collect_cadence` now enumerates
running `strategy_deployment` rows every cadence (judged from the moment the row
exists, keyed on the deployment id its orders carry), loads `monitor.limits` and
`monitor.return_baseline` per cadence, and `GET /api/monitor/coverage` lists, per
running deployment, which families are declared, whether it could leave paper, and
whether its feed is gauged — so a deployment the monitor measures but cannot judge
is visible, not implied; *no agent token can call a monitor POST, per scope* —
`api::monitor_authority::*`, and every new POST here goes through `require_control`.
**Remaining (human / ops):** start `tbot-monitor` beside the platform — 2026-09-21
found no monitor container running, and with no lease the gate refuses every order;
the weekly `service_restored` drill files a defect until it is. The operations
runbook (`docs/operations/`) is outside this track's territory.

COMP-007's monitor, built in Set S Phase 7, promoted to running beside the fleet: a separate
process, input-first limits, a fail-closed lease. Set S proves it works; this task proves it
**is working, here, now**, with its lease renewing and its limits loaded from the deployments
Phase 1 created.

**Files:** deployment configuration, `crates/api` lease wiring, `docs/operations/`.
**Acceptance:** the monitor runs as an independent process; killing it stops order flow within
one cadence; its limits derive from live deployment rows rather than a static config; no agent
token can call a monitor POST, proven per scope.
**Traces:** COMP-007 §8; D-S6, D-S10.

### T-2.4 Drills — M
`DONE` 2026-10-03 (was `NOT STARTED`) — `reconciliation::drills::run_all` runs the
four drills and records each in `monitor.drill_run` (0165) with its outcome, timing
and measurements: **`monitor_lost`** (the real lease row's expiry replayed against a
scratch gate: refused after expiry, and closing within one cadence of the last
renewal), **`divergence_planted`** (the real `DivergenceMonitor` into a scratch gate:
that strategy refused before its next order, another admitted), **`kill_switch_tripped`**
(a `drill:` scope tripped through the real `monitor.kill_switch` path via
`Rehearsal`, propagation within a cadence; a global trip refuses every running
deployment; an `agent:` reset is refused) and **`service_restored`** (a valid lease
and no drill residue afterwards). **Unattended, on schedule:** weekly, from
`apps/platform/src/health_jobs.rs::drill_pass` (checked hourly by `suite_is_due`);
on demand at `POST /api/monitor/drill-runs` (human-only). **A failed drill files a
`monitor.platform_defect`** before its run row, the schema refuses a failed run
that names none, and the failure is logged at `error` — the alert channel that
exists. **Cannot run against live capital by construction:** `run_all` refuses if
any running deployment is not paper, and `monitor.drill_run.venue_class` admits only
`'paper'`. Verified by `reconciliation::drills::tests::*` (7, incl.
`the_venue_class_is_asserted_in_code_and_in_the_schema`), `monitor`
`a_failed_drill_run_must_name_its_defect_and_only_paper_can_be_drilled` (as
`app_role`), and `health_jobs::feed_tests::the_fleet_and_drill_passes_run_from_the_loop_that_runs`.
ADR-T2-03 (new): the drills live in `crates/reconciliation`, not
`crates/observability/src/drills.rs` as planned — the divergence drill needs the
reconciliation detector and the kill-switch drill the monitor store, and
`reconciliation` already depends on both; rejected: a new dependency edge from
observability onto both. Results go to `monitor.drill_run`, the operations log the
desk can read, rather than to `docs/operations/drills.md`.

Fail-closed is a claim until it is drilled. A scheduled exercise that, on a schedule and on
demand: kills the monitor and asserts order flow stops within one cadence; plants a divergence
and asserts the halt; trips the kill switch and asserts every deployment stops and no agent
token can reset it; and restores service, recording how long each took.

Results are written to an operations log, and a failed drill is a `platform_defect`, not a
retry.

**Files:** `crates/observability/src/drills.rs`, `docs/operations/drills.md`.
**Acceptance:** each of the four drills runs unattended on schedule and records its outcome and
timing; a drill that fails files a defect and alerts; the drill cannot be run against live
capital by construction (there is none, and a test asserts the venue class).
**Traces:** COMP-007 §8; Set S round-3 review ("kill-switch drills, without which fail-closed is
a claim").

---

## Phase exit

- Multiple strategies run on paper inside declared ceilings.
- Systematic simulator error is visible as common-mode divergence and files a defect.
- The monitor is running, and the drills prove it, on a schedule, unattended.
