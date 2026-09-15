# Phase 2 — The paper fleet

**Completion: 0% (0 / 4 tasks)**

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
`NOT STARTED`

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
