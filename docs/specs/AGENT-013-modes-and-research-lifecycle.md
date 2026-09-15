# AGENT-013: Modes and the Research Lifecycle — what the user can ask the agent to be, and what every research run must pass through

**Status:** Proposed (product + implementation spec; ready to build)
**Date:** 2026-09-14
**Author:** Claude (Fable 5.1), as architect and product designer
**New:** no prior spec defines user-facing modes or the end-to-end research lifecycle. AGENT-008 defines a *step*; AGENT-006 §6 defines *workflows as composed tools*; SPEC §10 defines a *campaign*; nothing defines the unit the user actually chooses or the stages a research question passes through from intake to knowledge
**Depends on:** AGENT-005 (context), AGENT-006 (catalogue), AGENT-007 (memory), AGENT-008 (protocol), AGENT-010 (runtime), AGENT-011 (authoring), AGENT-012 (desk)
**Related:** AGENT-014 (continuous research — the Research mode with the toggle on), ADR-0037, `ml-platform-context-pack/spec/SPEC.md` §4, §10–§12, ADR-0025, ADR-P2-04, ADR-P2-18, INV-16, INV-21

---

## 0. Why this spec exists

The North Star is a research agent whose success is measured by **the quality of its research decisions, the reliability of its evidence, and its ability to identify robust strategies — or correctly reject weak ones**. Set S builds the harness that makes evidence honest (trial ledger, gates, null library, verified findings) and the loop that makes a step reliable (plan → act → verify → record). It does not say:

- what the user is choosing between when they open the desk — today's composer has a segmented control reading *Ask / Research / Diagnose / Write strategy* (AGENT-012 §3.3), which mixes a goal kind, a workflow and an editor;
- what a research question must pass through before its answer counts — the gate funnel governs a *strategy's* promotion, but nothing governs the *run*: nothing stops a run from choosing its evaluation after seeing a result, refining a strategy without a diagnosed cause, or reporting `inconclusive` because nothing was ever written down that could have been refuted;
- what the agent may and may not touch in each mode — the boundary between researching a strategy, changing a platform artifact, paper trading and live execution is enforced by envelopes (AGENT-006 §2.6) but never *stated* as a ring the user can see;
- how a run is measured as a research act — knowledge produced per dollar, not tokens per step.

This spec defines the two user-facing modes and the Continuous toggle on Research, the seven workflows inside Research, the settings orthogonal to both, the four rings of what may be modified, the ten-stage lifecycle every trial-spending run passes through, the gates between stages, and the *yield* every run reports. AGENT-014 specifies continuous research in full. ADR-0037 records the decision.

## 1. Principles

- **P1 — A mode fixes what the agent may spend and what it may write.** Chat spends no trial and writes only its workspace and preferences; Research spends pre-registered trials and proposes into rings 1 and 2. Whether Research is handed a question or a charter is the Continuous toggle — a qualifier that changes the input and the output, never the rings. Anything else that varies is a workflow or a setting.
- **P2 — Every trial-spending run is pre-registered.** The evaluation protocol — objective, benchmark, `delta_practical`, window, embargo, cost model, null family, trial budget — is sealed *before* the first trial. The platform already does this per Experiment (`prereg_hash`); this spec does it per run and includes the falsifier.
- **P3 — A hypothesis names its falsifier or it is not a hypothesis.** "Momentum works on ETH" cannot be refuted; "12-1 momentum on ETH 1d has net Sharpe ≥ 0.5 over 2022–2025 after measured costs, else refuted" can. The verdict is computed by the platform against the falsifier, never declared by the model.
- **P4 — A refinement answers a diagnosis.** Changing a strategy because it failed is search; changing it because *this* is why it failed, and predicting what the change will do, is research. The platform records the chain and stops it when it stops paying.
- **P5 — Every run yields knowledge, including the ones that fail.** Findings, negative results with a failure class, questions opened and closed, contradictions resolved. A run that spent budget and produced none of these is a defect, reported as one.
- **P6 — Rings are enforced below the model.** The four rings (research · platform artifacts · paper · live) are envelopes, scopes and absent tools, not prose. The model is told which ring it is in so that it can say so; it cannot leave it by being clever.
- **P7 — The user can always see the stage, the ring, and what would change the verdict.** Understanding is two-way (AGENT-012 P12): the run shows its pre-registration, its falsifier and its current stage; the user can steer any of them, and a steer that changes the protocol after sealing creates a new registration with lineage, never an edit.

## 2. Modes

### 2.1 Two modes, and the toggle

The user chooses between **Chat** and **Research**. Research carries a **Continuous** toggle. (Revised 2026-09-15: an earlier draft made continuous research a third mode; the user chose two modes with a toggle, and ADR-0037 §2 records why the design still holds.)

