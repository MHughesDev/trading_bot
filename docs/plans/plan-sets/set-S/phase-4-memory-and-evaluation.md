# Phase 4 — Memory and evaluation

**Completion: 0% (0 / 17 tasks)**

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
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

Extend `migrations/0055_knowledge_plane.sql`'s tables with the shared fields (subject, claim,
evidence_ids, n, se, confidence, evidence_class, data_window, valid_until, provenance,
superseded_by, conflicts_with, stale, embedding) and the six entry kinds.

**Files:** `migrations/0066_semantic_memory.sql`, `crates/api/src/routes/`.
**Acceptance:** `cargo test -p invariants --test db_invariants -- --ignored` green (it asserts
every migration applies to an empty database); an entry without provenance cannot be inserted.
**Traces:** AGENT-007 §2.2.

### S-4.4 Negative results are derived, not declared — M
`NOT STARTED`

When the funnel refuses a gate, a study fails its objective, or a workflow verdict is
`reject`, the **platform** writes the negative result with a `failure_class` from a closed
enum and a `retry_condition` from the verdict. The agent's tool may only annotate.

**Acceptance:** a negative result cannot be created with a class the funnel did not produce;
an agent attempting it is refused. This is the mechanism that stops "capacity_insufficient"
being used to keep a dead idea alive.
**Traces:** AGENT-007 §3.2; §8.3.

### S-4.5 The task brief, and the retrieval log — L
`NOT STARTED`

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
`NOT STARTED`

`ask_user` answers and "remember that…" turns become `user_preference` entries with a scope,
retrieved into every brief for that scope at top rank, listable and deletable by the user.

**Files:** `crates/api/src/routes/memory.rs`, `migrations/0068_user_preferences.sql`.
**Acceptance:** the agent stops re-asking a question the user has answered, demonstrated by a
two-turn test; the user can delete a preference and the next brief omits it.
**Traces:** AGENT-007 §3.4.

### S-4.7 Instrument profiles by schedule — M
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

`routing`, `budget`, `stopping` — all deterministic, all run on every PR in seconds. Seed
them from the nine documented failures in `LOCAL_TIER_FINDINGS §8–9`, each of which was the
harness overruling a model that had done the right thing.

**Acceptance:** the alphabetical-truncation case, the truncated-reply fence, the
`finish_task`-ate-the-only-step case and the unread-`idempotent` case all exist as passing
replay cases; the budget suite asserts AGENT-005 §2.2 arithmetic holds on every call.
**Traces:** AGENT-009 §3; §8.2.

### S-4.10 The honesty suite — M
`NOT STARTED`

Planted ground truth, not an LLM judge: a planted oracle must be killed by `run_oracle_test`
rather than rewarded; a fake id in `finish_task` must be refused; a `suggestive` finding must
never be reported `established`; a seeded lookahead must be caught by the leakage battery;
the null comparison must not be skippable before a gate.

**Depends on:** S-3.6…S-3.11.
**Acceptance:** each case fails loudly when its mechanism is removed — the suite is tested by
breaking the thing it guards.
**Traces:** AGENT-009 §3; AGENT-006 §10.6.

### S-4.11 Scorecard and the non-inferiority gate — M
`NOT STARTED`

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
`NOT STARTED`

`agent_token_calibration(model, bytes_per_token, sample_n, updated_at)` computed from the
median ratio of the provider's `prompt_tokens()` to the harness estimate over the last 50
runs, clamped to [2.5, 5.0], with 4.0 as the prior for an unseen model.

**Depends on:** S-2.8.
**Files:** `migrations/0070_token_calibration.sql`, `crates/harness/src/context.rs`.
**Acceptance:** a row exists per model with ≥ 10 runs; the budget suite reports the
over-estimate rate and it stays ≥ 95%.
**Traces:** AGENT-009 §5, P7; §8.5. AGENT-005 §7.

### S-4.13 Replace the prose profile claims with measurements — M
`NOT STARTED`

`MultiStepEvidence` is hand-written prose about a run someone did once. Generate it from the
`decode` and `routing` suites on named hardware. Run the context-pressure sweep (the
`research` suite at 25/50/75/100% of declared budget) to set `effective_budget_tokens` where
quality stops degrading, and record `effective_budget_evidence`.

**Acceptance:** no `config/profiles/*.yaml` contains `MultiStepEvidence` prose; each local
profile points at a scorecard row; an uncalibrated profile loads with a warning and reports
its tier as `unmeasured`.
**Traces:** AGENT-009 §5, P3; §8.4.

### S-4.14 The live session that has never happened — M
`NOT STARTED` · **BLOCKED:HUMAN** — needs an Anthropic credential in the encrypted store.

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
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

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
