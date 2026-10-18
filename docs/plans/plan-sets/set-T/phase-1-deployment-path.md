# Phase 1 — The deployment path

**Completion: 20% (1 / 5 tasks), plus 4 `PARTIAL` (T-1.1 armed-later switch and the decision UI; T-1.3 needs its React page; T-1.4 forward capture needs the T-0.4 deployment; T-1.5 server route blocked on a single-run spec and on Set U's v3 backtest path). 2026-10-04 (track F-DATA-T): the legacy sweep is scheduled, the arming SQL is written and the runbook exists; arming itself is `BLOCKED:HUMAN`.**

**Requirement IDs:** FE-DEP-01…FE-DEP-04, FE-REC-05.
**Spec:** [AGENT-013 §4, §7](../../../specs/AGENT-013-modes-and-research-lifecycle.md) (rings,
`Proposal`); ADR-P2-14 (`paper_v1`); migration 0054 (approval envelope); OQ-08 option A.
**Depends on:** Phase 0.
**Blocks:** Phase 2.
**Migration range:** 0107–0108.

Phase 0 proves the measurement works on one strategy someone deployed by hand. This phase makes
deployment a **path** — one door, a human at it, an envelope recorded — so that when the agent
starts producing candidates there is somewhere for them to go that is neither "nowhere" nor "a
side door somebody added under time pressure".

---

### T-1.1 `Proposal` as the only door into ring 2 — M
`PARTIAL` 2026-10-04 (track F-DATA-T) — **the route mounts and the route hook exist (`POST /api/deployments` takes `proposal_id`; `/api/deployment-proposals`, `.../decision`, `/api/deployment-overview`, `/api/deployments/{id}/revoke`, `/api/deployment-stop-all`, `/api/deployment-ceilings` are in `routes/mod.rs`). What keeps it PARTIAL: arming is irreversible and the owner's decision (`BLOCKED:HUMAN`, exact SQL in `docs/operations/forward-clock-runbook.md`, "Arming"), and the frontend decision UI is not built (frontend, outside this track's territory: needs `frontend/src/pages/DeploymentsPage.tsx` over those routes and a route line in `App.tsx`).** The one thing that would make arming unsafe, a deployments UI that does not send `proposal_id`, is why the owner arms it and a track does not.

Earlier: `PARTIAL` 2026-10-03 — **the door is built and enforced in the schema; it is not armed, and the route does not yet ask for a proposal.** Migration 0180: `deployment_proposal` (all sixteen verdicts, thresholds, capital, period; `CHECK verdict_revision_hash = revision_hash`), `deployment_decision` (human-only `decider_kind`, envelope with who/when/evidence hash/limits), `deployment_authorisation` (composite FK to an *approved* decision, one use), and `strategy_deployment_requires_authority`, a BEFORE INSERT trigger checking revision, owner, capital and instrument against the proposal; selection members inherit their parent's authority. Code: `ledger::deployment_path`, `api::deployment_gate` (+ `_routes`). Tests (`crates/api/tests/deployment_path.rs`, real PG, 15 pass): `a_deployment_exists_only_with_a_decided_proposal_behind_it`, `no_scope_an_agent_can_hold_decides_or_authorises_a_deployment` (per scope), `the_envelope_records_who_when_and_on_what_evidence`, `a_stale_verdict_is_refused`, mutation `without_the_trigger_the_door_is_open`. **Missing:** (1) the trigger is dormant until `deployment_gate_switch` is armed (ADR-T1-01), because arming before the route sends `proposal_id` would break every `POST /api/deployments`; (2) the deployments route hook and route mounts (orchestrator, see report); (3) the frontend decision UI. ADR-T1-01: one-way arming switch (rejected: always-on trigger, which breaks the live route and Set U tests the moment the migration applies; rejected: a config flag, which can be turned off). ADR-T1-02: own `deployment_proposal` tables rather than a new `ProposalKind` (rejected: extending `ledger::proposal`, outside this track's files, and its `deploy_paper` kind arms an automation, not a strategy).

A deployment proposal carries: the strategy revision, the gate profile it passed, the full
verdict (all sixteen gates with their evidence), the declared divergence thresholds for Phase
0's detector, the paper capital, and the review period. It is created by a research run
(ring 0) and **decided by a human**; the decision writes an approval envelope (migration 0054's
shape) and the deployment row.

No tool deploys. The agent's most capable action here is producing a proposal good enough that
a human says yes.

