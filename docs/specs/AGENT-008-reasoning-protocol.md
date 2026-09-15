# AGENT-008: The Reasoning Protocol — how the agent plans, acts, verifies and stops, at every tier

**Status:** Proposed (implementation spec; ready to build)
**Date:** 2026-09-14
**Author:** Claude (Fable 5.1), from `crates/harness/src/{drive,validation,canary,policy}.rs`, `docs/LOCAL_TIER_FINDINGS.md`, ADR-0032, and the research brief Theme 9
**New:** no prior spec covers the loop's cognitive structure; ADR-0032 and HARNESS_CONFORMANCE items 4, 9, 12–13 describe pieces of it
**Depends on:** AGENT-005 (working state budget), AGENT-006 (`report.*`, workflows), AGENT-007 (findings, brief), AGENT-010 (the single loop this protocol runs in)

---

## 0. Why this spec exists

The loop in `crates/harness/src/drive.rs` (2,100 lines) already contains a state machine (`State`, `:93`), a plan grammar (`PlanStep`, `:468`), two-step constrained decoding (`Decode`, `:156`), a validation ladder with a retry budget (`validation.rs:27, :401`), an adversarial canary (`canary.rs`), a no-progress breaker (`MAX_BARREN_STEPS`, `drive.rs:318`), and typed termination with an id check (`drive.rs:1830`). These are good mechanisms. What they lack is a *protocol*: a stated shape for a step that says what the harness guarantees before, during and after the model acts, and what the model is actually being asked to do at each tier. The evidence that this matters is in `LOCAL_TIER_FINDINGS.md` §8–9: nine of the ten failures that made the local tier "unreachable" were the harness overruling a model that had done the right thing (alphabetical truncation hiding a searched-for tool, a truncated reply read as a grammar violation, `finish_task` eating the only step, an unread `idempotent` flag producing twelve identical calls). A protocol is how those stop recurring.

Three things are missing outright: an explicit working-state object the model can read instead of reconstructing from a transcript; a verification phase between acting and recording; and a reasoning step that costs nothing in tool budget for tiers that can use one. And one open bug: the planner emits non-tool steps (`LOCAL_TIER_FINDINGS` §9, "still open").

## 1. Principles

- **P1 — The harness owns state; the model owns judgement.** Goal, criteria, plan, evidence, budget and errors are rendered *to* the model every call. The model is never asked to remember what happened; it is asked what to do next.
- **P2 — A step is plan → act → verify → record.** Nothing enters memory or a claim without passing verification, and verification is deterministic first, model-assisted second, and only on tiers where the model can be trusted to assist.
- **P3 — Plans are made of tools.** A plan step names a catalogue tool or workflow, or it is not a plan step. Thinking is a step type of its own (`reflect`), and only on profiles that declare it.
- **P4 — The decode ladder is per tier and declared.** Native tool calls; select-then-fill under a grammar; single-shot fallback. The model is never asked to produce a shape the ladder has not verified it can produce (the canary).
- **P5 — Corrections are specific and cheap.** A rejected call gets one sentence naming the rung it failed and the fix (`validation.rs` does this); three identical failures change the approach, not the wording.
- **P6 — Stopping is a decision with evidence.** `finish_task` with ids the run produced; "I could not determine this" with the tools that were tried; escalation to a stronger tier with the working state attached. Prose never ends a task; a budget never silently ends one either — it buys a last turn to report.
- **P7 — Understanding beats coverage.** A lookup goal is answered by reading; a research goal follows the protocol; the agent says which it is doing and why, in the working state, before it spends a trial.

## 2. The working state

[DO] Introduce `WorkingState` in `crates/harness/src/state.rs`, owned by `Loop`, replacing the ad-hoc `WorkingState` blocks pushed at nine sites in `drive.rs` (`:926, :963, :1476, :1544, :1624, :1751, :1818, :1986, :2033`):

