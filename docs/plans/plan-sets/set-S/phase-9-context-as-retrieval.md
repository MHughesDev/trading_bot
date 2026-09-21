# Phase 9 — Context as retrieval

**Completion: 85% (17 / 20 tasks), plus 3 `PARTIAL` — S-9.5's fixture, S-9.10's
corpus and S-9.12's promotion are built and have nothing live to run against.**

> Updated 2026-09-21. S-9.19 closed: the counterfactual ablation, the `indeterminate`
> verdict and the per-record clause in the prompt were all built, and the clause was
> being **discarded at the timeline boundary** - so the one question the scrubber
> exists to answer was unanswerable about precisely the records it is asked about.
> The remaining three are one block, not three: **gate H-2**. Each is built and has no
> live corpus to run against, and no amount of code closes that.

> Status 2026-09-16. The phase is built and the control is intact: `ContextPolicy::v1`
> is Part I exactly — `SelectionMode::Chronological`, `DegradationMode::PartOne`, the
> profile's own window, no manifest, no pull tools, no spill — so every existing run is
> unchanged and `v1` is a control that can be *asserted* rather than argued.
>
> `crates/harness/src/context/` is now `store`, `body`, `manifest`, `degrade`, `select`,
> `render`, `policy`, `budget`, `prefix`, `projection`, `utility`, `ablation`, `fit`.
> The write path reaches the loop: every `observe` site records a typed `Record`, under
> both policies, because the manifest, `expand`, attribution and the ablation all need
> the record to exist whichever path rendered the prompt.
>
> **What the three partials are waiting on is a corpus, not code.** Attribution,
> ablation and the weight fit all run offline over recorded tapes at zero model cost —
> and there are no recorded tapes, because there have been no live runs (roadmap gate
> H-2). `fit` returns the declared defaults and says so when the corpus is thin, which
> is the honest failure rather than a confident number over ten records.

**Requirement IDs:** CX2-01…CX2-20.
**Spec:** [AGENT-005 Part II §13–§30](../../../specs/AGENT-005-context-assembly.md), ADR-0036.
**Depends on:** Phase 1 (the budget arithmetic and `WorkingState` are the query), Phase 2
(the effect log is the store's persistence and the replay substrate), Phase 4 (the eval suite
is the only thing allowed to promote a policy).
**Blocks:** nothing. It is the last phase because it is the only one that must *prove* itself
against a baseline before it counts.
**Migration range:** 0089–0091.
**Parallel with:** Phase 8.

Everything else in Set S makes context assembly **correct**. This phase changes what it
**is**: from a chronological buffer that gets trimmed when full, to retrieval over a store,
rendered semantically, under a policy that is measured.

**Nothing here ships by being written.** `ContextPolicy v1` is Part I exactly — fixed layout,
fixed shares, recency ordering, byte truncation — and it is the control. `v2` is promoted only
by beating it on every tier under AGENT-009 §4's interval rules. A phase that redesigns the
prompt and cannot prove the redesign helps a 7B has not earned its place.

---

## The store and the manifest

### S-9.1 The run context store — L
`DONE` 2026-09-16 — `context/store.rs`, and every `observe` site in `drive.rs` writes one.

Every observation becomes an addressable typed `Record` (id, step, kind, source, provenance,
subject, ids, typed `Body`, tokens, utility) in a per-run store, rather than a line in a
buffer. Records are **never deleted during a run** — they leave the prompt, never the store,
which is what makes every later mechanism recoverable instead of lossy. The store is the same
object the effect log persists, so a replay reconstructs context exactly.

**Files:** `crates/harness/src/context/store.rs`, `crates/harness/src/drive.rs`.
**Acceptance:** every block site in `drive.rs` writes a `Record`; a replayed trajectory
reconstructs a byte-identical store; nothing is deleted mid-run.
**Traces:** AGENT-005 §14; §30.4.

### S-9.2 The manifest — M
`DONE` 2026-09-16 — `context/manifest.rs`; degrades by grouping to one line per kind and never truncates.