| | **Chat** | **Research** (continuous off) | **Research** (continuous on) |
|---|---|---|---|
| What the user hands over | a question | a question | a **charter** (AGENT-014 §2.2) |
| Unit of work | a turn | a run, in one conversation | an agenda executed as many runs; the conversation is the journal |
| Rings writable (§4) | ring 0 workspace; `user_preference` memory | ring 0; proposals into ring 1 and ring 2 | **identical to continuous off** — the toggle changes nothing about what may be spent or touched |
| Trials | **never** — no `TrialTicket` is issued on a Chat turn | yes, pre-registered (§6) | via its child runs, each pre-registered |
| `GoalKind` (AGENT-008 §3.1) | `Lookup` only; a `Research`-kind goal is answered with an *offer* (§2.2) | any | selected per agenda item |
| Completion | reply sent | verdict in a dossier (§5.9) or the budget's report turn | AGENT-014 §7: budget · schedule · agenda drained · stall · human stop |
| Supervision | reading; `/why` on any card | controls row (AGENT-012 §3.4); State panel | the same controls on any live child, plus the Agenda and Charter tabs and the digest |
| Memory writes | `user_preference` | all kinds, by tools (AGENT-007 P1) | all kinds, plus `open_question` (§9) |
| Tier | the profile the user picked | same | **frontier profiles only** (AGENT-014 §8) |

**What a mode fixes, and what the toggle fixes.** A mode fixes what the agent may *spend* (trials) and *write* (rings): Chat and Research differ on both. The toggle fixes what the user *hands over* (question or charter) and therefore what comes back (one verdict or a stream of them) — it is a qualifier on Research, and every setting it enables (schedule, autonomy, replenish, review cadence, concurrency) exists only when it is on. Internally both are one object, `ResearchSession { continuous }` (AGENT-014 §2.1); a bounded run is the one-item case. That is what keeps the merge honest: the continuous machinery is not hidden inside a run, and the user is not asked to learn a third word.

### 2.2 Chat

Chat is the front door and the cheapest thing the platform does. A turn runs on the same loop (AGENT-010 P1) with `GoalKind::Lookup` forced, a plan of at most three read-only tools, `report.*` tools hidden, and no trial-spending tool exposed — the exposure slice is filtered by ring before routing, so a Chat turn *cannot name* `run_study`. It may call `memory.search`, `analysis.*` templates on already-materialised data (they spend no trial), `get_self_status`, and every `discovery`/`data`/`backtest` read.

**The ceiling is an offer, not a refusal.** When the classifier (AGENT-008 §3.1) rates a Chat goal as `Research`, `Diagnose` or `Compare`, or when a turn would need a trial, the turn ends with a **Research offer card** (AGENT-012 §3.2, new card type): the question as the agent understood it, the workflow it would use, the estimated cost and trials, the pre-registration it would seal, and one button that starts the run. The user never discovers the ceiling by watching a turn fail. A `user_preference` may set "always offer" to "always start" for a scope.

Chat turns carry the conversation's episodic memory (AGENT-007 §2.1) so a Chat after a Research run can interrogate its dossier; they are not archived separately.

### 2.3 Research, continuous off

One question, one run, one verdict. The run passes through the lifecycle (§5) under one of seven workflows (§3), with the gates in §6 enforced by the platform. Everything AGENT-008 says about a step and AGENT-010 says about a run applies unchanged; this spec adds the stage machine around them and the objects it produces (§7).

### 2.4 Research, continuous on

The question field becomes a charter; the run becomes an agenda of runs; the session acquires a budget over days, a schedule, a stall detector, a digest and an autonomy level. **Endless is `replenish: on`** — it regenerates the agenda when it drains. AGENT-014 is the spec. The rings, the lifecycle and the gates are exactly those of continuous off, applied to every child run.

### 2.5 Evaluated and rejected as modes

| Candidate | Why it is not a mode | What it is |
|---|---|---|
| Diagnose | same rings and completion as Research; differs in required steps | workflow `diagnose` |
| Audit / Falsify | same | workflow `falsify` |
| Replicate | same | workflow `replicate` |
| Compare | same | workflow `compare` |
| Survey (no experiments) | same; ring 0 only | workflow `survey`; also the replenish step of a continuous session |
| Optimise (drive a campaign) | same; the campaign is a platform object the run suspends on | workflow `optimise` |
| Build / Author | differs in rings only; "the agent edits the registry directly" is exactly the thing the `Promotion` envelope exists to prevent | the SLv2 editor (AGENT-011) plus proposals into ring 1 |
| Operate / Monitor | the monitor is an independent process by design (COMP-007 P1); an agent that "operates" the book is the failure mode | COMP-007; continuous sessions *subscribe* to its events as agenda sources |
| Watch (observation-driven) | a source of questions, not a way of answering them | an AGENT-014 §4 agenda source |
| Program / Endless as a third mode | the user chose two modes with a toggle (2026-09-15); the object model keeps the continuous machinery distinct so the toggle is a qualifier, not a disguise | Research with Continuous on |

## 3. Workflows (inside Research)

A workflow is a **stage skeleton**: which lifecycle stages (§5) are required, which may be skipped, which gates (§6) apply, and what the verdict enum is. The agent picks one at intake from the question's shape; the user may override with a chip in the composer (AGENT-012 §3.3). A workflow is not a tool — the AGENT-006 §6 composed tools (`leakage_audit`, `precapital_battery`, …) are *called by* stages.