```rust
pub struct WorkingState {
    pub goal: String,
    pub kind: GoalKind,                       // Lookup | Research | Diagnose | Compare — set at step 0 (§3.1)
    pub mode: Mode,                           // Chat | Research — AGENT-013 §2; Chat forces Lookup; a continuous session's children are Research runs with a session_id
    pub workflow: Workflow,                   // investigate | falsify | replicate | compare | diagnose | survey | optimise — AGENT-013 §3
    pub ring: Ring,                           // 0 research | 1 platform artifacts | 2 paper — AGENT-013 §4; filters exposure before routing
    pub stage: Stage,                         // the ten lifecycle stages — AGENT-013 §5; transitions are effects
    pub prereg: Option<PreregRef>,            // the sealed registration and its hash — AGENT-013 §7.2
    pub acceptance: Vec<String>,              // what "done" means, from the goal + constraints
    pub hypothesis: Option<Hypothesis>,       // {family_id, statement, falsifier, role, status: Untested|Supported|Refuted|Inconclusive} — status is platform-written (AGENT-013 §8.1)
    pub plan: Vec<PlanStep>,                  // each with status: Pending|Active|Done|Skipped{why}|Failed{why}
    pub evidence: Vec<EvidenceRef>,           // ids seen this run, with the tool and step that produced them
    pub findings: Vec<FindingRef>,            // ids of verified findings (AGENT-007 §3.1)
    pub open_questions: Vec<String>,
    pub last_error: Option<Correction>,       // the most recent rejection, rendered once
    pub budget: BudgetView,                   // steps/tokens/usd remaining, trials spent, n_eff
    pub decided: Vec<Decision>,               // §3.6: choices made and their reason, so they are not re-made
}
```

It is rendered by tier: the flat tier gets a ≤ 400-token card (goal, active step, last error, budget, three most recent evidence ids, findings count); the rich tier gets the card plus plan, hypothesis and open questions. It is one `Section::Subtask` block at priority 255, rewritten each step (never appended — the lesson of AGENT-005 §4.4). The anchor text (`drive.rs:1168`) is derived from it.

## 3. The step

### 3.1 Step 0: classify the goal

[DO] Before any plan, the harness runs a **deterministic classifier** over the goal and constraints (keyword and shape rules, then — on profiles with `reflection: true` — one model call with a fixed prompt whose only output is `{kind, acceptance[]}` under a grammar). `Lookup` goals get a plan of at most three read-only tools and no trials; `Research` goals get the research protocol's phases as plan scaffolding (AGENT-005 §3.3) and the task brief (AGENT-007 §4). The classification is written into `WorkingState.kind` and shown to the model; the model may contest it via `reflect` (§3.5), which re-runs the classifier with the model's reason attached.

This is the "many goals are lookups" instruction from the charter (`prompt.rs:15`) turned from a plea into a mechanism.

**Mode constrains the classifier** (AGENT-013 §2.2, added 2026-09-14). A `Chat` run forces `Lookup`: when the rules or the model would classify the goal `Research`, `Diagnose` or `Compare`, the turn does not plan — it ends with a *Research offer* carrying the classification, the workflow it implies, and the estimated cost, and the user decides whether to start a run. A `Research` run's classification also selects the workflow's stage skeleton (AGENT-013 §3), which is the plan scaffolding this section previously left to the protocol prose.

### 3.2 Plan

- `PlanStep` (`drive.rs:468`) gains `kind: Tool { name } | Workflow { name } | Reflect | AskUser`, and the plan grammar (`drive.rs:818-849`) enumerates **tool and workflow names**, not free text. A step the parser cannot bind to a catalogue entry is rejected at parse with the correction "each step names a tool; if none fits, use search_tools". This closes the open planner bug.
- The planner's `namespace` field is deleted; routing is the catalogue search over the step's goal, which already outranks the planner's declaration (`drive.rs:1016-1041`).
- Plans are short: `max_plan_steps` per profile (frontier 8, local 4). Longer work is re-planned after the plan is exhausted, with the working state carrying what was done.
- `mode: freeform` (frontier profile) keeps the planner optional: the frontier model may act without a plan, but every act still passes §3.4 and the working state still tracks it as an implicit step.

### 3.3 Act: the decode ladder

Declared per profile (`output.constrained_decoding`, `native_tool_template`, and a new `output.decode: native | select_fill | single_shot`), verified by the canary before step 1 (`canary.rs:131`), enforced by the adapter (`adapter.rs:64` `verify`):