A small, **always-rendered, never-degraded** index: one line per record (id, step, kind,
source, subject, size, shown/elided), roughly 12 tokens each, degrading by grouping rather
than by dropping. This is the difference between an agent that *forgot* and one that *knows
where that is* — and it is the mitigation for the failure mode of any retrieval system, the
miss. It converts a silent absence into a visible, recoverable `expand`.

**Files:** `crates/harness/src/context/manifest.rs`.
**Acceptance:** every record in the store appears in the manifest; the manifest never degrades
below one line per group; an elided entry is `expand`able by the id shown.
**Traces:** AGENT-005 §17; §30.2.

### S-9.3 Semantic degradation — L
`DONE` 2026-09-16 — `context/body.rs` + `context/degrade.rs`; `truncate_middle` is not reachable from the record path.

`Body` becomes typed (`Table`, `Series`, `Stat`, `Text`, `Json`) and each declares a shrink
ladder: drop columns → sample rows → aggregate → schema and count; decimate → downsample →
min/max/mean/last. **Delete `truncate_middle` from the record path.** Cutting a 500-row table
at a byte offset yields broken JSON with no schema and no row count, when five rows plus the
column names plus "495 more" is smaller *and* more informative. Every degraded render carries
`degraded {from, to, how}` and its handle; a record that bottoms out and still does not fit is
dropped, not cut, and stays in the manifest.

`ToolResult` (AGENT-006 §2.4) is already the `Stat` ladder, so most of the catalogue gets this
for free.

**Files:** `crates/harness/src/context/{body,degrade}.rs`.
**Acceptance:** no record is ever byte-truncated; every shrink is a declared ladder step; a
degraded table still parses and still names its schema and row count.
**Traces:** AGENT-005 §16; §30.1.

---

## Selection

### S-9.4 Assembly becomes a query — L
`DONE` 2026-09-16 — `context/select.rs`; deterministic, no model call, no wall clock.

`select(store, working_state, budget, policy)`. The **query is the working state**: goal and
kind, active plan step and the tool it names, hypothesis, last error, ids already cited.
Scoring is structural and cheap — subject match, id overlap, kind prior for the goal kind,
shallow recency decay, in-run citation count, same-tool history, corrections for the active
tool — with the charter, goal, manifest and last error pinned.

Winners are **re-sorted into the stable layout** before rendering, so the model reads a stable
prefix while the harness chose by relevance. Under the trigger with everything fitting,
selection is a no-op and behaviour is identical to today.

**Files:** `crates/harness/src/context/select.rs`.
**Acceptance:** deterministic — same store, working state and policy version produce a
byte-identical prompt, asserted by replaying one trajectory twice; no model call and no
wall-clock in the path; under-budget runs are unchanged.
**Traces:** AGENT-005 §15; §30.4, §30.10.

### S-9.5 The case this phase exists for — M
`PARTIAL` 2026-09-16 — the forty-step fixture exists and both behaviours are asserted (`tests/context_is_retrieval.rs`). What is missing is the *corpus* half: the test is synthetic, and the headline claim deserves the same case drawn from a real trajectory. Waits on gate H-2.

A forty-step fixture where the observation that decides step 30 arrives at step 3. Under
recency ordering it is long gone; under selection it is rendered. Build the fixture, assert
both behaviours, and keep it as the phase's headline regression test.

**Acceptance:** `v1` omits the deciding record at step 30 and `v2` renders it; the test names
both policy versions so the difference is the assertion.
**Traces:** AGENT-005 §30.3.

### S-9.6 `expand` and `focus` — M
`DONE` 2026-09-16 — registered in `registry.rs`, answered in `drive.rs`, exposed only under a retrieval policy.

Two harness-owned core tools, no trial: `expand(record_id, level)` brings a degraded or elided
record back at higher fidelity for this step and the next two; `focus(subject)` biases
selection for the rest of the plan step, because the model saying "I am on ETH funding now" is
a better relevance signal than anything the harness can infer. Plus the automatic rule that
needs no tool: a record expanded twice is **promoted** for the rest of the run — the model has
now told us twice that the scoring was wrong.