**Files:** `crates/domain/src/proposal.rs`, `crates/api/src/routes/proposals.rs`,
migration 0107 (`deployment`, `deployment_decision`).
**Acceptance:** a deployment exists only with a decided proposal behind it, enforced by a FK
and a test per entry point; no agent token can create a deployment, proven per scope; the
envelope records who decided, when, and on what evidence; a proposal whose verdict is stale
relative to the strategy revision is refused.
**Traces:** AGENT-013 §4, §7; D-T2; ADR-0025.

### T-1.2 `paper_v1` authorises paper, and cannot authorise capital — M
`DONE` 2026-10-03 — `ledger::deployment_path::VenueClass` has one variant; `PaperAuthority::from_pass` (gates 1..=15, the paper_v1 shape) has no conversion to capital, and `capital::authorise` still refuses `paper_v1` (`a_paper_v1_pass_cannot_be_converted_into_capital`, `the_only_venue_class_is_paper`); schema: `CHECK (venue_class = 'paper')` on proposal and authorisation, and the ramp trigger refuses a `paper_v1` step_up (`paper_v1_cannot_authorise_capital_in_the_schema`). `strict_v1` thresholds untouched. Note: implemented in new files rather than `gates/profile.rs` (outside territory); the profile's `authorises_capital` column (0050/0053) is the existing half.

The profile exists (ADR-P2-14). This task makes the authorisation boundary structural: a
`paper_v1` pass produces a deployment whose `venue_class` is paper and whose type cannot be
converted to a capital-authorising one. `strict_v1` is untouched.

The two profiles are already non-comparable by mechanism (AT-29); this extends that to what
each may *cause*.

**Files:** `crates/backtest/src/gates/profile.rs`, `crates/domain/src/deployment.rs`,
migration 0108.
**Acceptance:** a `paper_v1` pass cannot produce a capital deployment — the type does not
permit it and a test asserts the conversion does not compile or is refused at the DB CHECK;
`strict_v1` thresholds are unchanged, asserted by the existing immutability test.
**Traces:** ADR-P2-14; D-T5; SPEC §12.3 C-5.

### T-1.3 Deployment is visible, and revocable in one action — M
`PARTIAL` 2026-10-04 (track DESK) — **the page is built; it is not reachable until `App.tsx` mounts it.** `frontend/src/pages/DeploymentsPage.tsx` over `api/deployments.ts` (`/api/deployment-overview`, `/api/deployments/{id}/revoke`, `/api/deployment-stop-all`, `/api/deployment-proposals`, `…/{id}/decision`, `/api/deployment-ceilings`, and `POST /api/deployments`): one row per deployment with its gate verdict (and which gates did not pass), the `legacy_ungated` marker where no proposal stands behind it, ramp size, review-period countdown (an ended period says "review due", never "in 0 days") and divergence state (no forward recorder reads "not measured", never "no halts"); **Stop…** posts that deployment's revoke route; **Stop everything…** is dead until a reason is typed, says it is the kill switch's action, and posts `{reason}` to stop-all; a pending proposal is decided through the decision route, and **an approved, undeployed proposal is deployed with its `proposal_id` in the body** (the strategy, instrument and capital sent are the proposal's own, so the trigger's check cannot disagree with the request); the ceilings table says an undeclared ceiling refuses every deployment. Reads go through `ReadGate`: a list that did not load is "unknown, not empty". Meridian primitives only; no Tailwind alias. Test `frontend/src/pages/__tests__/a-deployment-is-visible-and-stops-in-one-action.test.tsx` (9). **Missing:** the route and nav entry (shared files, below) — until `App.tsx` carries `/deployments` the page is built and unreached; the server's `unmeasured` flag is typed as optional because this tree's `overview` does not send it yet; not driven in a browser. Arming the gate was not touched. Earlier note: `PARTIAL` 2026-10-04 — **backend built and mounted; the page is not.** The routes are registered (see T-1.1) and `overview` now also reports whether a deployment is **unmeasured** (no forward recorder), which the ramp pass flags hourly (ADR-T3-04). The React page is outside this track's territory (`frontend/src/pages/DeploymentsPage.tsx` over `GET /api/deployment-overview`, `POST /api/deployments/{id}/revoke`, `POST /api/deployment-stop-all`; no deprecated Tailwind alias).