| Rung | Who | Mechanism | Notes |
|---|---|---|---|
| `native` | frontier, `local_high` with vLLM native tools | provider tool calls, ≤ `max_exposed_per_step` schemas | parallel calls allowed if `tools.parallel_calls` |
| `select_fill` | `local_mid`, `local_small` | call 1: enum grammar over exposed names (`drive.rs:1103`); call 2: the one flat schema as `guided_json` (`drive.rs:1391`) | the exposure budget *is* the grammar; an unrouted tool is unnameable |
| `single_shot` | `degraded` | one call, one tool, `finish_task` next | earnable upgrade via `MultiStepEvidence` (ADR-0032) |

[DO] Few-shot exemplars (`few_shot_examples`, today read by nothing) are two canned select→fill pairs pushed as a `FewShot` block on `select_fill` profiles, chosen by the step's namespace from `crates/harness/src/fewshot/<namespace>.json`. Measured in AGENT-009 §3 before it is kept.

[DO] `ModelTruncated` (`drive.rs` `Input::ModelTruncated`) on a `select_fill` reply re-asks once with `max_tokens` doubled and the instruction "answer with the JSON only"; on `native` it appends the nudge the hosted driver already uses (`driver.rs:580`). Never fences on truncation (LOCAL_TIER_FINDINGS §8 bug 1).

### 3.4 Verify

Every tool result passes, in order, before it is shown to the model:
1. **Schema/shape** — `ToolResult` (AGENT-006 §2.4) or the legacy JSON; error results are typed (`Input::ToolError`).
2. **Provenance tagging** — `Tagged` with the source (`provenance.rs`), so untrusted content is fenced.
3. **Idempotent-repeat guard** (exists) — identical read repeated → the cached result plus the notice at priority 215; three repeats → `Correction("you have this result; use it or change the call")`.
4. **Evidence capture** — every id in `ids` is added to `WorkingState.evidence` with tool and step.
5. **Numbers get n** — a numeric field without `n` is rendered with "(n unknown)" so the model cannot cite it as established.

And every *claim* the model makes passes, before it is recorded or reported:
6. **Deterministic recheck** — ids exist in `evidence`; numbers appear in the cited result (AGENT-007 §3.1). Fails → correction, not rejection: the finding is stored `suggestive` and the model is told why.
7. **Tool-grounded recheck** (`report.recheck_claim`, AGENT-006 §5.8) on `established` claims that will appear in `finish_task`, on profiles with `reflection: true`: re-run the statistic or re-query the ledger; a discrepancy blocks the report.
8. **Critic** (`report.adversarial_critique`) in a fresh session on `frontier`/`local_high` before a gate advance or a finish with `established` findings; on smaller tiers a **deterministic checklist critic** runs instead (every acceptance criterion has an evidence id; every trial is in a family; no `suggestive` finding is presented as established; the null comparison ran). The checklist is the same list the human reviewer sees in the dossier.

### 3.4a A steer that changes the goal re-classifies

