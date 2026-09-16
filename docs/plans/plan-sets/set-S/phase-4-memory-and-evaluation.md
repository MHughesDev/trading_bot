# Phase 4 — Memory and evaluation

**Completion: 0% (0 / 17 tasks) done, but 14 are `PARTIAL` — the harness half is built and tested, and nothing persists.**

> **This file was wrong, and the correction matters more than any single task in it.** Until
> 2026-09-16 it recorded all seventeen tasks as `NOT STARTED`. S-4.2 was picked up to be
> built and was found already implemented, with its adversarial acceptance test passing under
> its own name. An audit of the other sixteen followed.
>
> What is actually true: `crates/harness/src/memory/` (brief, conflict, negative, retention,
> turn, verify) and `crates/harness/src/eval/` (cases, holdout, scorecard, tape) are about
> 6,800 lines carrying **98 passing tests**. Every migration this phase names is absent, and
> `crates/api/src/routes/memory.rs` does not exist.
>
> So the remaining work is not "build memory". It is **S-4.3, the semantic schema**, and then
> the persistence half of thirteen tasks whose logic is already written and tested. One task,
> S-4.14, is a human gate (H-2) rather than work.
>
> The lesson is the one ADR-I-01 records, in the other direction: a plan is a second source of
> truth, and a second source of truth drifts. A task marked `NOT STARTED` that is built costs
> a rebuild; one marked `DONE` that is not costs a shipped gap. This file had sixteen of the
> first kind.

**Requirement IDs:** RM-01…RM-15, EC-01…EC-13.
**Specs:** [AGENT-007](../../../specs/AGENT-007-research-memory.md),
[AGENT-009](../../../specs/AGENT-009-evaluation-and-calibration.md).
**Depends on:** Phase 1 (all tracks). The evaluation half additionally depends on
Phase 2 S-2.5 (`agent_events`) for real trajectories — until that lands, build the suites
against synthesised tapes.
**Blocks:** Phase 8's report layer; the Set-wide phase-exit gate.
**Migration range:** 0065–0069 (memory), 0070–0073 (evaluation).
**Parallel with:** Phases 3, 5, 6, 7.

Two halves that share a dependency and nothing else, so they can be two worktrees. Memory
makes the agent *understand*; evaluation makes every claim about the agent *measured*. They
meet at one place: `memory_retrievals.cited` is a score the eval suite reads.

**An eval harness already exists and is in CI** — `evals/runner/run.py`, `evals/tests`, run
by the `eval-graders` job. Extend it for model-in-the-loop suites; add Rust replay suites in
`crates/harness/src/eval/` only where determinism demands them. Do not build a second
harness beside it.

---

## Memory

### S-4.1 Turn summaries replace the prose carry — M
`PARTIAL` 2026-09-16 — `memory/turn.rs` (459 lines, 5 tests) builds the structured summary. `migrations/0065_agent_turn_summaries.sql` does not exist, so `prior_context` cannot yet be a renderer over a table — the acceptance's second half, a second turn receiving the first's conclusions with ids, needs the row.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

`conversations::prior_context` (`conversations.rs:339`) builds a "you concluded" string from
the model's own final prose — which the hosted driver then ignored entirely. Replace it with
a structured summary the **harness** writes from the findings ledger and the effect log:
findings with ids and confidence, negative results, trials spent, `n_eff_after`, gate states,
open questions, rejected ideas with reasons, budget, current step. Rendered at ≤ 600 tokens,
truncating by confidence and never dropping ids.

**Files:** `migrations/0065_agent_turn_summaries.sql`,
`crates/api/src/agent/conversations.rs`, `crates/harness/src/drive.rs`.
**Acceptance:** `prior_context` becomes a renderer over the table; a second turn receives the
first's conclusions with ids on both a 7B and a frontier profile.
**Traces:** AGENT-007 §2.1; §8.2.

### S-4.2 `record_finding` becomes verified — M
`PARTIAL` 2026-09-16 — **The acceptance is met in full on the harness side**, and this is the task that exposed the drift. `memory/verify.rs` (468 lines, 10 tests) carries exactly the named cases, adversarial one included: `a_finding_citing_an_id_no_tool_returned_is_refused` and `a_number_absent_from_the_cited_result_is_stored_suggestive`. `drive.rs:2518` names this task in a comment. What is missing is `0066_semantic_memory.sql`: the verdict is computed and the `suggestive` label has nowhere to be stored.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

