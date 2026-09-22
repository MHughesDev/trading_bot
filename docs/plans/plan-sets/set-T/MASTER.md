# Forward Evidence — Set T

**Completion: 88% left (1 / 17 tasks). 1 done, 3 partial, 1 BLOCKED:HUMAN, 12 not started.**

> **This line read `0% (0 / 17 tasks). NOT STARTED.` until 2026-09-21** — while
> this set’s own phase files recorded real work. It was wrong by the maximum
> possible amount for six days, because the test that holds every other count
> honest scans `phase-*.md`, and a MASTER is not a phase. The file whose name
> promises the summary was the only one outside the mechanism that checks
> summaries. `every_set_master_counts_its_own_set` now covers it.

**Status:** DRAFT (2026-09-15) — ready for execution.
**Created:** 2026-09-15
**Stage:** [0 through 3](../../ROADMAP.md#4-the-stages) — **Phase 0 starts immediately.**
**Derived from:** ML platform [SPEC §12.3](../../../ml-platform-context-pack/spec/SPEC.md) (Gates 15, 16),
HANDOFF §5.3 ("forward-test observations (15) need the reconciliation crate wired"),
ADR-P2-14 (`paper_v1`), ADR-P2-27 (re-gate, ramp down, nobody grandfathered),
[COMP-002](../../../specs/COMP-002-execution-and-risk-gate.md),
[COMP-007](../../../specs/COMP-007-live-monitor-and-kill-switch.md),
[AGENT-013 §4](../../../specs/AGENT-013-modes-and-research-lifecycle.md) (the four rings),
migrations 0053 (capital ramp) and 0054 (approval envelope), OQ-08 option A.
**Requirement IDs:** FE-G15, FE-G16, FE-DEP-01…04, FE-REC-01…05, FE-RAMP-01…04.
**Migration range:** 0105–0112.
**Scope class:** End-state architecture.

---

## 1. Why this set exists

**Gate 15 has no input.** Of the platform's sixteen gates, five judge on nothing and one of
those five — forward-test observation — is the only gate that cannot be satisfied by computing
harder. It needs a strategy to have traded, forward, in observed time, and for those fills to
be compared against what the backtest said would happen.

That comparison is the single most informative measurement the platform can make. Every other
gate asks whether the evidence is internally sound. Gate 15 asks whether the *simulator is
telling the truth* — and a simulator nobody has checked against reality is a machine for
producing confident wrong answers at scale.

**The clock starts when the first strategy reaches paper, and not before.** Which is why this
set's Phase 0 sits in Stage 0 of the roadmap beside the forward collectors, ahead of the agent
work, and why its first deployment should be a **hand-authored strategy** — waiting for the
agent to produce a good candidate delays the clock by however long Set S takes, for no gain.
The agent's value here is volume later, not the first observation now.

**What this set is not.** It is not live trading. Ring 3 has no tool and never will
(AGENT-013 §4, D-12, ADR-0025). Paper deployment is ring 2 and is reached only by a proposal a
human decides. Capital is roadmap gate H-4, which is a conversation about liability, not a
build task.

---

## 2. Scope

### In scope

| Area | Today | Set T delivers |
|---|---|---|
| Reconciliation | `crates/reconciliation` exists, unwired | paper fills reconciled against backtest distributions, continuously |
| Gate 15 | `NotComputable` | real forward observations, counted, with a stated observation window |
| Paper deployment | manual paper orders exist; no proposal path | `Proposal` → human decision → deployment, with the envelope recorded |
| The legacy route | `POST /api/backtests` registers under `legacy_unlogged` | retired — OQ-08 option A, "when the frontend is next touched" |
| Divergence | nothing compares live to expected | a divergence detector that halts before it compounds (SC-6) |
| Capital ramp | migration 0053 exists; no mechanics | Gate 16's 10/25/50/100% ladder and ADR-P2-27's step-down |
| Monitor | COMP-007 built in Set S Phase 7 | running in production with drills that prove fail-closed |

### Out of scope

- **Live capital.** Roadmap H-4 (Q-4, liability) first, then `strict_v1`. No task here moves
  money.
- **The monitor's construction** — that is Set S Phase 7. This set *operates* it and proves it.
- **New execution venues.** Alpaca paper is the paper venue (ADR-0006, Q-1, resolved).
- **The agent authoring the deployed strategy.** Deliberately: Phase 1 deploys something
  hand-authored so the clock starts now.

---

## 3. Locked decisions carried into this set

| ID | Decision | Source |
|---|---|---|
| D-T1 | Gate 15 evidence comes from **observed-knowledge rows only**. A backfilled row never becomes forward evidence | ADR-P2-24, D-P3 |
| D-T2 | Paper deployment is ring 2: reachable only through a `Proposal` a human decides. The agent may propose; deployment is a human action | AGENT-013 §4, D-S12 |
| D-T3 | Nobody is grandfathered. Existing running strategies get a `legacy_ungated` marker — a fact, never a permission — and their forward record from today counts as Gate 15 evidence | ADR-P2-27 |
| D-T4 | An ungated strategy may not increase size, holds for one review period, and steps down 50% per period to zero if it fails its profile | ADR-P2-27 |
| D-T5 | `paper_v1` authorises paper only and can never authorise capital; `strict_v1` is unchanged and immutable | ADR-P2-14 |
| D-T6 | Divergence halts before the next order, not after (SC-6). Reconciliation is a safety mechanism first and a measurement second | `artifact.md` SC-6, FM-4 |

---

## 4. Phase summary

| Phase | Title | Tasks | Migrations | Stage | State |
|---|---|---|---|---|---|
| 0 | [Reconciliation and the first observation](phase-0-reconciliation.md) | 4 | 0105–0106 | 0 | NOT STARTED |
| 1 | [The deployment path](phase-1-deployment-path.md) | 5 | 0107–0108 | 1–2 | NOT STARTED |
| 2 | [The paper fleet](phase-2-paper-fleet.md) | 4 | 0109–0110 | 3 | NOT STARTED |
| 3 | [Ramp mechanics](phase-3-ramp-mechanics.md) | 4 | 0111–0112 | 3 | NOT STARTED |

### Dependency graph

```
Phase 0 ──▶ Phase 1 ──▶ Phase 2 ──▶ Phase 3
   │                       ▲
   └── (Set S Phase 7: the monitor) ──┘
```

Phase 0 depends on nothing in Set S and can start today. Phase 2 needs the monitor from Set S
Phase 7 to be running, which is why Phase 7 is one of the tracks that starts early.

---

## 5. The agent contract

As [Set S §5](../set-S/MASTER.md#5-the-agent-contract), with these differences:

- **Migration numbers come only from 0105–0112.**
- **This set touches the execution path.** `crates/risk` is the single chokepoint (SC-2) and
  nothing here may create a second one. Every order this set causes passes the same gate as a
  manual order, and a test proves it per entry point.
- **No task here may introduce a live-venue code path.** A test asserts the paper venue is the
  only reachable one from anything this set adds.
- **Halting is correct behaviour.** When reconciliation finds divergence, the system stops
  submitting. Do not add a flag that continues; if one is requested, it is an ADR and a
  conversation, not a config key.

---

## 6. Exit criteria

1. Paper fills reconcile against the backtest's return distribution, and the comparison is
   stored as Gate 15 evidence with its observation window stated.
2. Gate 15 renders a real verdict for at least one strategy — no `NotComputable`.
3. A strategy reaches paper only through a proposal a human decided, recorded with the approval
   envelope; no other path exists, proven per entry point.
4. A planted divergence halts order submission before the next order, in a drill.
5. The legacy `POST /api/backtests` route is gone and the Back Testing UI runs on the
   Experiment path (OQ-08 option A).
6. Every currently-running strategy carries its `legacy_ungated` marker and its forward record
   is being captured.
7. A kill-switch drill stops order flow within one cadence, with the monitor as an independent
   process (COMP-007 §8).

---

## 7. Coverage

| Source | Criteria | Tasks |
|---|---|---|
| SPEC §12.3 Gate 15 | forward observations | T-0.1…T-0.4 |
| SPEC §12.3 Gate 16 | capital ramp ladder | T-3.1, T-3.2 |
| ADR-P2-27 | re-gate, no grandfathering, step-down | T-3.3, T-3.4 |
| ADR-P2-14 | `paper_v1` authorises paper only | T-1.2 |
| AGENT-013 §4 | rings; proposal as the only door | T-1.1, T-1.3 |
| OQ-08 option A | legacy route retired | T-1.5 |
| `artifact.md` SC-6, FM-4 | divergence halts | T-0.3 |
| COMP-007 §8 | drills prove fail-closed | T-2.3, T-2.4 |

---

## 8. Risks

| Risk | Mitigation |
|---|---|
| Reconciliation is built as a report nobody reads rather than a halt nobody can bypass | D-T6 and T-0.3: the halt is the acceptance criterion; the report is a by-product |
| The first deployment waits for a "good enough" candidate and the clock never starts | Phase 1 deploys a hand-authored strategy deliberately; goodness is not the point, observation is |
| Gate 15 evidence quietly includes backfilled rows and the gate becomes decorative | D-T1, enforced by the same flag Set P P-1.6 makes structural |
| Retiring the legacy route breaks the Back Testing UI mid-flight | T-1.5 moves the UI first and deletes the route second, in that order, in one PR |
| Paper divergence is chronic and everything halts constantly | the detector's thresholds are declared per strategy on deployment and recorded; a chronic halt is a finding about the simulator, which is exactly what this set exists to surface |
| A "temporary" live path appears because paper felt proven | §5: no live-venue code path, asserted by a test; capital stays behind roadmap H-4 |

---

## 9. Progress log

| Date | Phase | Task | Note |
|---|---|---|---|
| 2026-09-15 | — | — | Set T created from SPEC §12.3, HANDOFF §5.3, ADR-P2-14/27 and COMP-002/007. Placed at Stage 0 so the Gate 15 clock starts before the agent work lands. Nothing implemented. |