**Acceptance:** an expansion that does not fit is refused with what it would have to drop, not
silently ignored; a twice-expanded record stays rendered without further calls; both tools
spend no trial.
**Traces:** AGENT-005 §18; §30.9.

---

## Budget and prefix

### S-9.7 Budget follows value — M
`DONE` 2026-09-16 — `context/budget.rs`; `RunBudget` bounds the run's total rather than hoping the multipliers average out.

`step_value` derived from the active plan step's tool (`spends_trial`, envelope,
`max_calls_per_run`) and the working state (a first attempt after a correction is worth more
than a repeat). Body budget scales `0.6×`–`1.4×` within the profile's hard input budget and the
25% floor. Total run spend is unchanged; its distribution is not. A step about to spend a trial
and a step listing instruments get identically sized bodies today, and that is simply wrong.

**Acceptance:** a trial-spending step receives a larger body than a lookup step on the same
profile; the hard budget and the floor are never crossed; total tokens across a fixed run do
not rise.
**Traces:** AGENT-005 §20.

### S-9.8 The composed charter — M
`DONE` 2026-09-16 — `prompt::charter_for`; four byte-stable prefixes, one per `GoalKind`.

The charter becomes fragments — identity, termination, tools, plus one per `GoalKind` —
assembled at run start. A lookup goal never pays for the edge-finding protocol, roughly 900 of
its 3,200 bytes. Deterministic, and byte-stable **per kind**: four stable prefixes instead of
one, so caching is preserved rather than sacrificed.

**Depends on:** S-1.16 (`GoalKind`), S-1.5 (the charter split).
**Acceptance:** a lookup charter omits the edge fragment; each per-kind charter is byte-stable
across calls and under `CHARTER_MAX_BYTES`.
**Traces:** AGENT-005 §21; §30.6.

### S-9.9 Cross-turn prefix continuity — M
`DONE` 2026-09-16 — `context/prefix.rs` plus `Loop::with_prior_exposure` and a ledger per conversation in `AgentManager`.

Every turn re-assembles from scratch today, so the provider cache is cold at the start of every
turn even though the charter and most of the tool slice are identical. Turn N+1's prefix is
built as a **superset** of turn N's: same charter fragment, same tool slice with additions
appended and never reordered, previous summary appended rather than inserted. Ten turns should
pay one cold prefix, not ten.

**Depends on:** S-1.8 (breakpoints), S-2.4 (append-only slice).
**Acceptance:** across a ten-turn conversation, cache-creation tokens are paid once; a turn
that adds a tool appends it rather than reordering the slice.
**Traces:** AGENT-005 §21; §30.7.

---

## Making it empirical

### S-9.10 Utility attribution — L
`PARTIAL` 2026-09-16 — `context/utility.rs` attributes a tape and aggregates per bucket, with §28.2's `indeterminate` kept apart from `unused`. **No corpus exists**: there have been no live runs, so `utility_rate` has nothing to report per tier. Waits on gate H-2.

After each call, attribute every rendered record against what the model did next: an id
appearing in the next tool call, a finding or `finish_task` evidence → **used**; the same tool
called again with derived arguments → **used**; an `expand` → **wanted, and we under-rendered
it**; rendered and never referenced → **unused**. Per run it is noise; aggregated per
`(GoalKind, RecordKind, signal bucket)` across the scorecard corpus it is the first real
evidence this platform has had about what belongs in a prompt.

`utility_rate` (used ÷ rendered tokens) joins the scorecard per tier beside cache hit.

**Depends on:** S-4.8 (trajectories), S-4.11 (scorecard).
**Acceptance:** attribution runs offline over recorded trajectories at zero model cost;
`utility_rate` appears per tier; the aggregation is reproducible from the corpus.
**Traces:** AGENT-005 §19; §30.5.

