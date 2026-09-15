# ADR-0037: Two Agent Modes — Chat and Research — with a Continuous Toggle on Research; "Endless" Is the Toggle's `replenish` Setting

**Status:** Proposed
**Date:** 2026-09-14; revised 2026-09-15 (three modes → two modes and a toggle, on the user's decision)
**Spec:** `docs/specs/AGENT-013-modes-and-research-lifecycle.md`, `docs/specs/AGENT-014-continuous-research.md`
**Amends:** AGENT-012 §3.3 (the composer's mode control), AGENT-008 §2 (working state gains `stage`, `mode`, `workflow`, `ring`), AGENT-010 §4 (runs carry `mode`, `workflow`, `session_id`)
**Related:** ADR-0025 (invariants enforced at services), ADR-P2-04 (event-sourced campaign on the job service), ADR-0035 (one loop), INV-16, INV-21

## Context

The platform's North Star is a persistent research agent that "supports both focused
assignments and continuous research across long-running sessions", measured by the quality
of its research decisions and the reliability of its evidence. Set S specifies the harness
(honest context, catalogue, memory, protocol, runtime, evals, authoring, desk, monitor) but
never the *unit the user chooses*: the desk's composer offered *Ask / Research / Diagnose /
Write strategy*, which mixes a goal classification, a workflow and an editor, and nothing said
what happens when the agent should keep going after a run ends.

The user's proposal was two modes — Chat and Research — with Research carrying an optional
*Endless* setting. Three objects already exist nearby and none is the missing one: a **run**
(one goal, one working state, designed to end — AGENT-008 §4), the platform's
**`mlops.campaign`** (an optimisation over one hypothesis family under one DEFINE — SPEC
§10, built), and FEAT-003 §8's campaign loop (the agent's strategy inside HYPOTHESIZE).

An earlier draft of this ADR proposed a third mode ("Program") on the argument that continuous
research differs from a run in unit of work and completion condition. The user decided
(2026-09-15) that the user-facing surface shows **two options and a toggle**, and that the
continuous machinery is a set of *qualifiers* on Research. This revision records that decision
and how the design keeps it honest.

## Decision

1. **Two user-facing modes: Chat and Research.** A mode fixes what the agent may *spend*
   (trials) and *write* (rings). Chat spends no trial, ever, and writes only its workspace and
   `user_preference`; Research spends pre-registered trials and proposes into rings 1 and 2.

2. **Research carries a Continuous toggle.** Off: a question, one run, one verdict. On: the
   question field becomes a **charter**, the run becomes an **agenda** of runs, and the session
   acquires a budget over days, a schedule, autonomy, a stall detector, a digest and
   `replenish`. The toggle changes what the user hands over and what comes back; it changes
   nothing about the rings, the lifecycle or the gates, which apply to every child run.
   **"Endless" is `replenish: on`.**

3. **The toggle is honest because the object is one.** `ResearchSession { continuous }`
   (AGENT-014 §2.1): a bounded Research run is the one-item case of the same fold. The
   continuous machinery is not hidden inside a run designed to end, and the user is not asked to
   learn a third word. Nothing in the desk is called a program.

4. **Diagnose, Falsify, Replicate, Compare, Survey and Optimise are workflows inside
   Research**, not modes: they share the run object, the gates and the dossier, and differ
   only in required stages. "Write strategy" is an action (the SLv2 editor). "Operate" is
   COMP-007 and is deliberately not the agent.

5. **Continuous research is built beside `mlops.campaign`, not on it.** A campaign answers
   "search this space well"; a continuous session decides which space is worth searching. A
   session's `optimise` agenda item *is* a campaign, driven through HYPOTHESIZE by a worker
   sub-run of the parent Research run (AGENT-013 §8.5) — which also supplies the `ResearchRun`
   child worker the campaign has been missing (HANDOFF §2). Sessions reuse ADR-P2-04's pattern —
   an append-only event fold driven by a job with idempotent children — and do not reuse its
   tables.

6. **Every trial-spending run is pre-registered and every hypothesis names its falsifier**
   (AGENT-013 P2, P3). The verdict is computed by the platform against the sealed
   registration; the model cannot set it. A refinement must cite a diagnosis and predict its
   effect (P4). Every run reports a `Yield` (§10) — the measurable form of "research produces
   reusable knowledge even when a strategy fails".

7. **Four rings, enforced below the model** (AGENT-013 §4): research (free within budget),
   platform artifacts (proposals under `Promotion`), paper (proposals under `Spend`; deployment
   is a human action), live (no tool exists). A mode is a ring filter on exposure applied
   before routing, so a tool outside the ring is unnameable rather than refused.

8. **Settings decided with the user (2026-09-15):** default autonomy `notify`; exploration
   floor 0.10; day budget defaults from the desk budget policy ($25/day) and is never required;
   continuous research runs on **frontier profiles only**, with a revisit trigger.

## Rationale

- **Modes are the user's contract, not the harness's.** The harness already classifies goals
  (`GoalKind`) and composes workflows (AGENT-006 §6). What the user needs is a small, stable
  set of promises: "this will not spend a trial" and "this will produce verdicts with evidence
  — one, or a stream of them until I stop it or it stalls". Two promises, two modes, one toggle.
- **Continuous research fails in ways a single run cannot.** Duplicate work across days, drift
  into refinement-only motion, budget spent on a broken tool, a stale agenda, an operator who
  cannot tell what happened while away. Each needs a mechanism (AGENT-014 §4 dedupe three
  times, §7.1 stall detector, §6.6 session breaker, §4 subscriptions, §2.4 digest). Making the
  toggle a qualifier that switches on a distinct internal object gives those mechanisms a home
  without giving the user a third mode.
- **Pre-registration and falsifiers are the lifecycle-level form of INV-16.** The trial
  ledger makes every look counted; pre-registration makes every look *pre-committed*, which
  is what stops the evaluation being chosen after the result. The platform already does this
  per Experiment (`prereg_hash`, ADR-P2-33 for Gate 1); extending it to the run and adding the
  falsifier costs one object and closes the largest remaining door.
- **Yield is how "quality of research decisions" becomes a number.** Without it the only
  per-run metrics are tokens, steps and trials, all of which reward motion. Cost per knowledge
  unit rewards the thing the North Star asks for, and it is what the stall detector and the
  `judgement` suite read.
- **Frontier-only for continuous.** A session left alone with a budget over days is the one
  place where a mis-measured tier costs real money silently. Local tiers earn their way in by
  passing `judgement` on three consecutive scorecards, and opening the door is a spec change
  someone has to sign.

## Consequences

- AGENT-012's composer offers *Chat / Research* with a **Continuous** toggle and workflow chips
  under Research; a *Research offer* card is added; the session's conversation gains Agenda and
  Charter tabs; there is no `/desk/programs`. AGENT-008's `WorkingState` gains `stage`, `mode`,
  `workflow`, `ring`; AGENT-010's `agent_runs` gains `mode`, `workflow`, `session_id`,
  `agenda_item_id`. AGENT-007 gains the `open_question` kind and a brief slot. AGENT-009 gains
  the `judgement` suite. AGENT-006 gains the `lifecycle` namespace (twenty-two in total).
- Set S gains Phase 10 (lifecycle, 16 tasks) and Phase 11 (continuous research, 14 tasks);
  migration ranges 0089–0091 and 0092–0096 are reserved.
- Reserved Set R-b's "campaigns, the regime engine, autonomy" is now: campaigns → the
  `optimise` workflow over the existing `mlops.campaign`; autonomy → continuous research; the
  regime engine remains open (it feeds source 4.7 when it exists; the instrument profiler's
  regime field stands in until then).
- Nothing in this decision creates a path from any mode to ring 3. That was true before and
  is now stated where the user can see it.

## Alternatives Considered

- **Three modes (Chat, Research, Program).** The earlier draft. Rejected by the user: the
  surface should show two options and a toggle. Kept from it: the distinct internal object and
  every mechanism; dropped: the third word and the separate pages.
- **Endless as a flag inside a single run.** Rejected: puts agenda state, dedupe, stall
  detection and a multi-day budget inside an object designed to end; gives the user no charter
  or digest. The toggle instead switches the *object* the conversation is (one-item session →
  agenda session).
- **Endless as a setting on the conversation** (keep starting turns). Rejected: a turn is
  answered, not chosen; the choosing is the whole problem.
- **Continuous research as `mlops.campaign` with a wider DEFINE.** Rejected: a campaign's
  DEFINE is one objective over one family with one `delta_practical`; a session spans families
  and objectives, and each child seals its own `delta_practical`. Widening DEFINE would weaken
  the gate the campaign exists to hold.
- **More modes (Diagnose, Audit, Build, Watch, Operate).** Rejected; each is a workflow, an
  action, a source or another service (AGENT-013 §2.5).
- **Letting `autonomous` sessions approve their own ring-1/2 proposals.** Rejected outright:
  the human decides promotion and paper deployment in every setting (AGENT-014 P6).

## References

- `docs/specs/AGENT-013-modes-and-research-lifecycle.md`, `AGENT-014-continuous-research.md`
- `docs/ml-platform-context-pack/spec/SPEC.md` §4.4, §10, §11.5, §12
- `docs/ml-platform-context-pack/backlog/PHASE-2-5-PLAN.md` §2.1; `decisions/HANDOFF.md` §2
- `docs/specs/FEAT-003-strategy-research-agent.md` §8
- `docs/plans/plan-sets/set-S/phase-10-lifecycle.md`, `phase-11-continuous-research.md`
