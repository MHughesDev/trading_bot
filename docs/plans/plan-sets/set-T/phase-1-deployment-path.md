# Phase 1 — The deployment path

**Completion: 0% (0 / 5 tasks)**

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
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

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