### S-9.11 `ContextPolicy` as a versioned object — M
`DONE` 2026-09-16 — `context/policy.rs`; `v1` is Part I exactly and is asserted to be.

Weights, kind priors, degradation order, manifest budget, step-value bounds and trigger become
one versioned object, recorded on every run and in every trajectory. `v1` is Part I exactly and
is the control. Because the loop is sans-IO, two policies can be A/B tested over the **same
recorded tapes at no model cost** — which is the payoff of that design and the reason context
tuning becomes an offline exercise rather than a live experiment.

**Files:** `crates/harness/src/context/policy.rs`, `config/profiles/*.yaml`.
**Acceptance:** with `context.policy = v1` the assembled prompt is **byte-identical** to
Part I's; the policy version is in every trajectory; an A/B over recorded tapes runs with no
provider call.
**Traces:** AGENT-005 §22; §30.10.

### S-9.12 Fit the weights, then earn the promotion — L
`PARTIAL` 2026-09-16 — `context/fit.rs` fits reproducibly and the promotion calls the existing eval gate, refusing a tier that lost utility and a tier with a baseline and no candidate. **The promotion has not been run**, because running it needs scorecards under both policies and those need live runs. The shipped weights are therefore still the declared ones, and `v2` is not promoted — which is the correct state, not a gap.

Fit the §15 selection weights to the attribution corpus rather than declaring them, then run
the promotion: `v2` ships only by beating `v1` on **every** tier under AGENT-009 §4's interval
rules, with `utility_rate` and cost reported beside pass rates. A policy that helps the
frontier and hurts the 7B does not ship (AGENT-009 P4) — and on this phase that is the likely
failure, because relevance scoring has the most room to go wrong exactly where the window is
smallest.

If `v2` loses, the phase still delivered: the store, the manifest, semantic degradation,
`expand`, the composed charter and cross-turn continuity are all independent of selection and
all keep their value. Record the loss and the weights that produced it.

**Depends on:** S-9.10, S-4.17 (flakiness and the sealed eval holdout).
**Acceptance:** the shipped weights are fitted and reproducible; promotion requires every tier;
a frontier-favouring regression is refused; a losing `v2` is recorded with its corpus rather
than quietly re-tuned until it passes.
**Traces:** AGENT-005 §22; §30.8. AGENT-009 §4, §4.2.

---

## Refinements — the eight ways a retrieval design fails that a buffer could not

### S-9.13 Selection quality: diversity, requirements, normalisation, hysteresis — L
`DONE` 2026-09-16 — all five, in `context/select.rs`.

Five defects a weighted sum has and an accumulation buffer does not. **Diversity:** top-k
picks ten records about one subject, because whatever scored one highly scores its neighbours
highly — strictly worse than recency, which at least samples across time. Maximal-marginal
relevance with a redundancy penalty on `(subject, kind, source)` and shared ids, plus hard
floors (≥1 `Correction`, ≥1 `Finding`, ≥1 record from the last two steps) so no weighting
starves a kind. **Requirements:** the next step's tool schema names required arguments; the
record that produced the `experiment_id` the step needs is not a candidate, it is a
must-include, satisfied before scoring. **Normalisation:** recency is `[0,1]`, citation count
is unbounded — summed raw the unbounded signal wins and the weights are decoration; normalise
every signal by percentile rank against the current store and record the normalisation in the
policy. **Hysteresis:** two thresholds, admit and drop, plus a stickiness bonus for anything
rendered last call. **Tie-break:** `(step asc, record id)`, never map order.

**Files:** `crates/harness/src/context/select.rs`.
**Acceptance:** on a ten-same-subject fixture the selected set contains at least one of each
required kind and never ten of one subject; a required id below the score cut is still
present; over ten steps with a slowly-changing query the set churns within bound and
cache-creation tokens do not rise against `v1`.
**Traces:** AGENT-005 §24.1–24.6; §30.11, §30.12, §30.13.