Earlier: `PARTIAL` 2026-10-03 — **backend built, page not.** `deployment_gate::overview` (every deployment with profile, verdict, gates passed, `legacy_ungated`, ramp size and period countdown, divergence-halt count and realised-vs-expected series where a forward record exists), `revoke_one` (registry withdrawn, order gate refuses its strategy scope at once, record closed) and `revoke_all` (the global kill-switch trip through the monitor's own store, then every deployment withdrawn). Test `stop_all_is_the_kill_switch_action_and_stopping_one_halts_its_scope` proves both converge on `MonitorStore::trip` + `KillSwitch::trip_scope` and that the gate refuses within the call. **Missing:** the React page (frontend is not this track) and the route mounts. ADR-T1-03: revoke trips a strategy scope (rejected: only deregistering, because the runner then stops at its next tick, not now).

A deployments surface listing every running strategy with its profile, its gate verdict, its
divergence state, its realised-vs-expected series, and its review-period countdown. One action
stops one deployment; one action stops all of them, and that action is the same one COMP-007's
kill switch trips.

Built on the Meridian primitives only, like the desk (S-6.1).

**Files:** `crates/api/src/routes/deployments.rs`, `frontend/src/pages/`.
**Acceptance:** every deployment is listed with its verdict and divergence state; stopping one
halts its submission within one cadence; the stop-all action and the kill switch converge on the
same mechanism, proven by a test; no deprecated Tailwind alias appears in the new page.
**Traces:** COMP-007; AGENT-012 §2.

### T-1.4 `legacy_ungated` markers on everything already running — M
`PARTIAL` 2026-10-04 — **the sweep is scheduled.** `apps/platform/src/health_jobs.rs::run_once` now calls `legacy_marker_pass`, which calls `api::legacy_marker::mark_unmarked` every hour (the test `the_fleet_and_drill_passes_run_from_the_loop_that_runs` fails if the call is removed). Remaining: the UI shows the marker only once the T-1.3 page exists, and forward capture waits on the T-0.4 deployment (`BLOCKED:HUMAN`, runbook).

Earlier: `PARTIAL` 2026-10-03 — migration 0180 backfills a `deployment_legacy_marker` for every running deployment, `legacy_marker::mark_unmarked` sweeps stragglers, the marker is refused on an authorised deployment, shown in `overview`, and no evaluator role holds a grant on it (`everything_running_carries_the_legacy_marker_and_no_gate_can_read_it`, `no_gate_or_evaluator_source_names_the_legacy_marker`; the pre-existing display flag `ComparisonFlags.legacy_ungated` is the one allowed mention). **Missing:** the sweep is not yet scheduled (hook for `health_jobs::run_once`), the UI shows nothing until the page exists, and forward capture waits on T-0.4 (a recorder per deployment; `BLOCKED:HUMAN`).

ADR-P2-27: nobody is grandfathered, nothing is pulled by the gate. Every currently-running
strategy gets a `legacy_ungated` marker on its lineage — **a fact shown beside its numbers,
never a permission and never an input to any threshold** — and its forward record from today is
captured as Gate 15 evidence.

This is the cheapest gate evidence the platform will ever get, and it is the reason "pull
everything immediately" was the wrong answer.

**Files:** `crates/ledger` lineage marker, migration 0108, backfill of existing rows.
**Acceptance:** every running strategy carries the marker; the marker appears beside its numbers
in the UI; a test asserts the marker is not readable by any threshold or gate computation; its
forward observations are being captured by Phase 0's pipeline.
**Traces:** ADR-P2-27; D-T3.

### T-1.5 Retire `POST /api/backtests` — M
`PARTIAL` 2026-10-04 (track T-P) — **still `PARTIAL`; the product decision that was blocking the design is made here, and what remains is Set U's.** **ADR-T1-06:** *no exploratory unlogged run exists.* INV-16 says every run is logged, and ADR-T1-05's open question (2) asked whether an unlogged quick run is to survive the retirement of `legacy_unlogged`. It does not: a quick backtest is a logged single-run trial on the Experiment path, never a run that registers nothing. Rejected: keeping an "exploratory" exemption (it is the same exemption OQ-08 option A exists to remove, and `legacy_unlogged` could then never be armed shut). Consequence for the missing list below: item (2) is closed by this row, leaving (1) a single-run spec on the suite (`initial_balance`, `quote_currency`, optional backfill, a run-level `BacktestResult` GET), (3) Set U moving its v3 path off `POST /api/backtests` (`crates/api/src/routes/backtests.rs` is reached by `crates/api/tests/v3_backtest_over_http.rs`; both are Set U's), and (4) deleting the route, asserting 404 and arming `legacy_unlogged_retired` (`BLOCKED:HUMAN`, irreversible, runbook SQL). Nothing in this track's territory can do (1) without (3), because a single-run spec that Set U's tile does not call would be a second unreached path.
`PARTIAL` 2026-10-04 (track F-DATA-T) — **the frontend move was investigated against the real code and cannot be done honestly yet; the server route stays.** `GET /api/backtests` lists `state.backtest` (`BacktestManager`); the Experiment routes use `state.suite` (`SuiteManager`) and nothing in `suite.rs` registers with the manager, so a tile created through an Experiment + Study would never appear in the list, nor get stop/rerun/delete. The Experiment path also cannot carry the tile's inputs or return its result: `CreateExperimentSpec` has no `initial_balance`, `quote_currency` or collect flag (the starting balance is a constant in `sim_executor.rs`); `delta_practical` is required with no honest tile value and a disjoint holdout window is mandatory; every Study burns a trial in the ledger; `StudyView` returns a scalar `Distribution` for one metric, not the `BacktestResult` the tile renders; `run_study` blocks and has no progress or cancel. And `POST /api/backtests` is no longer only the legacy tile: Set U's U-3.4 test (`crates/api/tests/v3_backtest_over_http.rs`) posts to it and the v3 branch runs on the one engine.
**ADR-T1-05:** make no frontend change and do not delete the route. Rejected: calling `experimentsApi.create` with an invented `delta_practical` and an auto-derived holdout, then `runStudy` with a one-element grid, and showing the median as the tile (fabricated pre-registrations, burned trials, a lossy result in place of the tile). Rejected: moving only the create call (the new tile is invisible to the list). Rejected: deleting the route now (breaks Set U's done task).
**Missing, in order:** (1) a single-run spec on the suite (`initial_balance`, `quote_currency`, optional backfill, a run-level `BacktestResult` GET), or a product decision that "quick backtest" becomes "create Experiment, then run Study"; (2) a decision on whether an exploratory unlogged single run exists at all (INV-16 and the retirement of `legacy_unlogged` re-open OQ-08); (3) Set U moves its v3 path off `POST /api/backtests`; (4) then delete the route, assert 404, and arm `legacy_unlogged_retired` (`BLOCKED:HUMAN`: irreversible; SQL in the runbook, with the warning not to arm before the route is gone).

Earlier: `PARTIAL` 2026-10-03 — **only the database rule is built; the route and UI are untouched, deliberately.** Migration 0180's `trg_trial_refuses_new_legacy` refuses any new `mlops.trial` under `legacy_unlogged` once `deployment_gate_switch` holds `legacy_unlogged_retired` (existing rows untouched; `once_armed_no_new_trial_registers_as_legacy_unlogged_and_history_stays`). The route still serves the Back Testing tile (`BacktestManager::create` registers under the legacy marker), so arming now would break the UI. **Missing, in order:** move the frontend tile to the Experiment path (frontend, outside this track), delete `routes/backtests.rs::create_backtest` and `BacktestManager`'s legacy entry, assert 404, then arm the switch. ADR-T1-04: ship the rule as an armed-later switch (rejected: always-on, which breaks the UI).

OQ-08 option A, whose recommendation was "B now, A when the frontend is next touched". This is
the frontend being touched.

In one PR and in this order: move the Back Testing UI onto the Experiment path, then delete the
route, `BacktestManager::create`'s legacy entry and the `legacy_unlogged` dispatch context it
needed. The `legacy_unlogged` *marker* stays for historical rows — history is not rewritten —
but nothing new can be created under it.

**Files:** `crates/api/src/routes/backtests.rs` (deleted), `crates/backtest/src/manager.rs`,
`frontend/src/pages/` Back Testing tile.
**Acceptance:** the route returns 404 and a test asserts it; the UI creates Experiments and its
"quick backtest" interaction still works end to end; no new row can be written with
`policy_id='legacy_unlogged'`, enforced by a DB rule, while existing rows are untouched; INV-16
is enforced with **no remaining exemption** except AT-64's external MCP door.
**Traces:** OQ-08; INV-16; ROADMAP §10 (retirement checkpoints).

---

## Phase exit

- One door into ring 2, with a human at it and an envelope behind it.
- `paper_v1` structurally cannot authorise capital.
- Everything running is marked, visible, and contributing forward evidence.
- The last human-facing unlogged dispatch path is gone.