Today it accepts any two strings. New behaviour: every id in `evidence` must be in the run's
`seen_ids`; any number in the claim must appear in a result carrying that id or the finding
is stored `suggestive`, never `established`; provenance and `valid_until` are set from the
run and the data window.

**Files:** `crates/harness/src/drive.rs`, `migrations/0066_semantic_memory.sql`.
**Acceptance:** a finding citing an id no tool returned is refused with a correction; a
finding whose number is absent from the cited result is stored `suggestive`. Adversarial: the
refusal is asserted.
**Traces:** AGENT-007 §3.1, P1; §8.1.

### S-4.3 The semantic schema — M
`NOT STARTED` — Unchanged, and it is the keystone: nearly every other task in this phase is waiting on this schema. `migrations/0066_semantic_memory.sql` does not exist and `0055_knowledge_plane.sql` is unextended.

Extend `migrations/0055_knowledge_plane.sql`'s tables with the shared fields (subject, claim,
evidence_ids, n, se, confidence, evidence_class, data_window, valid_until, provenance,
superseded_by, conflicts_with, stale, embedding) and the six entry kinds.

**Files:** `migrations/0066_semantic_memory.sql`, `crates/api/src/routes/`.
**Acceptance:** `cargo test -p invariants --test db_invariants -- --ignored` green (it asserts
every migration applies to an empty database); an entry without provenance cannot be inserted.
**Traces:** AGENT-007 §2.2.

### S-4.4 Negative results are derived, not declared — M
`PARTIAL` 2026-09-16 — `memory/negative.rs` (486 lines, 7 tests) derives the failure class from the verdict and holds the closed enum. Nothing writes a row, so the refusal of a class the funnel did not produce is enforced in the type and not yet at the database.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

When the funnel refuses a gate, a study fails its objective, or a workflow verdict is
`reject`, the **platform** writes the negative result with a `failure_class` from a closed
enum and a `retry_condition` from the verdict. The agent's tool may only annotate.

**Acceptance:** a negative result cannot be created with a class the funnel did not produce;
an agent attempting it is refused. This is the mechanism that stops "capacity_insufficient"
being used to keep a dead idea alive.
**Traces:** AGENT-007 §3.2; §8.3.

### S-4.5 The task brief, and the retrieval log — L
`PARTIAL` 2026-09-16 — `memory/brief.rs` (936 lines, 14 tests) assembles the brief. `migrations/0067_memory_retrievals.sql` does not exist, so `memory_retrievals.cited` — the one field the evaluation half reads from the memory half — has no table.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