### S-9.14 What must never be dropped, and what must never be merged — M
`DONE` 2026-09-16 — corrections pin on the *upcoming* tool; two disagreeing records both render, adjacent, with the disagreement named.

**Pin corrections for the *upcoming* tool, not only the active one.** The plan names the next
steps; a rejection about a tool the model is about to reach is worth more than one about the
tool it just used, and dropping it buys a repeated error — the most expensive wasted step
there is.

**Disagreement is not redundancy.** Two records answering the same question differently — two
sources, a re-read after a revision, a statistic computed twice — look like near-duplicates to
S-9.13's redundancy penalty, so exactly one survives and the harness has silently picked a
winner. When two records share a subject and a measured quantity and their values disagree
beyond their stated error, **both render, adjacent, with the disagreement named**. Same rule
as AGENT-007 P5, inside a run, for the same reason: the disagreement is the finding.

**Acceptance:** a correction for the next planned tool survives a budget squeeze that drops
higher-scoring records; two disagreeing records are both rendered and the disagreement is
stated, asserted on a fixture built from two sources.
**Traces:** AGENT-005 §24.7, §24.8.

### S-9.15 Rendering economy — L
`DONE` 2026-09-16 — `context/render.rs`; all five savings, each with its own test.

Five losses of nothing. **Tables are not JSON** — a 200-row result spends ~40% of its tokens
on repeated keys, braces and quotes; render a header and delimited rows. **Shared schemas are
stated once** across same-shape selected records. **Precision follows the error bar** — a mean
printed as `0.0234871629` against `se = 0.004` spends eight tokens on noise and invites the
model to quote it; significant figures derived from `se`. **The second observation is a
delta** — "as r07, except close 42,118 → 42,340; 3 rows changed", full record one `expand`
away, which is the largest single saving on polling- and re-read-shaped runs and is exactly
what the idempotent-repeat guard already detects. **Repeated corrections fold** into one with
a count, which also makes "3rd rejection on this path" legible as the signal to hand over a
worked example.

**Files:** `crates/harness/src/context/render.rs`.
**Acceptance:** no rendered table is JSON; a schema shared by two selected records appears
once; `se`-derived precision holds; a re-read renders as a delta naming its base; three
same-path corrections render as one with a count.
**Traces:** AGENT-005 §25.1–25.5; §30.14, §30.15.

### S-9.16 Fitting the budget, and paying for the furniture — M
`DONE` 2026-09-16 — the value-loss-per-token fitter beats the greedy baseline on the fixture, and the greedy one is implemented so the claim is measured rather than asserted.

**Degrade-or-drop is a choice, not a sequence.** Walking each record's ladder greedily is
wrong: degrading one large valuable record may free less than dropping two small worthless
ones, and it loses more. Choose the cut with the best value-loss per token recovered, repeat
until it fits — bounded work over tens of records, and the difference between "the budget fit"
and "the budget fit well".

**The harness's own furniture has a budget.** Degraded markers, handles, provenance marks,
"why you are seeing this" lines and the manifest grow with record *count*, not content. One
`overhead` line item, capped at 12% of the body, met by shortening furniture — **never** by
dropping content to make room for labels about content.

**Acceptance:** the fitter beats greedy on a fixture where it matters, measured by retained
value; overhead stays within cap and the cap is met by shortening.
**Traces:** AGENT-005 §25.6, §25.7; §30.17.

### S-9.17 Store discipline — M
`DONE` 2026-09-16 — supersession, per-kind half-lives, `Effect::Spill`, and `MMR_CANDIDATES` bounding the quadratic term.

**Supersession:** a second read of the same instrument/timeframe/window supersedes the first;
selection sees only the head of a chain and the manifest shows it collapsed with its depth.
Without this, S-9.13's redundancy penalty spends effort suppressing what the store should
never have offered twice. **Per-kind decay:** a `Correction` from step 2 is dead by step 30; a
`Finding` from step 2 may decide step 30 — one constant for both is wrong in one direction.
Half-lives per `RecordKind`, in the policy, fitted with everything else. **Spill:** beyond
`max_resident_records` the coldest bodies go to the workspace, metadata stays resident so
scoring and the manifest still see the whole store. **Incremental scoring:** signals that
cannot change between calls are computed at insert; a 400-step run must not spend more wall
clock assembling prompts than waiting for models.

