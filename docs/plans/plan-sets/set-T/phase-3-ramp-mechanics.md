# Phase 3 — Ramp mechanics

**Completion: 0% (0 / 4 tasks)**

**Requirement IDs:** FE-RAMP-01…FE-RAMP-04, FE-G16.
**Spec:** ML platform SPEC §12.3 Gate 16 (capital ramp 10 / 25 / 50 / 100%); ADR-P2-27
(re-gate, step-down); migration 0053 (`capital_ramp`).
**Depends on:** Phase 2.
**Blocks:** nothing buildable — Stage 6 is a human decision (roadmap H-4).
**Migration range:** 0111–0112.

Gate 16 describes a ladder nothing climbs. This phase builds the mechanics so that **if and
when** roadmap gate H-4 (capital and liability) is answered, the path from a `strict_v1` pass to
capital is a mechanism someone reviews rather than a decision someone improvises.

**Nothing in this phase moves money.** Every task builds and tests the ladder against paper
deployments. Ring 3 has no tool, and this phase does not add one.

---

### T-3.1 The ladder, as a state machine — M
`NOT STARTED`

`capital_ramp` (migration 0053) driven as an explicit state machine: 10% → 25% → 50% → 100%,
one step per review period, each step requiring the review period's evidence to still satisfy
the profile that authorised the deployment. A step is an event, not a column update, so the
ladder's history is reconstructible.

Steps are permitted only for deployments whose profile can authorise capital — which
`paper_v1` cannot (T-1.2). On paper, the ladder runs against paper capital, which is how it
gets tested without money.

**Files:** `crates/domain/src/capital_ramp.rs`, `crates/ledger` events, migration 0111.
**Acceptance:** a step advances only with satisfying evidence from the completed review period;
the ladder's history replays from events to the same state; a `paper_v1` deployment cannot enter
the capital ladder at all; skipping a rung is impossible by construction.
**Traces:** SPEC §12.3 Gate 16; migration 0053.

### T-3.2 Gate 16 judges the step — M
`NOT STARTED`

Gate 16 currently has no evidence source. Give it one: the completed review period's forward
observations (Phase 0), the divergence record (Phase 2), and the profile's thresholds. A step-up
is a gate verdict like any other, with its evidence stored, and it renders `NotComputable` with
a reason rather than passing when the period is incomplete.

**Files:** `crates/backtest/src/gates/`, migration 0112.
**Acceptance:** Gate 16 renders a verdict from a completed review period and `NotComputable`
with a stated reason from an incomplete one; the verdict's evidence is stored and reproducible;
a step-up with a failing Gate 16 is refused at the state machine, not just warned about.
**Traces:** SPEC §12.3 Gate 16; HANDOFF §5.3 (Gate 16's missing input).

### T-3.3 Step-down: 50% per period, to zero — M
`NOT STARTED`

ADR-P2-27's other half, and the one that actually applies today. An ungated or failing
deployment may not increase size, holds for one review period while a `strict_v1` or `paper_v1`
pass is attempted with its record, and **steps down 50% per period to zero** if it fails.

The step-down is automatic. It is not a recommendation on a dashboard, because the whole point
of ADR-P2-27 was that nothing gets to sit at its current size indefinitely on the strength of
never having been measured.

**Files:** `crates/domain/src/capital_ramp.rs`, a scheduled review worker.
**Acceptance:** a deployment that fails its review period is halved automatically at the period
boundary and reaches zero after the expected number of periods; the schedule is driven by
recorded period boundaries, not wall-clock drift; a step-down is recorded as an event with its
reason; no configuration permits skipping a step-down.
**Traces:** ADR-P2-27; D-T4.

### T-3.4 The capital gate is a document, not a code path — M
`NOT STARTED`

The final task of this set is deliberately not code. Write `docs/operations/capital-gate.md`:
the checklist that must be true before any capital deployment is even proposed — roadmap H-4
answered in writing (whose broker, whose money, who is liable), a `strict_v1` pass with its full
verdict, the fleet's divergence record, the drill log, the ramp starting at 10%, and the named
human who decides.

Then assert in code that no path exists to a capital deployment today: a test enumerating every
deployment entry point and asserting each produces a paper venue class.

**Files:** `docs/operations/capital-gate.md`, `crates/api/tests/no_capital_path.rs`.
**Acceptance:** the document exists and names every precondition including the human decision;
the test enumerates every entry point and asserts no capital venue is reachable; adding a capital
venue class fails that test, so enabling capital is a deliberate, reviewed change rather than a
configuration discovery.
**Traces:** ROADMAP §4 Stage 6, §8 H-4; Q-4; ADR-0025; D-12.

---

## Phase exit

- The ladder and the step-down are mechanisms with event histories, tested against paper capital.
- Gate 16 has an evidence source and refuses incomplete periods.
- The route to capital is written down, and code asserts that it is currently closed.