| Workflow | Question shape | Required stages | Skippable | Verdict |
|---|---|---|---|---|
| `investigate` (default) | "does X have an edge / hold / matter?" | all ten | none | `supported \| refuted \| inconclusive \| blocked` |
| `falsify` | "break this strategy / finding" | intake · ground · **preregister** (attack protocol) · execute (`precapital_battery`, `leakage_audit`, `red_team_falsify`) · evaluate · diagnose · report | hypothesise (the hypothesis is the artifact under attack) · design · refine | `survives \| fragile \| refuted` with the killing ticket |
| `replicate` | "reproduce this paper / finding / dossier" | intake · ground · preregister (the *original* protocol, sealed as-stated) · design · execute · evaluate · report | refine (a replication that refines is no longer a replication) | `replicated \| partially \| failed \| not_reproducible` with effect-size comparison |
| `compare` | "which of these N is better, under one protocol?" | intake · ground · preregister (one protocol for all) · execute · **evaluate via `compare_candidates`** (§8.3) · report | hypothesise · design · refine | a ranking with `P(A≻B)` and CI per pair, or `indistinguishable` |
| `diagnose` | "why did this fail / decay / disagree?" | intake · ground · hypothesise (causes) · execute (attribution, forensics — read-mostly, trials only for ablations) · evaluate · report | preregister *may* be a diagnosis protocol rather than an edge protocol · refine · design | `cause: <class>` with evidence, or `undetermined` naming what was ruled out |
| `survey` | "what is known / what should we ask next?" | intake · ground · report | everything that spends a trial — **`survey` cannot issue a `TrialTicket`** | a `SurveyReport`: findings, negatives, contradictions, open questions, proposed questions with value estimates |
| `optimise` | "search this family's space under this objective" | intake · ground · **preregister = campaign DEFINE** (SPEC §10) · execute (suspend on `JobKind::Campaign`; HYPOTHESIZE dispatches a worker sub-run, §8.5) · evaluate (the campaign's COMPARE/GATE) · report | hypothesise/design at run level (the campaign owns them) · refine (the campaign's REALLOCATE) | the campaign's terminal state and incumbent |

Two rules that hold across all seven:

- **A workflow cannot loosen a gate.** Skipping a stage is allowed only where the table says; skipping a gate never is. `falsify` skips *hypothesise* but not pre-registration — an attack protocol chosen after seeing which attack works is p-hacking with extra steps.
- **The workflow is recorded on the run** (`agent_runs.workflow`) and on every trial it spends, so the trial ledger can answer "how many of our trials are replications" and the scorecard can answer "which workflows does the 7B actually complete" (AGENT-009 §3, the `judgement` suite).

## 4. The four rings

What the agent may modify, in any mode, is one of four rings. Each ring is enforced by a mechanism below the model (P6), and the ring the run is in is rendered in the working state and the desk header.

| Ring | Contains | Agent may | Enforced by |
|---|---|---|---|
| **0 · Research** | the conversation workspace; strategy *drafts*; experiments, sweeps, studies, trials; all memory kinds; dossiers | create, run, record, spend budget — freely within the run's budget and pre-registration | budgets (AGENT-010 §6); trial ledger (INV-16); pre-registration gate (§6.1) |
| **1 · Platform artifacts** | the strategy registry (promoted versions), feature definitions, model versions, playbooks, datasets, the campaign registry, control limits | **propose**: create a candidate, attach the dossier, request promotion | `Promotion` envelope on `promote_model_version`, `allocate_book{apply}`, the new `strategy.propose_promotion` (§8.6); playbook admission (AGENT-007 §5); `PUT /api/monitor/limits` has no agent scope (COMP-007 §6) |
| **2 · Paper** | shadow sessions, paper deployments, the paper book | **propose**, and read execution quality; `run_shadow_session` under `Spend` | `Spend` envelope; the paper gate needs `go_live_readiness ≠ no_go` (AGENT-006 §6); deployment is a human action on the desk, never a tool |
| **3 · Live** | orders, the live book, the kill switch, automation arming | **read** monitor state (`portfolio.check_control_limits`) | no tool exists (`place_order` absent, AGENT-010 §11); no agent token holds a monitor POST scope (COMP-007 §5); ADR-0025; D-12 |

Three consequences worth stating:

1. **A mode is a ring filter on exposure.** Chat's slice is ring-0 reads plus preference writes; Research's slice — continuous on or off — is ring 0 plus the proposal tools; nothing in any slice reaches ring 3. The filter is applied before routing (AGENT-005 §5), so a tool outside the ring is unnameable, not merely refused.
2. **Proposals are first-class objects, not chat.** A `Proposal { kind: Promote | Deploy paper | Adopt playbook | Change limits, dossier_ref, evidence_ids, what_changes, reversibility, cost }` is written to the run, rendered as an approval card with its envelope (AGENT-012 §3.2), and lives in the desk's *Needs you* panel until decided. A decided proposal is a `decision` row (SPEC §4.4) with the human as actor.
3. **The kill switch reaches ring 0.** While the switch is tripped, no proposal into ring 2 can be *created* (not merely not approved), and every continuous session pauses (AGENT-014 §7). An agent proposing paper deployments while the book is halted is proposing into a state it does not understand.