**Acceptance:** a superseded record is never selected and appears collapsed in the manifest;
spill is invisible except in latency and `expand` reads it back; assembly time per call stays
flat as the store grows past a thousand records.
**Traces:** AGENT-005 §26.1–26.4.

### S-9.18 Budget refinements, and the untrusted ceiling — M
`DONE` 2026-09-16 — step-scaled output reserve, a 25% untrusted share reported when it binds, and a manifest that groups rather than truncates.

**The output reserve follows the step.** One constant per profile means reserving the maximum
on every call, which is why the body is smaller than it needs to be on most of them. Scale it
with the active tool's expected output shape, within profile bounds.

**Untrusted content cannot take the window.** Fetched pages are records, and a page matching
the subject scores well — so three long articles can fill a body with untrusted text. That is
an economic problem and the precondition for the injection the provenance fence exists to
contain. Cap untrusted at a share of the body (default 25%), enforced in selection, reported
when it binds. **The manifest gets a hard ceiling** too, met by grouping harder, never by
truncating — a manifest that is cut lies about what exists.

**Acceptance:** three long fetched pages cannot exceed the untrusted share and the cap is
reported; a report-writing step reserves more output than a lookup; the manifest never
truncates.
**Traces:** AGENT-005 §27.1–27.3; §30.16.

### S-9.19 What would have helped, and why you are seeing this — L
`DONE` 2026-09-21

**Counterfactual ablation.** §19 measures what the model *used*, which is not what it
*needed* — a record can be used because it was there and a better one was not. Over recorded
tapes, re-run a step with one selected record removed and compare the resulting call; a record
whose removal changes nothing was not load-bearing whatever attribution said. Cheap exactly
here and nowhere else: sans-IO loop, tape supplies the model, no provider call. It is
`ablation_sweep` pointed at the platform's own prompt.

**Separate "unused" from "checked and dismissed"** — the first is a selection failure, the
second a selection success, and they look identical as "rendered, not cited". Where evidence
cannot separate them the metric says `indeterminate` rather than the flattering reading.

**Every record says why it is here** (one clause), and **every assembly emits a
`SelectionReport`** — candidates, scores by signal, selected, degraded and how, dropped, which
constraints bound — into the trajectory. That is what lets the desk's replay scrubber answer
"why did the model not know that?", the most common question about an agent that did something
strange and one no version of this system could previously answer.

**Depends on:** S-9.10, S-4.8.
**Acceptance:** ablation runs over a corpus with no provider call and yields a load-bearing
ranking that differs from the citation ranking; every rendered record carries a reason; the
report is in every trajectory and drives the scrubber.
**Traces:** AGENT-005 §28.1–28.4; §30.18, §30.19.


**What was already here, and what was not.** Three of the four halves were built:
`context::ablation` runs the counterfactual with no provider call,
`Utility::Indeterminate` keeps "unused" and "checked and dismissed" apart where the
evidence cannot, and `context::render` puts each selected record's clause in the
prompt.

**The missing hop was the last one, and it removed exactly the half the feature is
for.** `Note::Selected` carried `selected` and `elided` as two lists of opaque ids and
dropped `SelectionReport::scores` - where the clause lives. So the trajectory could say
*what* was in the prompt and not *why this record was not*, which is the question
28.3 names as the one the scrubber exists to answer. The desk compounded it:
`'selection'` was a declared card kind with no case in `Cards.tsx`, so the note
rendered through the fallback.

