# Phase 2 — The paper fleet

**Completion: 13% (0 / 4 tasks), plus 1 `PARTIAL`.**

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
`NOT STARTED`

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
`PARTIAL` 2026-09-27 (was `NOT STARTED`) — `crates/reconciliation/src/fleet.rs`
(13 tests). The detector half is built; the dashboard and the automatic filing are
not.

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
`NOT STARTED`

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
`NOT STARTED`

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