At run start the harness assembles the brief into `Reference` blocks under that section's
share: user preferences in scope, instrument profile, relevant findings, negative results
(including any whose retry condition is now met, flagged), conflicts **always both sides**,
and the resident playbook listing in the charter tail. Every item renders as one line plus
ids plus *why it was retrieved*. Lexical retrieval first (reuse `registry::search`'s scorer);
embeddings later.

**Files:** `crates/harness/src/{context,drive}.rs`, `migrations/0067_memory_retrievals.sql`.
**Acceptance:** two conflicting findings are both retrieved and marked; a stale finding is
flagged; retrieval works with the embedding sidecar stopped; `memory_retrievals.cited` is set
when a later finding or `finish_task` names the entry.
**Traces:** AGENT-007 §4, P4, P5; §8.4, §8.5, §8.8.

### S-4.6 The user's own knowledge — S
`NOT STARTED` — Unchanged. Both named files are absent: `crates/api/src/routes/memory.rs` and `migrations/0068_user_preferences.sql`.

`ask_user` answers and "remember that…" turns become `user_preference` entries with a scope,
retrieved into every brief for that scope at top rank, listable and deletable by the user.

**Files:** `crates/api/src/routes/memory.rs`, `migrations/0068_user_preferences.sql`.
**Acceptance:** the agent stops re-asking a question the user has answered, demonstrated by a
two-turn test; the user can delete a preference and the next brief omits it.
**Traces:** AGENT-007 §3.4.

### S-4.7 Instrument profiles by schedule — M
`PARTIAL` 2026-09-16 — `memory::Kind::InstrumentProfile` and `jobs::JobKind::InstrumentProfile` exist, so the profile is a job kind that spends no trial. The schedule, and retaining a superseded profile rather than overwriting it, need the semantic schema.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

A weekly job per initialised asset running the Phase 3 `analysis` templates at 1h and 1d,
writing one `instrument_profile` that supersedes the last. Cost is bounded: templates, no
trials.

**Depends on:** S-3.3, S-3.4.
**Acceptance:** the profiler spends no trial; a superseded profile is retained, not
overwritten.
**Traces:** AGENT-007 §3.3.

---

## Evaluation

### S-4.8 Trajectories and tapes — L
`PARTIAL` 2026-09-16 — `harness/src/trace.rs`, `eval/tape.rs` (579 lines, 6 tests) and the `evals/` harness all exist. The acceptance is about **live runs** — every live run's trajectory replaying to identical effects — and there are none, which is S-4.14's blocker rather than this one's.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

A trajectory is the ordered `(Input, Vec<Effect>)` pairs a `Loop` produced plus profile,
registry hash, task and per-step state. Extend `crates/harness/src/trace.rs` `Event` with
`CoreTool{name,args,reply}` and `StateSnapshot{step,hash}` so harness-owned tools are
recorded — closing the gap `PROGRESS.md` reports as ADR-P0-18. A **model tape** is the
sequence of raw replies; replay feeds it back through the same `Loop` version and asserts
identical effects. Tapes are hand-editable to construct a case.

**Files:** `crates/harness/src/trace.rs`, `crates/harness/src/eval/`, `evals/`.
**Acceptance:** every live run's trajectory replays to identical effects; `is_replayable`
returns true for a nightly sample of real runs.
**Traces:** AGENT-009 §2, P1; §8.1.

### S-4.9 The replay suites — L
`PARTIAL` 2026-09-16 — All four named cases exist in `eval/cases.rs` (1,270 lines, 8 tests): the alphabetical truncation, the truncated reply, `finish_task` eating the only step, and the unread `idempotent`. Not marked done because the budget suite's §2.2 arithmetic assertion was not re-verified in this audit.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

`routing`, `budget`, `stopping` — all deterministic, all run on every PR in seconds. Seed
them from the nine documented failures in `LOCAL_TIER_FINDINGS §8–9`, each of which was the
harness overruling a model that had done the right thing.

**Acceptance:** the alphabetical-truncation case, the truncated-reply fence, the
`finish_task`-ate-the-only-step case and the unread-`idempotent` case all exist as passing
replay cases; the budget suite asserts AGENT-005 §2.2 arithmetic holds on every call.
**Traces:** AGENT-009 §3; §8.2.

### S-4.10 The honesty suite — M
`PARTIAL` 2026-09-16 — The honesty suite exists with named cases (`honesty/the_evidence_correction_is_satisfiable`, `honesty/a_flat_tier_is_graded_against_its_own_schema`). Its acceptance — that each case fails loudly when its mechanism is removed — is a mutation claim this audit did not test, so it is not reported as met.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

Planted ground truth, not an LLM judge: a planted oracle must be killed by `run_oracle_test`
rather than rewarded; a fake id in `finish_task` must be refused; a `suggestive` finding must
never be reported `established`; a seeded lookahead must be caught by the leakage battery;
the null comparison must not be skippable before a gate.

**Depends on:** S-3.6…S-3.11.
**Acceptance:** each case fails loudly when its mechanism is removed — the suite is tested by
breaking the thing it guards.
**Traces:** AGENT-009 §3; AGENT-006 §10.6.

### S-4.11 Scorecard and the non-inferiority gate — M
`PARTIAL` 2026-09-16 — `eval/scorecard.rs` (880 lines, 15 tests) holds the scorecard and the non-inferiority gate. Whether a regressing PR is actually **blocked** depends on CI wiring that was not checked here.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

Per profile per suite: pass rate, failure attribution by rung (routing / exposure / decode /
validation / tool / memory / model choice), cost, median steps, cache hit, memory recall,
HITL precision. Published by CI as an artifact. The gate: **no tier regresses by more than
2 points and at least one improves**, or the PR is labelled `neutral` with a reason.

**The gate obeys the statistics it demands of strategies.** Deterministic replay suites are
exact — a newly failing case blocks, no threshold. Stochastic live suites need a one-sided 95%
Wilson interval below zero and **n ≥ 30** per suite per profile; below that the suite reports
`underpowered` and can neither block nor clear. Benjamini-Hochberg across (profile × suite)
within a PR, because the gate runs on every PR and is therefore a repeated test. A profile
with no prior scorecard establishes a `baseline` and reports `unmeasured` until two exist —
otherwise adding a profile is the obvious way around the gate.

**Acceptance:** a PR that regresses the 7B's routing replay suite is **blocked even if the
frontier improves**; a 3-point move on a live suite with n = 12 is reported and does **not**
gate; a brand-new profile cannot pass by having nothing to regress against.
**Traces:** AGENT-009 §4, P4; §8.3.

### S-4.12 Calibrate the estimator — M
`PARTIAL` 2026-09-16 — `context::calibration_factor` is built, is p90 nearest-rank and refuses below ten samples (S-1.18). `migrations/0070_token_calibration.sql` does not exist, so there is no per-model row and no over-estimate rate to report.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

`agent_token_calibration(model, bytes_per_token, sample_n, updated_at)` computed from the
median ratio of the provider's `prompt_tokens()` to the harness estimate over the last 50
runs, clamped to [2.5, 5.0], with 4.0 as the prior for an unseen model.

**Depends on:** S-2.8.
**Files:** `migrations/0070_token_calibration.sql`, `crates/harness/src/context.rs`.
**Acceptance:** a row exists per model with ≥ 10 runs; the budget suite reports the
over-estimate rate and it stays ≥ 95%.
**Traces:** AGENT-009 §5, P7; §8.5. AGENT-005 §7.

### S-4.13 Replace the prose profile claims with measurements — M
`PARTIAL` 2026-09-16 — No file under `config/` contains `MultiStepEvidence` prose, so the first half of the acceptance holds. Pointing each local profile at a scorecard row needs the calibration table from S-4.12.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

`MultiStepEvidence` is hand-written prose about a run someone did once. Generate it from the
`decode` and `routing` suites on named hardware. Run the context-pressure sweep (the
`research` suite at 25/50/75/100% of declared budget) to set `effective_budget_tokens` where
quality stops degrading, and record `effective_budget_evidence`.

**Acceptance:** no `config/profiles/*.yaml` contains `MultiStepEvidence` prose; each local
profile points at a scorecard row; an uncalibrated profile loads with a warning and reports
its tier as `unmeasured`.
**Traces:** AGENT-009 §5, P3; §8.4.

### S-4.14 The live session that has never happened — M
`BLOCKED:HUMAN` 2026-09-16 — Not started and **not startable**: it needs a real 30-iteration run against a frontier model, and `eval/scorecard.rs:196` records why in the source itself — "a credential the platform does not have (S-4.14, roadmap gate H-2)". This is a human gate, not work. `NOT STARTED` implied somebody could pick it up.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists. · **BLOCKED:HUMAN** — needs an Anthropic credential in the encrypted store.

Set L's own MASTER says it plainly: "the one thing that has never happened: a live agent
session… It needs an Anthropic credential in the encrypted store, which this work did not
have and did not create." Two acceptance criteria in this set cannot be closed without it:
AGENT-005 §10.2 (cache hit ≥ 80% over iterations 3–30) and AGENT-009's `decode` suite.

**Human action required:** add a provider credential via Settings → LLM credentials, then an
agent can run the measurement.
**Acceptance:** one frontier and one local 30-iteration run recorded, with `Usage` cache
fields populated and the first calibration rows written.
**Traces:** AGENT-005 §10.2; AGENT-009 §8; MASTER §7 (the only criteria an agent cannot close
alone).

### S-4.15 Contradiction detection — the memory-poisoning guard — M
`PARTIAL` 2026-09-16 — `memory/conflict.rs` (479 lines, 8 tests) implements the detection including the subtle half — an overlapping interval is not flagged as disagreement. Neither named file exists, so nothing links two rows because there are no rows.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

AGENT-007 P5 says conflicts are surfaced and never merged, and every entry has
`conflicts_with[]` — but in the first draft nothing computed it, which would have left the
principle as a permanently empty field. On every finding write, check `established` findings
sharing the subject and an overlapping horizon; a contradiction is an **opposing direction on
the same measured quantity**, taken from the structured evidence, not from parsing prose. Two
findings whose confidence intervals overlap are not a contradiction. Both rows are linked,
**neither is auto-downgraded** (a later finding is not truer for being later), and resolution
is a research act: a third finding citing both.

Without this, one wrong `established` finding becomes an authoritative premise for every later
run and the whole honesty stack rests on an unchallenged claim.

**Files:** `crates/api/src/routes/memory.rs`, `migrations/0069_conflicts.sql`.
**Acceptance:** an opposing-direction finding links both rows and downgrades neither;
overlapping-interval disagreement is **not** flagged; both sides enter the brief together.
**Traces:** AGENT-007 §2.2.1, P5; §8.4.

### S-4.16 Retention, and the same finding twice — M
`PARTIAL` 2026-09-16 — `memory/retention.rs` (290 lines, 7 tests) has the archive rule and near-duplicate corroboration. An archived finding still resolving by id is a property of a store that does not exist yet.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

A store that only grows does not fail loudly, it degrades quietly: the brief fills with old,
narrowly-true findings and the agent's understanding gets *worse* over time while every
dashboard stays green. Retention per kind (AGENT-007 §2.4): findings archive when stale and
uncited for 180 days, profiles keep the latest and archive superseded rows after 90, negative
results are kept indefinitely because they are the cheapest row in the store and the likeliest
to save a budget. **Nothing an evidence chain points at is ever hard-deleted** — archived
means out of retrieval, still addressable, so an old dossier's citations never break.

And near-duplicate findings (same subject, same quantity, overlapping window and overlapping
intervals) do not create a row: they **increment a corroboration count** and append their ids.
That is better than dedup — a second independent measurement is evidence, and a finding
corroborated across three windows should outrank a singleton in retrieval.

**Files:** `crates/api/src/routes/memory.rs`, `migrations/0069_retention.sql`.
**Acceptance:** an archived finding leaves the brief but still resolves by id; a
near-duplicate increments corroboration rather than inserting; a monthly report gives store
size, archive rate and cited share.
**Traces:** AGENT-007 §2.4, §2.5.

### S-4.17 Flakiness, and a holdout for the evals themselves — M
`PARTIAL` 2026-09-16 — `eval/holdout.rs` (258 lines, 7 tests) seals the eval holdout, and `a_single_case_suite_seals_nothing_rather_than_sealing_everything` is the degenerate case handled. `migrations/0073_eval_holdout.sql` does not exist, so a re-draw cannot be recorded with the version that asked for it.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

A pass rate hides variance, and a gate that fires at random spends the team's trust: every
live suite runs k = 3 repeats with different seeds, and a suite whose flakiness exceeds 0.15
is reported `unstable` and **excluded from gating** until fixed.

Then the uncomfortable one. Every harness change is tuned until the suites pass, the suites
are small, and after twenty changes the harness is fitted to its own cases — and the scorecard
says everything is fine in exactly the way a curve-fitted backtest does. So 20% of every
suite's cases are **sealed by hash, never run on PRs**, and run only on the nightly promotion
to `current`. A version whose open-set rate exceeds its sealed-set rate by more than the
flakiness band is fitted to its evals and is not promoted. It is the identical argument the
platform makes to a researcher who wants one more look at a holdout, and a rule the platform
will not apply to itself is a rule it does not believe.

**Files:** `crates/harness/src/eval/`, `evals/`, `migrations/0073_eval_holdout.sql`.
**Acceptance:** an unstable suite cannot block or clear a PR; the sealed set never runs on a
PR; a harness version fitted to the open set fails promotion; a re-draw is recorded with the
version that asked for it.
**Traces:** AGENT-009 §4.1, §4.2.

---

## Exit criteria

- A conversation's conclusions survive into the next turn as structure, not prose, on every
  tier.
- A finding cannot claim an id no tool produced, and a dead idea cannot be relabelled.
- Every live trajectory replays to identical effects, and the nine historical harness bugs
  are permanent replay cases.
- The scorecard exists per tier and the gate blocks a frontier-favouring regression.
- Every profile claim is either measured or reports itself `unmeasured`.
- **Retroactive check:** re-run the gate against Phases 0–3, which were built before it
  existed.