## 5. The lifecycle

Ten stages. Every trial-spending run passes through them in order under `investigate`; other workflows skip what §3 permits. The stage is a field on the working state (`WorkingState.stage`, AGENT-008 §2, amended by this spec) and a card in the desk. Stage transitions are effects in the log (AGENT-010 §5), so a replay shows exactly when the run moved and on what evidence.

```
INTAKE ─► GROUND ─► PREREGISTER ─► HYPOTHESISE ─► DESIGN ─► EXECUTE ─► EVALUATE
                                        ▲                                  │
                                        │                                  ▼
                                        └────── REFINE ◄──── DIAGNOSE ◄────┘
                                                   │              │
                                                   ▼              ▼
                                                REPORT ◄──────────┘  ─► HAND-OFF
```

### 5.1 Intake

The question becomes a `ResearchQuestion` (§7.1): subject (instrument / family / strategy / component), the question in one sentence, scope constraints (from the composer chips, AGENT-005 §3.2), *what would change your mind* (the user's falsifier, if they gave one), and the workflow. The classifier runs (AGENT-008 §3.1). **`hypothesis_precheck`** (AGENT-006 §6) runs against memory and the ledger before anything else: `reject_duplicate` ends the run at intake with the prior verdict and its ticket — the cheapest run the platform does, and the one that most often saves a budget.

### 5.2 Ground

Assemble what is already known before deciding how to test: the task brief (AGENT-007 §4), the instrument profile, data coverage for the window (`data.*` probes), open questions in scope (§9), contradictions in scope (AGENT-007 §2.2.1), and — on capable tiers — a short survey of the signal registry. Ground spends no trial. Its output is the **evidence bundle** the pre-registration cites, and the first place the run can end: `blocked` when coverage is insufficient for any admissible protocol (minimum track record, AGENT-006 §5.4 `min_track_record`, computed *here*, before a trial is spent).

### 5.3 Preregister

The run seals a `Preregistration` (§7.2) *before* the first trial: objective (multi-objective, SPEC §12.1), benchmark, `delta_practical` (no default — SPEC §10), data window and embargo (computed, SPEC §12.2), cost model version, null family, trial budget for this run, gate profile, and **the falsifier for the primary hypothesis**. The hash is written to the ledger; every trial the run spends carries it (this extends the per-Experiment `prereg_hash` to the run and adds the falsifier to it). The desk renders it as a card the user can read *before* the run spends, and steer.

**Amending a sealed registration is a new registration** with `supersedes` set. The run may do this — protocols do turn out to be wrong — but the dossier shows both, the trial count does not reset, and a run that amends more than `max_prereg_amendments` (default 2) is reported as `blocked: protocol unstable`. This is what stops "choose the evaluation after seeing the result" without forbidding learning.

### 5.4 Hypothesise

Typed `Hypothesis` records (§7.3) — family, statement, proposed mechanism, prediction, **falsifier** (P3), status. A hypothesis whose falsifier the platform cannot evaluate (names no measurable quantity, or names one no tool returns) is rejected at write with the correction naming what is missing. Dedup runs per hypothesis (`memory.check_hypothesis`). The primary hypothesis is the one the pre-registration sealed; secondary hypotheses may be added during the run but are marked `exploratory` and their results cannot be reported `established` — they can only open a question (§9) or seed the next run.

### 5.5 Design

The experiment plan: strategy drafts in SLv2 (AGENT-011) with `preview_signals` before any trial, feature choices from the registry, the split spec, the null library choice, and a **power check**: given the window and the pre-registered `delta_practical`, is the minimum track record satisfied? If not, the design is refused *here* — not discovered at evaluate after the trials are spent. `simplify()` (FEAT-003 §8.3) runs on every draft before execute: complexity is overfitting's friend, and it is cheaper to remove a rule before it is measured than after.

### 5.6 Execute

Experiments, sweeps, studies through the job service (AGENT-010 §8); every trial registered before it runs (D-S7). The run is suspended for most of this stage and costs nothing. Progress reaches the desk as job cards. The exploration floor (INV-21) applies inside any campaign; at run level the analogue is that `execute` may not skip the null comparison the pre-registration named.

### 5.7 Evaluate

The verdict is **computed**: the gate funnel with its tickets (AGENT-006 §6 — `leakage_audit`, `selection_bias_verdict`, `precapital_battery`), out-of-sample performance against the pre-registered benchmark after the pre-registered costs, and the falsifier evaluated against the sealed criterion. The platform returns one of `supported | refuted | inconclusive` per hypothesis with the numbers that decided it; the model cannot set this field. `inconclusive` must name *why*: `insufficient_data | protocol_unstable | evidence_conflict | budget_exhausted` — an unnamed inconclusive is a reporting defect.

### 5.8 Diagnose (on anything but `supported`)

The `DiagnosticBundle` (FEAT-003 §11.1) and the attribution tools answer *why*: which gate refused, which rule the ablation blamed, which regime, which cost. The failure is classified into the closed enum (AGENT-007 §3.2: `weak_effect | null_indistinguishable | capacity_insufficient | execution_infeasible | data_quality`, extended by this spec with `leakage | protocol_unstable | fragile_parameters | regime_specific`) and a `negative_result` is derived — by the platform, not declared by the model. Diagnose is also where a *contradiction* with memory is detected and recorded (AGENT-007 §2.2.1).

### 5.9 Refine, or stop

A `Refinement` (§7.4) must cite a diagnosis and a predicted effect on the pre-registered objective (P4); the platform refuses one that does neither ("state which diagnosed cause this addresses and what you expect the objective to do"). Refinements form a chain from the original design. The chain stops when any of these holds, and the run moves to report:

- `max_refinements_per_run` (default 3; 2 on local tiers) reached;
- **diminishing returns**: the posterior probability that one more refinement exceeds the incumbent by `delta_practical` falls below 0.05 over the chain (SPEC §10's rule, at run scale) — with fewer than three refinements this is `NotComputable` and the count rule governs;
- the same failure class twice in a row — the diagnosis is not being addressed;
- the refinement would amend the pre-registration a third time (§5.3).

Stopping here is not failure. It produces the negative result with the most valuable field in the store: `retry_condition`.

### 5.10 Report, then hand off

`report_finalization` (AGENT-006 §6): claims extracted and rechecked, `metric_integrity`, the fresh-session critic, the dossier with mandatory *unresolved questions* and *reviewer critique*. This spec adds to the dossier: the pre-registration(s) with their hashes, the falsifier and how it was evaluated, the refinement chain, the reproducibility receipt (§8.4), and the **yield** (§10). Then hand-off: `Proposal`s into rings 1 and 2 if the verdict supports them, `open_question`s into memory, and — when the run belongs to a continuous session — agenda proposals (AGENT-014 §4).

## 6. Gates between stages

The stage machine is enforced, not advisory. Each rule below names the mechanism that refuses, and the test that proves the violation is refused (MASTER §5 DoD 3).

| # | Rule | Refused by |
|---|---|---|
| 6.1 | No trial without a sealed `Preregistration` for this run | the trial-ticket path (`crates/ledger`, the same wrap `spends_trial` uses): a ticket request carrying no `prereg_id` for the run is refused with `preregistration_required` |
| 6.2 | No hypothesis without an evaluable falsifier | `research.hypothesise` write validation: the falsifier must name a quantity in the closed metric vocabulary (`mlops.trial_statistic`'s) and a comparator |
| 6.3 | No design past the power check | `research.design` refuses when `min_track_record` for the pre-registered `delta_practical` exceeds the window; the refusal carries the window that would suffice |
| 6.4 | No evaluate without the pre-registered null | the funnel (exists: Gate 2 requires tickets) plus: the null family in the ticket must equal the pre-registration's |
| 6.5 | No refinement without a diagnosis and a prediction | `research.refine` validation; the correction names the two missing fields |
| 6.6 | No verdict set by the model | `WorkingState.hypothesis.status` is written only by `evaluate`'s platform code; the model's `record_finding` may cite it, never set it |
| 6.7 | No `established` finding from an `exploratory` hypothesis | AGENT-007 §3.1 step 2 extended: evidence traced to an exploratory hypothesis caps confidence at `suggestive` |
| 6.8 | No ring-2 proposal without `precapital_battery ≠ reject` and `go_live_readiness ≠ no_go`; none at all while the kill switch is tripped | `Proposal` creation checks the tickets and the monitor state; the refusal is rendered |
| 6.9 | No `supported` verdict without out-of-sample, costs and the benchmark in the evidence | `evaluate` requires the three ticket kinds; a dossier missing any renders the verdict as `inconclusive: evidence_incomplete` |
| 6.10 | Chat issues no trial ticket | ring filter on exposure plus the ticket path refusing a run whose `mode = chat` — belt and braces, because the exposure filter is a routing property and the ticket path is a ledger property |

## 7. Objects

All are rows the platform writes; the model proposes through tools and the harness validates (AGENT-007 P1). All carry `run_id`, `conversation_id`, `program_id?`, `created_at`, and are addressable ids the desk can open (AGENT-012 P9).

### 7.1 `ResearchQuestion`

```
{ id, subject: {kind: instrument|family|strategy|component|market, ref},
  question: ≤ 300 chars, workflow, constraints: {...from the opening message},
  change_my_mind: ≤ 300 chars | null, source: user|agenda|proposal|observation,
  precheck: {verdict: proceed_new|proceed_as_variant|reject_duplicate, refs[]},
  dedupe_key: hash(normalised question, subject) }
```

### 7.2 `Preregistration`

```
{ id, run_id, seq, supersedes?: id,
  objective: {maximize[], subject_to{}},        # SPEC §10 shape; no scalarisation by default
  benchmark: {kind, ref},                        # buy-and-hold, a registry strategy, a factor
  delta_practical: f64,                          # REQUIRED, no default
  window: {start, end}, embargo: computed, cost_model: {version, venue},
  null_family: [...], trial_budget: u32, gate_profile: "strict_v1",
  primary_hypothesis_id, falsifier: {metric, comparator, threshold, window},
  sealed_at, hash, ledger_ref }
```

`hash` is over every field but `sealed_at`; the ledger row is the attestation that it predates the trials (the same argument as ADR-P2-33).

### 7.3 `Hypothesis`

```
{ id, family_id, statement ≤ 300, mechanism ≤ 300, prediction ≤ 200,
  falsifier: {metric, comparator, threshold, window},
  role: primary|secondary|exploratory,
  status: untested|supported|refuted|inconclusive{why},   # platform-written
  evidence_ids{}, dedupe: {similar[], verdict} }
```

### 7.4 `Refinement`

```
{ id, parent_design_id, diagnosis_id, cause_class, change: Move | SLv2 diff,
  predicted_effect: {metric, direction, magnitude?}, observed_effect?: {...},
  seq_in_chain }
```

### 7.5 `Proposal`

```
{ id, kind: promote_strategy|deploy_paper|adopt_playbook|change_limits|start_continuous_research,
  ring: 1|2, dossier_ref, evidence_ids{}, what_changes ≤ 300, reversibility, cost_estimate,
  envelope: Promotion|Spend|Tier3Memory, status: pending|approved|denied|withdrawn,
  decided_by?, decision_ref? }
```

### 7.6 `Verdict`

```
{ run_id, workflow, primary: supported|refuted|inconclusive{why}|blocked{why},
  per_hypothesis[], decided_by: {gate tickets[], falsifier evaluation, oos, costs, benchmark},
  yield: §10, receipt: §8.4 }
```

## 8. Tools and mechanisms this spec adds

Registered under AGENT-006 §3's one path; each carries a `ToolSpec` with the fields §2.3 requires. The `research` namespace is at 14 of 16; the six new tools push it over, so this spec **splits it**: `research` keeps experiments, sweeps, studies, funnel, nulls, gates; a new namespace **`lifecycle`** holds the stage tools below (AGENT-006 §2.1 amended: twenty-two namespaces).

| Tool | Tier | Signature | Envelope |
|---|---|---|---|
| `lifecycle.preregister` | T | `(objective, benchmark, delta_practical, window, cost_model, null_family, trial_budget, falsifier) -> {prereg_id, hash, computed: {embargo, min_track_record}}` — refuses without `delta_practical` | — |
| `lifecycle.hypothesise` | T | `(statement, mechanism, prediction, falsifier, role) -> {hypothesis_id, dedupe}` — §6.2 | — |
| `lifecycle.design` | T | `(hypothesis_id, strategy_version, split_spec, null_choice) -> {design_id, power: ok\|insufficient{window_needed}}` — §6.3; runs `simplify` | — |
| `lifecycle.refine` | T | `(design_id, diagnosis_id, change, predicted_effect) -> {refinement_id, chain: {n, diminishing: bool\|not_computable}}` — §6.5 | — |
| `lifecycle.compare_candidates` | T | `(candidate_ids[], protocol: prereg_id) -> {matrix[{a, b, p_a_gt_b, ci_lower, verdict}], ranking\|indistinguishable}` — §8.3 | — |
| `lifecycle.reproduce` | T | `(experiment_id) -> {bit_identical: bool, receipt, diff?}` — §8.4; re-runs under the receipt; spends **no** trial (a re-run of a registered trial is not a new look, ADR-P2-17's argument) | — |
| `lifecycle.propose` | T | `(kind, dossier_ref, what_changes, reversibility) -> {proposal_id}` — §7.5; §6.8 | `Promotion` (kind = promote_strategy, change_limits) · `Spend` (deploy_paper) · none for adopt_playbook (admission is its own path) |
| `lifecycle.open_question` | T | `(question, why_it_matters, blocking: bool, value: low\|med\|high) -> {question_id}` — §9 | — |
| `lifecycle.get_stage` | H | `() -> {stage, workflow, ring, prereg_id, hypotheses[], chain, what_would_change_the_verdict}` — the model's view of §5; rendered to the desk from the same struct | — |

### 8.1 The verdict is a platform function

`evaluate` is not a tool the model calls; it runs when the funnel returns and when the falsifier's metric is available in `mlops.trial_statistic`, and it writes `Hypothesis.status`. The model *sees* the verdict in the working state and the desk sees the same struct. This is the mechanism behind P3 and §6.6.

### 8.2 Failure classes, extended

AGENT-007 §3.2's enum gains `leakage`, `protocol_unstable`, `fragile_parameters`, `regime_specific`. Each is derived from a specific ticket: `leakage_audit = leaking`, §5.3's amendment cap, `parameter_fragility_map = knife-edge`, `regime_failure_search = current regime is worst`. The model still cannot create one.

### 8.3 `compare_candidates` is SPEC §11.5 as a tool

The comparison protocol exists on paper and partly in code (`ComparisonPlan::judge`, HANDOFF §2). This tool is the one door to it: all nuisance sources randomised with **platform-held seeds** (ADR-P2-18 — the agent cannot pick the seed), ~29 paired replicates, `A ≻ B` iff `P(A>B) ≥ 0.75 and P(A>B) − CI_lower > 0.5`, Holm across pairs, anytime-valid confidence sequences with a ROPE of ±`delta_practical` because the agent will peek. Output is a Multiple Comparison Matrix; the desk never renders a critical-difference diagram (SPEC §11.5). The `compare` workflow cannot rank without it (§3).

### 8.4 The reproducibility receipt

Every experiment result and every dossier carries a `ReproSpec`: dataset hash (SPEC §3.1 — a dataset is a hash), feature registry version, calendar package version, engine version, strategy version hash, cost model version, seeds (the platform's, not the agent's), harness version. `lifecycle.reproduce` re-runs under the receipt and reports bit-equality. A nightly job reproduces a 2% sample of the last 30 days' experiments and publishes `repro_drift` on the governance page; a non-zero drift is a platform defect (AGENT-007 §2.2 `platform_defect`) filed automatically, because "the same backtest gave different numbers six months later" (SPEC §2) is the failure that makes every other number a rumour. A dossier without a receipt cannot carry a `supported` verdict (§6.9 extended).

### 8.5 `optimise` and the campaign that already exists

`mlops.campaign` is built: DEFINE, the event fold, the job driver, idempotent child jobs, the platform seed, the exploration floor, sixteen gates (IMPLEMENTATION-CHECKLIST 2.1, 2.14). What is missing is the `ResearchRun` child worker (HANDOFF §2) — the thing that proposes at HYPOTHESIZE. This spec supplies it: the parent Research run's pre-registration **is** the campaign DEFINE (same fields, one object, one hash); the run calls `define_campaign` and emits `Effect::Await { job_id }`; at HYPOTHESIZE the campaign driver dispatches a **worker sub-run** (AGENT-010 §9) of the parent, with a *projection* of the parent's context store (AGENT-005 §29: findings, the diagnostic bundle, the parent candidate and cousins per FEAT-003 §8.2 — never the parent's reasoning), whose only output is one `Move` with a hypothesis and parameter ranges; the driver logs it as a `propose` decision with its candidate set and continues. The parent resumes at the campaign's terminal event. The platform owns the phases, budget, floor and gates; the agent owns what to try — exactly the PHASE-2-5-PLAN §2.1 reconciliation, now with a runtime to do it on.

### 8.6 `strategy.propose_promotion` replaces direct promotion

Any tool that would move a strategy into the registry's promoted set from a run is replaced by a `Proposal` of kind `promote_strategy`. The registry write happens when a human approves, as that human, and is a `decision` row. Ring 1 has no other door from an agent token; a test asserts that no tool with `Risk::Write` targets a registry table outside the proposal path.

## 9. Open questions are memory

AGENT-007 §2.1 carries `open_questions` as strings in a turn summary, where they die with the conversation. This spec makes them a semantic memory kind (AGENT-007 §2.2 amended): `open_question { question, why_it_matters, subject, opened_by: run_id, blocking: bool, value: low|med|high, status: open|closed{by: run_id, how: answered|superseded|withdrawn}, related_finding_ids[] }`. They enter the brief (AGENT-007 §4, new slot: up to 10% of `Reference`, in-scope open questions, `blocking` first). They are the second-largest agenda source for a continuous session (AGENT-014 §4), and closing one is a yield event (§10). The desk's memory browser gets an *Open questions* tab (AGENT-012 §7).

## 10. Yield — what a run produced, as a number

Every run ends with a `Yield` row, computed by the platform from the effect log and memory writes:

```
{ run_id, findings_new: {established, suggestive}, negatives_new, negatives_new_class: bool,
  questions_opened, questions_closed, contradictions_resolved, proposals: {made, approved},
  hypotheses: {tested, refuted, supported}, duplicates_avoided_at_intake: bool,
  cost: {usd, tokens, trials, wall_s}, cost_per_knowledge_unit }
```

`knowledge_unit` = an established finding, a negative result with a *new* failure class for the subject, a closed question, or a resolved contradiction — the things that make the *next* run cheaper. A run with `knowledge_units = 0` and `trials > 0` is a **barren run**, reported in the scorecard per tier and per workflow (AGENT-009 §4 amended), surfaced in the desk's usage page as cost per knowledge unit, and read by a continuous session's stall detector (AGENT-014 §7). This is the measurable form of the North Star's "research should produce reusable knowledge even when a strategy fails".

## 11. Tier behaviour

Everything above runs on every tier through the same loop. What differs:

| | frontier / local_high | local_mid | local_small / degraded |
|---|---|---|---|
| Workflows available | all seven | all but `optimise` (the worker sub-run needs a plan of moves) | `survey`, `investigate` with ≤ 1 refinement, `replicate` |
| Pre-registration | model drafts, platform validates | model fills a **skeleton** under grammar (the fields are enums and numbers) | skeleton from the question's constraints; model confirms |
| Falsifier authoring | free text → validated | grammar over `(metric, comparator, threshold)` | grammar; one metric offered |
| Refinement chain | ≤ 3 | ≤ 2 | ≤ 1 |
| Critic at report | fresh-session model | checklist | checklist |

Each of these is a profile value with a conformance reader (`tests/conformance.rs`), and the scorecard's `judgement` suite measures whether the tier completes the workflows its profile claims.

## 12. Build order

| Phase | Items |
|---|---|
| 1 | `WorkingState.stage`, `mode`, `workflow`, `ring` on runs; the ring filter on exposure; Chat as `Lookup`-forced with the Research offer card; the Continuous toggle on the run's session row (AGENT-014 §2.1); `Preregistration` with the ledger gate (§6.1) |
| 2 | `Hypothesis` with falsifier validation; platform-written verdict (§8.1); extended failure classes; `open_question` memory kind and brief slot |
| 3 | `Design` with the power check; `Refinement` with the chain rules; `Yield` |
| 4 | `Proposal` and the ring-1/2 doors; `strategy.propose_promotion`; kill-switch coupling |
| 5 | `compare_candidates`; `reproduce` and the nightly drift job; `optimise` worker sub-run and the `ResearchRun` child worker |
| 6 | the desk: stage card, pre-registration card, falsifier on hypothesis cards, proposals in *Needs you*, yield in usage (AGENT-012 amendments) |

## 13. Acceptance

1. A Chat turn cannot obtain a `TrialTicket`: the exposure slice contains no trial-spending tool, and a forged request through the ticket path with `mode = chat` is refused.
2. A Chat goal classified `Research` ends with a Research offer card whose one action starts a run with the same question; no trial was spent by the turn.
3. `run_study` (and every `spends_trial` tool) is refused on a run with no sealed `Preregistration`; the refusal names `preregistration_required`.
4. A `Preregistration` without `delta_practical` is refused; amending a sealed one creates a new row with `supersedes` set, the trial count carries over, and a third amendment ends the run `blocked: protocol_unstable`.
5. A hypothesis whose falsifier names no evaluable metric is refused with the missing part named; a run with a sealed falsifier has its `status` written by the platform, and a `record_finding` attempting to set it is corrected.
6. A design whose window is below `min_track_record` for the pre-registered `delta_practical` is refused *before* any trial, with the sufficient window stated.
7. A refinement without a `diagnosis_id` or a `predicted_effect` is refused; the same failure class twice in a row ends the chain; the chain never exceeds the profile's cap.
8. A verdict `supported` cannot be produced without out-of-sample, cost and benchmark tickets and a reproducibility receipt; removing any one renders `inconclusive: evidence_incomplete`.
9. `lifecycle.reproduce` on a recorded experiment reports bit-identical; after a deliberate calendar-version bump it reports the diff; the nightly job files a `platform_defect` on drift.
10. No agent tool writes to the promoted-strategy set; `lifecycle.propose{promote_strategy}` creates a `Promotion`-enveloped proposal and the registry changes only when a human approves; a static test proves no other write path exists.
11. With the kill switch tripped, `lifecycle.propose{deploy_paper}` is refused and the refusal is rendered; after reset it succeeds.
12. `compare_candidates` uses platform-held seeds (the agent cannot supply one — the field does not exist), Holm across pairs, and returns `indistinguishable` on two copies of the same strategy.
13. The `optimise` workflow drives an `mlops.campaign` through HYPOTHESIZE via a worker sub-run whose context provably excludes the parent's reasoning; AT-59 (resume is a pure fold) still passes with the agent in the loop.
14. Every finished run has a `Yield` row; a run with trials and zero knowledge units is reported barren in the scorecard; `open_question`s appear in the next run's brief for the same subject.
15. On `qwen2.5-7b`, `investigate` completes to a report with a grammar-filled pre-registration and ≤ 1 refinement; `optimise` is not exposed on that profile and the conformance test reads the field that hides it.

## 14. Non-goals

- Automatic promotion or deployment under any autonomy setting. Rings 1 and 2 are proposals; ring 3 is unreachable.
- A general-purpose workflow engine. Seven skeletons, each a table row, each with a reader; a new workflow is a spec amendment.
- Bayesian model averaging across hypotheses or learned priors over families — measure the simple thing first (AGENT-009's rule).

## 15. Open questions

- Whether `delta_practical` should have a per-family *suggested* value shown in the pre-registration card (still no default in the object). Ship without; measure how often the user changes it.
- Whether `survey` should be allowed a small trial budget for power checks on proposed questions. Ship without: a survey that spends trials is an investigation.
- Whether `exploratory` hypotheses should count toward `N_eff`. They spend trials, so yes by INV-16; recorded here because it makes exploration visibly expensive, which is the intent.