Both closed. `Note::Selected` now carries `reasons: Vec<SelectionLine>` - id, whether
it reached the prompt, the clause, and the score the decision was actually made on -
**elided first**, because that is the half a reader is looking for and a cap that kept
the winners would cut the half being asked about. Within each half, worst-scoring
first, so the near-misses meet at the boundary: the last record that lost sits beside
the weakest one that beat it, and the desk draws a rule between them.

`MAX_SELECTION_LINES` is 20 with `lines_omitted` beside it, because the note is
written on **every** step and a long run's store holds hundreds of records - and
because a truncated list that does not say it is truncated reads as a complete one,
which would have a reader conclude the record they are hunting for was never a
candidate.

**Still waiting on gate H-2, and only this:** "ablation runs over a *corpus*". The
sweep, the ranking and the difference from the citation ranking are asserted on a
fixture; the corpus is S-9.10's and does not exist. That is the same block, recorded
once.

**Evidence:** `harness::drive::tests::{the_records_that_lost_are_the_ones_the_note_leads_with,
the_near_misses_end_up_adjacent, every_line_says_why, a_capped_list_says_how_many_it_left_out,
a_nan_score_does_not_take_the_run_down, an_assembly_with_no_candidates_still_reports}`;
`context::ablation::tests::{the_sweep_makes_no_model_call,
the_load_bearing_ranking_differs_from_the_citation_ranking}`;
`frontend .../selection-note.test.tsx` (8 tests, including that a line with no clause is
dropped rather than rendered as a line with an empty one).

### S-9.20 Sub-run context is a projection, not a second pipeline — M
`DONE` 2026-09-16 — `context/projection.rs`; the critic's exclusion is by construction and the projection is recorded.

AGENT-010 §9 gives a critic "the evidence bundle only" and a worker "a byte-identical brief",
and neither says how those are built — assembling them by hand would be a second context
pipeline with none of this one's guarantees. A sub-run's context is a **projection of the
parent store**: a filter plus a budget, through the same selection, degradation and manifest.
The critic's projection excludes the parent's reasoning and corrections **by construction** —
which is what "fresh context" means mechanically rather than as an instruction — and carries
findings, the evidence they cite, and the dossier. The worker's projection is pinned so every
worker's brief is byte-identical, which is what makes fan-out cache-friendly. Projections are
declared in the policy and recorded on the sub-run, so a critic that saw too much is visible
after the fact rather than arguable.

**Depends on:** S-2.12 (sub-runs).
**Acceptance:** a critic projection provably excludes parent reasoning and corrections; two
workers receive byte-identical briefs; the projection used is recorded on the sub-run.
**Traces:** AGENT-005 §29; §30.20. AGENT-010 §9.

---

## What is built and what is waiting

Every mechanism in this phase exists and is tested. Three things are **not** claimed:

* `v2` has **not** been promoted, and must not be until S-9.12's gate is run. Every
  run is on `v1`.
* The weights in `policy::Weights::default` are the table §15 implies, **not** a
  measurement. `fit` exists to replace them and has not had a corpus to fit to.
* Attribution and ablation have been exercised against fixtures, not against a
  recorded corpus. They are correct over the tapes they have been given and have been
  given no real ones.

## Exit criteria

- Assembly is retrieval: the prompt answers "what does this step need", not "what happened
  recently".
- No record is ever cut at a byte offset; every shrink is a declared, labelled ladder step.
- Nothing leaves the prompt without leaving a manifest line and a handle, and a miss is
  recoverable in one step.
- Context policy is a versioned object, A/B testable offline over real trajectories.
- The selection weights are fitted to measured utility, not asserted in source.
- `v2` beat `v1` on every tier — or it did not ship, and the loss is on the record.
- Selection is diverse, stable and requirement-satisfying; it cannot collapse onto one subject,
  cannot lose an id the next call needs, and cannot thrash the cache.
- Nothing is rendered as JSON that a table would say cheaper, at a precision its error bar
  does not support, or twice when a delta would do.
- Untrusted content cannot take the window, and the harness's own furniture cannot crowd out
  the content it labels.
- "Why did the model not know that?" is answerable from the trajectory.