`WorkingState.goal` is set at step 0 and the acceptance criteria derive from it. A steer can
change what is being asked ("actually, check the 4h too", "forget the edge, just tell me the
coverage") — and continuing to plan against a stale acceptance set is how an agent finishes
confidently against the wrong question.

On `Input::Steer` the harness classifies the text deterministically as one of three:

| Steer kind | Effect |
|---|---|
| **Constraint** ("use 4h", "stop after two sweeps") | narrows the existing goal; plan kept; acceptance amended; recorded in `decided` |
| **Redirection** (the goal changes) | re-runs §3.1 classification, rewrites `acceptance`, marks remaining plan steps `Skipped{why: "goal changed at step N"}`, re-plans. **Findings and evidence are kept** — they were true about the data, not about the goal |
| **Comment** (encouragement, context, an answer to a question) | appended to the working state; no re-plan |

An ambiguous steer is treated as **Redirection** and the model is told so: an unnecessary
re-plan costs one step, a missed one costs a whole run aimed at the wrong target. The old
goal and the reason are kept in `decided`, so a reader can see the run changed direction and
why.

### 3.5 Record and reflect

- **Record**: `record_finding` (verified) or the harness-derived negative result (AGENT-007 §3.2). A step that ends without either, and without a tool call that produced new evidence, is *barren*; three barren steps trigger the forced wrap-up turn (`drive.rs:924`) — unchanged.
- **Reflect** (new core tool, `reflect(observation, change_of_plan?)`): costs a model call but **is not counted against `max_steps`**, capped at `max_reflections_per_run` (default 3, at most one per step). A 15-step local budget that charges 7% of itself for the agent to notice it is going the wrong way is a budget that discourages noticing. Its only effect is a `WorkingState` update (open question added, plan step skipped with a reason, hypothesis status changed, goal-kind contested). Exposed only on profiles with `reflection: true`. It is how the model changes course *explicitly* instead of drifting — and the working state shows the human what it decided.

### 3.6 Decisions are remembered

Every choice the model makes that the protocol asked it to make — which instrument, which timeframe, which null, which change to try next — is written to `WorkingState.decided` with its reason, and the renderer shows it. A model that re-asks a decided question gets `Correction("decided at step 4: …; say why it should change or move on")`. This is the single largest source of wasted steps in the local-tier traces (re-listing instruments, re-choosing timeframes).

### 3.7 Liveness: the two loops the protocol could otherwise sustain

Plans are short and re-planned when exhausted (§3.2), and a rejected call is corrected and
retried (P5). Both are right, and both are unbounded as written — a model can re-plan
forever, and it can keep calling a tool that is simply broken. The barren-step breaker
catches neither, because re-planning and retrying both look like progress.

**Re-plan budget.** `max_replans_per_run` (default 3; 2 on local tiers). On exhaustion the
run does not fail: it enters `wrapping_up` with the working state intact, so the outcome is a
report of what was established and what defeated it — which is a finding. A fourth plan for
the same goal is not thinking, it is drifting.

**Broken-tool circuit breaker.** Three consecutive non-retryable errors from the *same tool*
in a run open a breaker on that tool for the rest of the run. The tool is removed from
exposure, the model is told once ("`get_bars` failed three times: <last fix>; it is not
available for the rest of this run, route around it or finish"), and the failure is recorded
as a `platform_defect` candidate (AGENT-007 §2.2) rather than absorbed silently. An agent
that spends fifteen steps on a tool the platform has broken is the platform's fault, and the
harness is the only party in a position to notice.

Both limits are profile values with conformance readers, both are reported in the scorecard,
and neither is a wall clock: they bound *repetition*, not duration.

## 4. Stopping

**Cost is an approval axis, not only reversibility.** §6's checkpoint policy gates on
reversibility and confidence, which is right and incomplete: a reversible action can still be
expensive, and "reversible" is no comfort after a sweep has spent a day of budget. A call
whose `estimate_cost` (AGENT-006 §2.4b) exceeds `budgets.ask_above_usd` (default: 20% of the
task budget, or any single call over $5) requires a human, **whatever its reversibility**, and
the approval card shows the estimate beside the remaining budget. Below that line the agent
spends freely — the point is not to interrupt, it is to make sure the one decision that is
genuinely the human's actually reaches them.

| Trigger | Behaviour |
|---|---|
| acceptance met | model calls `finish_task`; §3.4 rungs 6–8 run; `Outcome::Finished` |
| cannot be met | `finish_task` with `result` stating what was tried and `evidence` naming the runs; the harness accepts an outcome of "could not determine" as complete when ≥ 1 relevant tool was called |
| budget nearly spent | `wrapping_up` (exists): one last turn with only `finish_task` exposed |
| no progress | `MAX_BARREN_STEPS` → wrap-up |
| needs a stronger model | `Outcome::Escalated` (exists) **now carries the full `WorkingState`** so the next tier resumes rather than restarts (AGENT-010 §6) |
| needs a human | `AskHuman` on any tier (AGENT-010 §7); the working state records the question and the answer becomes a `user_preference` when scoped |
| backend broke a guarantee | `Degradation` → fenced (exists) |
| pre-registration amended a third time | `blocked: protocol_unstable`; the report turn runs (AGENT-013 §5.3) |
| refinement chain at diminishing returns, or the same failure class twice | the chain ends; `report` with the negative result and its `retry_condition` (AGENT-013 §5.9) |

Two bare prose replies no longer fail the run outright (`driver.rs:595`): the second reply is parsed for a report shape; if it has a result and ids, it is treated as a malformed `finish_task` and the model is asked to call the tool with those exact fields (one more turn). Failing that, `failed` as today.

## 5. Tier-specific behaviour, in one table

| | frontier | local_high | local_mid | local_small / degraded |
|---|---|---|---|---|
| goal classifier | rules + model | rules + model | rules | rules |
| plan | optional (`freeform`) | required, ≤ 8 | required, ≤ 4 | required, ≤ 3 / none |
| decode | native | native or select_fill | select_fill | select_fill / single_shot |
| few-shot | 0 | 0 | 2 | 2 |
| reflect | yes | yes | no | no |
| recheck (rung 7) | yes | yes | no | no |
| critic (rung 8) | fresh-session model | fresh-session model | checklist | checklist |
| working-state card | full | full | short | short |
| max searches/step | 2 | 2 | 2 | 1 |

Everything in this table is a profile value read by the loop, and `tests/conformance.rs` needs a reader for each new one (`goal_classifier`, `max_plan_steps`, `decode`, `critic`).

## 6. Build order

| Phase | Items |
|---|---|
| 1 | `WorkingState` and its renderer replacing the nine block sites; tool-shaped plan grammar (closes the planner bug); `decided` |
| 2 | goal classifier (rules); verification rungs 1–6 as one function; truncation re-ask |
| 3 | `reflect`; few-shot blocks; rung 7 recheck on capable tiers |
| 4 | critic: checklist for small tiers, fresh-session for large (needs AGENT-010 sub-sessions) |
| 5 | escalation carrying working state; prose-report salvage |

## 7. Acceptance

> **Form note (Set S task S-0.4, 2026-09-15).** Converted from a plain numbered
> list to the repository's acceptance form so `docs/procedures/execute-plan.md`
> step 3c can close the chain requirement -> criterion -> evidence. No criterion
> was added, removed, reworded or softened; each kept its position, so item *n* of
> the old list is `AC-n`. An unticked box with `Verified by: [--]` means nothing
> has been proved, which is the honest default.
>
> The plan's S-0.4 gives the target form as `6.1.A [ ] ... - Verified by: [--]`.
> The form actually used across the tracked specs (DATA-003, COMP-001..004,
> FEAT-001, SYS-001) is `- [ ] AC-n: ... - Verified by: [--]`, and that is what
> execute-plan step 3c ticks. This uses the tracked form; the plan's example is
> stale.

- [ ] AC-1: No `WorkingState` content is appended across steps; the block is rewritten and its rendered size is bounded per tier. - Verified by: `harness::state::tests::the_card_stops_growing_once_it_is_populated` (200 updates change the card by < 40 bytes) and `a_flat_card_is_bounded_however_long_the_goal_is`. **Partial:** proved of the type; `Loop` does not own one yet, so the nine append sites in `drive.rs` are still live.
- [ ] AC-2: A plan containing a non-tool step is rejected at parse with the named correction; the replay eval that exposed the bug passes. - Verified by: [--]
- [ ] AC-3: On `qwen2.5-7b`, a lookup goal completes with ≤ 3 tool calls and 0 trials; a research goal on the same profile reaches a plan whose every step is a catalogue name. - Verified by: [--]
- [ ] AC-4: A claim citing an id not in `evidence` is corrected, never stored `established`. - Verified by: [--]
- [ ] AC-5: Re-asking a `decided` question is corrected with the step number of the decision. - Verified by: `harness::state::tests::re_asking_a_decided_question_is_corrected`. **Partial:** the correction is produced by `WorkingState::redecision_correction`; nothing in the loop calls it yet.
- [ ] AC-6: Escalation resumes on the stronger tier from the carried working state without re-running completed steps (verified by trial count). - Verified by: [--]
- [ ] AC-7: The truncated-reply fence from LOCAL_TIER_FINDINGS §8 cannot recur: a truncated select reply produces a re-ask, not a `Degradation`. - Verified by: [--]
- [ ] AC-8: The working state carries `mode`, `workflow`, `ring`, `stage` and `prereg`; a Chat run classified `Research` produces a Research offer and no plan; a stage transition the workflow's skeleton forbids is refused and recorded. *(Added 2026-09-14, AGENT-013.)* - Verified by: [--]
## 8. Non-goals

- Chain-of-thought capture or scoring: the model's private reasoning is not stored; its *decisions* are.
- Multi-agent debate as verification (rejected on evidence, research brief Theme 3).
- Learned planners: plans come from the model under a grammar, or from a playbook (AGENT-007 §2.3).

## 9. Open questions

- Whether the rules-only goal classifier is good enough on the 7B or whether one grammar-constrained model call is worth its cost there; AGENT-009 decides.
- The right `max_plan_steps` per tier; start with the table, tune on trajectories.
