# Phase 4 — Memory and evaluation

**Completion: 71% (12 / 17 tasks) done, 9 `PARTIAL`, 1 `BLOCKED:HUMAN` — the harness half was built and tested, and now much of it persists.**

> **Audited a second time, 2026-09-23, against acceptance criteria rather than against code presence.** Four `PARTIAL` markers were under-claimed (S-4.3, S-4.5, S-4.7, S-4.9): every clause held, with CI-running evidence, and the note just hadn't caught up. Moved to `DONE`. Four others were over-claimed: their `PARTIAL` note asserted something false about the tree — a file said not to exist that does, a caller said to exist that doesn't. Those stay `PARTIAL`, with the note corrected to name the real blocker (S-4.8, S-4.13, S-4.15, S-4.17). The remaining five `PARTIAL`s (S-4.6, S-4.10, S-4.11, S-4.12, S-4.16) were checked and found correctly `PARTIAL`, several with a sharper gap than the note previously stated.

> **Updated 2026-09-16.** The sentence this header used to end with — "and nothing persists" — was true that morning and is no longer. Four tables exist and are verified against a live Postgres: `agent_memory.entry` (0122), `agent_turn_summaries` (0123), `memory_retrievals` (0124), and the stores over them. The carry between turns is structured rather than the model's own prose, and `assemble_brief` — 936 tested lines that had **no caller anywhere in the workspace** — now runs in the driver.
>
> **The migration numbers are not the plan's.** 0065–0073 were allocated against a tree that has since moved past them; the tree is at 0121 and these follow it. The same collision ADR-S3-08 records for S-3.7.
>
> **OQ-18 is resolved** (ADR-S4-04) and the write path is open: a finding's window is the union of the windows of the results it cites, and every evidence-producing tool now stamps one — `run_backtest` was the last, and its absence would have rejected the central case.
>
> **What the remaining `PARTIAL`s wait on is mostly the funnel and the scheduler, not the store.** The derivations are built and the constraints enforce them; what is missing is the event that fires them: nothing enqueues the weekly profiler (S-4.7), and the evaluation half (S-4.8 onward) waits on phase 2's `agent_events` for real trajectories.
>
> The other half of the remainder is the evaluation side (S-4.8 onward), which depends on phase 2's `agent_events` for real trajectories.

> **The judgement suite: the planted corpus is built, 2026-09-21, and it found this
> file wrong a second time.**
>
> This file says the evaluation half "waits on phase 2's `agent_events` for real
> trajectories". `crates/harness/src/eval/judgement.rs` — the scoring for all six
> criteria, with tests — **has been in the tree since S-10.16**, recorded in
> `phase-10-lifecycle.md`. The evaluation work is split across two phase files and
> neither cross-references the other, so each reads as though the other half does
> not exist.
>
> New: `eval/planted.rs` (a seeded generator, the answer key, a reference
> detector) and `eval/discrimination.rs` (the two rates, `TierScore`, a declared
> `Threshold`, `done_4`). **722 → 747 tests.**
>
> **Three rules are types rather than conventions.** `Discrimination` reports
> `miss_rate()` and `false_positive_rate()` and **has no method that combines
> them** — no accuracy, no F-score — because a single number over a mixed set is
> gamed by a constant answer, and `a_constant_answer_fails_whichever_constant_it_is`
> proves both constants fail. `TierScore` is `Unmeasured(NotMeasured)` or
> `Measured(Measurement)`, and `Measurement` is reachable only through the second
> variant, so **rendering an unmeasured tier as a number is a compile error**.
> `Threshold`'s only constructor is a *private* `const fn` and it derives
> `Serialize` but **not** `Deserialize`, so a bar cannot be built from a
> measurement or reconstituted from data — a suite that sets its bar from the
> scores it is judging never fails anybody.
>
> **The answer key is generated and never stored.** No `expected.json`, no table,
> no fixture; no `Serialize`/`Deserialize` on `AnswerKey`, `PlantKind` or
> `judgement::Truth`, so every JSON body, tool result and tape is a compile error
> rather than a review item; `Debug` is redacted, because `{:?}` in a `tracing`
> line is the ordinary route from a private field to a public string. The
> agent-visible surface is **tested**, and the test proves its search tokens are
> real discriminators by asserting they appear in the key's own rendering — a
> search for a string that appears nowhere passes trivially.
>
> **The residual exposure, stated rather than hidden:** the seed reconstructs the
> corpus, key included. Keeping it out of the runner's environment is a deployment
> property this crate cannot enforce.
>
> **Every tier is honestly `Unmeasured(NeverRun)` today**, and that is DONE-4's
> second clause being satisfied rather than evaded: generating ≥30 verified cases
> per tier is done, *running* them needs a recorded trajectory through the planted
> data, which needs the live-run credential (S-4.14, gate H-2).
>
> One criterion is **refused by name**: "a dead idea already in memory is refused
> at intake" has its answer in the semantic memory store, not in data, so no case
> was planted whose answer could not be verified. And the workflow-shape,
> chain-stopping, falsifier-evaluability and yield criteria are properties of the
> *run* rather than of the data — asserted against a label, not measured against a
> planted mechanism, and the module header says so.
>
> Three spec contradictions found, none implemented: Q-S4-01 (§3's `expected.json`
> format is the answer key on a filesystem the agent reaches), Q-S4-02 (a miss rate
> over a replayed tape measures the tape), Q-S4-03 (§4.2's sealed 20% hides the
> cases and not the answers when the corpus is generated).

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
`DONE` 2026-09-16 — `migrations/0123_agent_turn_summaries.sql` (not 0065; the ADR-S3-08 collision), `crates/api/src/turn_summary_store.rs`, and `prior_context` now renders from it.

Both halves of the acceptance hold and are verified against a live Postgres. `prior_context` reads the structured summaries first and falls back to the old prose only for turns that predate the table — a silent fallback would look like a working carry while reintroducing the failure, so a malformed row warns.

A second turn receives the first's conclusions **with ids**: asserted on `f_1`, `exp_42`, `t_9` and `n=812` in `crates/api/tests/turn_summary_round_trip.rs`, together with the retry-replaces-not-appends rule (the carry must be what the turn finally did) and the per-conversation scope.

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
`DONE` 2026-09-16 — the verdict is computed **and stored**.

The harness half was met all along and is what exposed this file's drift: `memory/verify.rs` (468 lines, 10 tests) carries exactly the named cases, adversarial one included — `a_finding_citing_an_id_no_tool_returned_is_refused` and `a_number_absent_from_the_cited_result_is_stored_suggestive`.

What was missing was the write, and the blocker was one field: `Entry` requires a `data_window` and the loop tracked none, so the verdict lived for exactly one run. **OQ-18 is resolved as ADR-S4-04** — the window is the union of the windows of the results the finding cites, derived rather than stated, because the claim rests on those results and nothing else. `Effect::Remember` carries the row out of the sans-IO loop and the driver stamps the run id, blank rather than a sentinel so `Entry::validate` refuses it if the driver ever forgets.

Building it found the real constraint. The first version *refused* a finding whose window could not be derived, and the eval suite went red on `honesty/a_suggestive_finding_is_labelled` — which led to the discovery that **`run_backtest` stamped no window** while every other producer did. Refusing would have rejected the central case. That is now fixed at the source (`shape_snapshot`), and until it was, a finding with no derivable window was recorded for the run and said out loud that it had not persisted.

**PARTIAL history:** this entry read `PARTIAL` for the harness-only state above. `memory/verify.rs` (468 lines, 10 tests) carries exactly the named cases, adversarial one included: `a_finding_citing_an_id_no_tool_returned_is_refused` and `a_number_absent_from_the_cited_result_is_stored_suggestive`. `drive.rs:2518` names this task in a comment. What is missing is `0066_semantic_memory.sql`: the verdict is computed and the `suggestive` label has nowhere to be stored.

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
`DONE` 2026-09-23 (was `PARTIAL` 2026-09-16, under-claimed) — `migrations/0122_semantic_memory.sql` written: `agent_memory.entry` with all the shared fields, the seven kinds, and the constraints that make the acceptance real.

**Two corrections to this task's own text.** It names `0066`, which was taken long ago — the tree is at 0121 and Set S's declared ranges were allocated against a tree that has since moved, the same collision ADR-S3-08 records for S-3.7's `0079`. And it says "the six entry kinds": that count is from §2's summary row, written before AGENT-013 §9 added `open_question`. The spec's own §2.2 table and `memory::Kind` both carry seven, and they agree with each other.

Every CHECK restates a rule `Entry::validate` already enforces. That is the point rather than redundancy: the acceptance is that an entry without provenance **cannot be inserted**, which is a claim about the database and not about one caller. All four provenance columns are `NOT NULL` with non-empty CHECKs, because `Provenance::is_complete` requires all four — a run id with no tool is not provenance.

Two constraints exist to stop a *future* writer: a `model_summary` can never be `established` (a named CHECK — it is the whole distinction the evidence classes draw), and there is no `winner` / `resolved_by` / `resolution` column, because §2.2.1 says conflicts are surfaced and never merged and a column recording which side won is a merge with a record of the loser. `crates/invariants/tests/schema_mirrors_memory_types.rs` asserts both, and compares the SQL's CHECK lists against the Rust enums so the two cannot drift.

**Verified against a live Postgres 2026-09-16.** The earlier note here said this was unverified because Docker was not running. That premise was false: there is a **native PostgreSQL 17.7 on port 5433** on this machine, and the verification took minutes once the assumption was checked rather than believed.

`crates/invariants/tests/semantic_memory_refuses.rs` states the acceptance as behaviour and proves it: fourteen refusals, each asserting the error names *the constraint it expects* rather than merely that the insert failed — a row can fail for the wrong reason, and an assertion that only checks for failure passes on all of them. All four provenance columns, both blank-string CHECKs, the 300-character claim, the empty subject, the three closed vocabularies, `a_model_summary_is_never_established`, `the_window_runs_forwards`, `measurement_is_whole_or_absent` and `entry_corroborations_check`.

`crates/api/tests/memory_store_round_trip.rs` covers what no unit test could: `MemoryStore::put` binds twenty-nine columns positionally, and a transposed pair is invisible to the compiler. Seventeen fields plus all five measurement parts survive the round trip, compared one at a time so a failure names the column.

**Re-audited 2026-09-23: the pgvector blocker above is stale, and this is `DONE`.** CI's `platform-invariants-live` job already declares `image: pgvector/pgvector:pg16` (comment: "pgvector, not plain postgres") and already runs `cargo test -p invariants --test db_invariants -- --ignored` — the acceptance verbatim. The deployed stack runs the same image with `vector 0.8.6` installed, and `agent_memory.entry.embedding` is a genuine `udt_name=vector` there, not the `bytea` substitute the local-Postgres fallback used. The earlier note was true of the native PG 17.7 on port 5433 an earlier audit used, and false of both CI and the deployed stack. All four `prov_*` columns are confirmed `NOT NULL` with live `btrim(...) <> ''` CHECKs, read directly out of `pg_constraint`.

Extend `migrations/0055_knowledge_plane.sql`'s tables with the shared fields (subject, claim,
evidence_ids, n, se, confidence, evidence_class, data_window, valid_until, provenance,
superseded_by, conflicts_with, stale, embedding) and the six entry kinds.

**Files:** `migrations/0066_semantic_memory.sql`, `crates/api/src/routes/`.
**Acceptance:** `cargo test -p invariants --test db_invariants -- --ignored` green (it asserts
every migration applies to an empty database); an entry without provenance cannot be inserted.
**Traces:** AGENT-007 §2.2.

### S-4.4 Negative results are derived, not declared — M
`DONE` 2026-09-16 — `memory/negative.rs` (486 lines, 7 tests) derives the class from the verdict, and **the database now enforces the same rule** (`migrations/0125_negative_result_class.sql`).

The acceptance says "cannot be created", not "is not created", and the difference is where the rule lives. `derive_negative_result` has no class parameter — the class comes from `FailureClass::from_source` over an event the funnel emitted, so there is no path from a string the model chose. That is a property of one function and holds until the second writer.

Three constraints, verified live, and the last two exist because the first alone is not enough: the class must be one of the ten; a negative result must *have* a class (without which the first is satisfied vacuously by omitting the field); and it must name its ticket, since a refusal nobody can trace to the gate that issued it cannot be checked against its own retry condition. Written over `extra` and phrased so the other six kinds are never asked about a field they do not have — asserted, because a constraint that quietly applied to findings would refuse every one of them.

`schema_mirrors_memory_types.rs` compares the list against `FailureClass::all()`, so a class added to one and not the other fails the build rather than a run.

**The event is wired now, and S-4.4 is `DONE`.** `advance_funnel`'s route writes one negative result per refused gate, through `crates/api/src/negative_from_funnel.rs` — in `crates/api` because it needs `backtest::gates` and `harness::memory` and neither crate depends on the other: the funnel does not need a memory store to run gates, and the store does not need a simulator to hold entries.

Gate 0 forced a fix on the way. It refuses for two unrelated reasons — a lookahead leak or a gross edge under the cost floor — and `integrity_scan`'s typed codes for them were being formatted into a sentence and discarded. That difference decides the retry condition, so "when the leak is fixed" and "when costs fall" were collapsing into one future; `GateVerdict` now carries its reasons and the two are told apart.

The later mappings are argued rather than assumed: gate 1 is `weak_effect` and not `no_better_than_null`, because the null has not run at that point and claiming otherwise would be a claim about a comparison nobody made; gate 2 is `fragile_parameters` and not `regime_specific`, because the gate varies configuration and windows rather than regime.

Idempotent by construction — the entry id is the ticket plus the gate — and a failed store is a warning, since the verdict is already recorded and returned.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

When the funnel refuses a gate, a study fails its objective, or a workflow verdict is
`reject`, the **platform** writes the negative result with a `failure_class` from a closed
enum and a `retry_condition` from the verdict. The agent's tool may only annotate.

**Acceptance:** a negative result cannot be created with a class the funnel did not produce;
an agent attempting it is refused. This is the mechanism that stops "capacity_insufficient"
being used to keep a dead idea alive.
**Traces:** AGENT-007 §3.2; §8.3.

### S-4.5 The task brief, and the retrieval log — L
`DONE` 2026-09-23 (was `PARTIAL` 2026-09-16, under-claimed) — `memory/brief.rs` (936 lines, 14 tests) assembles the brief, and **the retrieval log now exists**: `migrations/0124_memory_retrievals.sql` (not 0067; the ADR-S3-08 collision), `crates/api/src/retrieval_store.rs`.

The acceptance's fourth clause is met and verified against a live Postgres: `cited` is set when a later act names the entry, and the recall share follows from it. Four decisions are written into the migration rather than left implicit — `None` and `Some(0.0)` stay different answers (a run with nothing to recall has not failed at recall); there is **no foreign key to the entry**, because §2.4 lets an uncited entry be deleted and a cascade would remove the misses while keeping the hits, so the metric would improve whenever the store was tidied; one row per `(run_id, entry_id)` so a re-planned run is not penalised for reassembling its brief; and `mark_cited` returns its count, because citing an id the brief never showed is a different failure from a low share and is otherwise invisible.

The slot CHECK was wrong in every entry on the first attempt and would have refused every row the brief produced, at runtime — `schema_mirrors_memory_types.rs` now compares it against `Slot` directly.

**The assembler had no caller at all**, which is sharper than "needs the driver": 936 lines and 14 passing tests, exported and never executed. A module can be entirely correct and entirely unreached, and its own tests cannot tell you which.

It is wired now. `Loop::with_brief` takes what the driver assembled — a builder, so a run with no memory stays the ordinary construction — and the brief enters `body()` as `Reference`, below the state card and above the raw observations, held apart from `observations` so a retrieved claim does not read like something this run established. The budget is `Section::Reference`'s own 20% of the assembler's *effective* budget rather than the advertised window. Retrievals are logged before the run reads the brief, so the denominator survives a crash.

`crates/api/tests/memory_reaches_the_run.rs` walks the whole chain against the live Postgres: written through `MemoryStore`, read as the driver reads, assembled as the driver assembles, the user's own words found in the rendered blocks, and the recall share moving from 0.0 to 1.0 as the entry is cited.

**Re-audited 2026-09-23: this is `DONE`, and the note above was self-contradictory.** It said re-asserting the first three clauses "needs findings in the store, which is OQ-18" — while this file's own header says OQ-18 is resolved. All four clauses hold with CI-running evidence: `brief.rs:592` `both_sides_of_a_conflict_are_retrieved_and_marked`, `:666` `a_stale_finding_is_retrieved_and_flagged`, `:789` `retrieval_works_with_the_embedding_sidecar_stopped` (true by construction — `brief.rs` uses no embeddings at all today), and `:849` plus `RetrievalStore::mark_cited` plus migration 0124 for the fourth.

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
`PARTIAL` 2026-10-04 (S-CATALOGUE track) - The written-but-unrun Postgres tests were run on a scratch database (5433): `memory_reaches_the_run` 6/6 pass, including `an_answer_given_in_turn_one_is_in_turn_twos_brief_at_the_top` and `a_deleted_preference_does_not_reach_the_next_brief`; `memory_store_round_trip` 9/9. Still PARTIAL: "the agent stops re-asking" is model behaviour and needs a live turn (H-2).
`PARTIAL` 2026-10-03 (MEMORY track) - The two-turn test now exists: `memory_reaches_the_run::an_answer_given_in_turn_one_is_in_turn_twos_brief_at_the_top` records the answer as a preference (as `POST /api/memory/preferences` does), assembles a second run's brief and asserts the answer is rendered and ranks first. **Still open:** "the agent stops re-asking" is model behaviour; what is proved is that nothing but the model's own choice can make it ask again. Needs a live turn (H-2).

Previous marker `PARTIAL`, 2026-09-16 — `crates/api/src/routes/memory.rs` exists and serves the whole listable/deletable half; `POST /api/memory/preferences` records what the user said.

**`migrations/0068_user_preferences.sql` is not written, and that is the decision rather than the omission** (ADR-S4-01): `user_preference` is one of §2.2's seven kinds and shares every shared field with the other six, so a second table would be a second store for the brief to retrieve from, a second retention rule and a second thing `memory.search` must union — in exchange for a `scope` that fits in `extra`.

The acceptance's second half holds and is verified live: a deleted preference stops being returned, asserted on the *read* rather than on the `archived` flag, since a test of the flag would pass while the brief still showed it. This is also the one kind OQ-18 does not block — "the user said this" is true as of when they said it, so today-to-today is a fact rather than a guess, and `human` + `established` is the one combination needing no verification pass.

**What keeps it `PARTIAL`, re-checked 2026-09-23**: the driver wiring this note used to cite as the blocker is done — `local_driver.rs:1235` calls `assemble_brief`, `:1576` calls `with_brief` — so that reason is stale. The actual gap is narrower and still open: no test anywhere exercises two turns, so "the agent stops re-asking" is unasserted. The only re-asking hits in the tree (`state.rs:1279`, `one_working_state.rs:511`) are in-run question state, not a preference carried across turns.

`ask_user` answers and "remember that…" turns become `user_preference` entries with a scope,
retrieved into every brief for that scope at top rank, listable and deletable by the user.

**Files:** `crates/api/src/routes/memory.rs`, `migrations/0068_user_preferences.sql`.
**Acceptance:** the agent stops re-asking a question the user has answered, demonstrated by a
two-turn test; the user can delete a preference and the next brief omits it.
**Traces:** AGENT-007 §3.4.

### S-4.7 Instrument profiles by schedule — M
`DONE` 2026-09-23 (was `PARTIAL` 2026-09-16, under-claimed) — `memory::Kind::InstrumentProfile` and `jobs::JobKind::InstrumentProfile` exist, so the profile is a job kind that spends no trial: the acceptance's first clause holds by construction, since a job kind that registers no trial cannot spend one.

**The second clause is now built and verified**: `MemoryStore::supersede` writes the replacement and points the old row at it, leaving the old content untouched. `put` could not do this — it is `ON CONFLICT DO UPDATE`, right for a correction and wrong for a succession — and a profiler that overwrote would leave no way to ask what the instrument looked like last month, when "the profile changed" is often the finding.

The current view (`by_kind`, `for_subject`) excludes anything superseded, so one profile per instrument reaches the brief while the history stays queryable through `get`: retained and shown are different things. Both writes are one transaction with the replacement first, asserted by a succession the database refuses — half a succession shows either two profiles or none.

**Both acceptance clauses are now met and asserted.** The first — "the profiler spends no trial" — holds by construction: `JobKind::counts_trial` is the single place the platform decides and the counting happens inside the submission transaction, so a profiler that spent one could not do it quietly. `the_instrument_profiler_spends_no_trial` asserts it anyway, because it is cheap to lose by accident and expensive when lost, and a companion test pins the whole counting set so adding to it reads as the decision it is rather than a refactor.

**Re-audited 2026-09-23: `DONE` on the acceptance as written.** Both clauses are met with CI-running evidence: `the_instrument_profiler_spends_no_trial` (`crates/jobs/src/types.rs:673`, a lib test) asserts the first over `counts_trial`; `MemoryStore::supersede` (`memory_store.rs:301`) with `by_kind`'s `AND superseded_by IS NULL` filter proves the second.

**What is described but not in the acceptance, and remains unbuilt — real gap, kept open, does not block `DONE`**: the schedule. Nothing enqueues the profiler weekly per initialised asset — the job kind exists, the supersession it needs exists, and no scheduler fires it. `JobKind::InstrumentProfile` has no enqueue site and no worker anywhere in `crates/` or `apps/`; today it is an enum variant and a trial classification, not a running job. The acceptance under-specifies the task — it asks about what happens *if* the profiler runs, not whether anything makes it run — which is why this is `DONE` on its own text while the feature it describes is not live.

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
`PARTIAL` 2026-10-04 (S-CATALOGUE track) - Re-checked, no status change. Replay of live runs to identical effects needs live runs (H-2). Spans are rebuilt from `agent_effect_log`, whose ToolCall rows carry `result_bytes` and not the result, so a trajectory alone cannot rebuild a run's context store (relevant to S-9.10). Queued for run: `a_trajectory_is_read_from_the_effect_log`, `pg_agent_effect_log`.
`PARTIAL` 2026-10-04 (F-LIFECYCLE track) - The core-tool `args` fix is in place (`Note::CoreTool.args`, written by `describe_effect`, read by `trace_from_effects`); this pass found two core notes still logging `args: null` (`get_self_status`, `exposure_full`), which `trace::is_replayable` reads as "arguments not logged", so they now log `{}`. `spawn_replay_sampler` is called from `apps/platform/src/main.rs`. Still open: replay of live runs to identical effects needs live runs (H-2).
`PARTIAL` 2026-10-03 (MEMORY track) - `spans_for_run` now has a caller and tests: `a_trajectory_is_read_from_the_effect_log::{the_trajectory_is_the_effect_rows_in_order_with_arguments_intact, the_sampler_counts_replayable_runs_and_names_the_ones_that_are_not}` run the real SQL against a real `agent_effect_log` (0061). The nightly consumer exists: `trace_from_effects::sample_replayable` and `spawn_replay_sampler` (asks `is_replayable` of the 20 most recent runs). **Found while testing, and it is a real defect:** `describe_effect` writes a core-tool `Note` as `{core_tool, detail}` and `Note::CoreTool` carries no `args`, so a run that used `record_finding` or `finish_task` projects a `CoreTool` span with null arguments and is correctly reported unreplayable (`a_core_tool_note_as_production_writes_it_is_not_replayable_yet` pins it). ADR-P0-18 is closed for the name and open for the arguments; the fix is `args` on `Note::CoreTool` in `harness/src/drive.rs` and in `agent/local_driver.rs`, both off this track. **Still open:** (1) the platform must call the spawner, one line in `apps/platform/src/main.rs` (orchestrator); (2) "replays to identical effects" for live runs needs live runs (H-2). `is_replayable` is checked, not a full effect-for-effect replay of a stored run.

Previous marker `PARTIAL`, 2026-09-16 — this entry was right that live runs are the blocker and wrong that they are the only one.

**`harness::trace` had no producer.** 422 lines over nine tests — `Span`, `Event`, `is_replayable`, `unreplayable_reason` — and no `Span::new` anywhere in `crates/` or `apps/` outside its own tests. So "every live run's trajectory replays" was unreachable twice over: a live run would have produced no trajectory either.

`crates/api/src/trace_from_effects.rs` now derives spans from `agent_effect_log`, which already holds the ordered rows for every run and is what a resume reads. **Derived, not written twice**: a second write path would be a second record of one history, and `knowledge_jobs` already states the rule — a derived table that drifts from its source is worse than none, being a second answer with no way to tell which is stale.

**ADR-P0-18's gap is closed**: core-tool notes are logged structurally. `search_tools`, `record_finding`, `reflect` and `finish_task` are never dispatched — the gate short-circuits them — so they never became `ExecuteTool` rows, and a trajectory without them cannot replay a run whose whole content was findings, which is what the `honesty` suite's cases are made of. They had been written as a debug string, and a field recovered by parsing one is a derive away from being gone.

**Re-audited 2026-09-23: the claim above overstated it.** "Asserted against the real log producer, so what remains is the credential, not the code" is false on both halves. `spans_for_run` — the only function that touches Postgres — has zero callers and zero tests; the three tests in `trace_from_effects.rs` exercise `event_for` on hand-built `json!` literals via `Span::new`, never the query. And no nightly `is_replayable` sampler exists anywhere. `harness::trace` went from no producer to a producer with no consumer — still `PARTIAL`, but on a different axis than the credential, and the credential (S-4.14) is not the only thing standing between this and `DONE`.

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
`DONE` 2026-09-23 (was `PARTIAL` 2026-09-16, under-claimed) — All four named cases exist in `eval/cases.rs` (1,270 lines, 8 tests): the alphabetical truncation, the truncated reply, `finish_task` eating the only step, and the unread `idempotent`. The prior audit withheld `DONE` only because the budget suite's §2.2 arithmetic assertion had not been re-verified — re-checked 2026-09-23 at `cases.rs:1180`, `the_budget_arithmetic_holds_on_every_call_of_every_case`, which rewrites every case's expectation to `Expect::BudgetArithmeticHolds`. It's a lib test, so it runs in CI's `cargo test --workspace --lib`. All five clauses met; the `PARTIAL` recorded an audit gap, not a build gap.

Audited 2026-09-16 against the source rather than against the plan, after S-4.2 was found implemented while this file said `NOT STARTED`. `crates/harness/src/memory/` and `crates/harness/src/eval/` are ~6,800 lines carrying 98 passing tests; **no migration this phase names exists** and `crates/api/src/routes/memory.rs` does not exist. So the harness computes and nothing persists.

`routing`, `budget`, `stopping` — all deterministic, all run on every PR in seconds. Seed
them from the nine documented failures in `LOCAL_TIER_FINDINGS §8–9`, each of which was the
harness overruling a model that had done the right thing.

**Acceptance:** the alphabetical-truncation case, the truncated-reply fence, the
`finish_task`-ate-the-only-step case and the unread-`idempotent` case all exist as passing
replay cases; the budget suite asserts AGENT-005 §2.2 arithmetic holds on every call.
**Traces:** AGENT-009 §3; §8.2.

### S-4.10 The honesty suite — M
`DONE` 2026-10-03 (MEMORY track) - All five named mechanisms now have a broken twin. Tape cases: fake id, planted oracle, suggestive finding, seeded lookahead (new: `a_seeded_lookahead_is_reported_not_buried`); the fifth, the null comparison that cannot be skipped, is enforced by `report::critic` and not by the `Loop`, so its pair is `the_null_comparison_check_fails_when_removed`. `honesty_pairs()` holds the four tape pairs; `the_honesty_suite_fails_when_its_mechanism_is_removed` requires each intact case to pass and its twin, graded against the same expectations, to fail (the nightly twin covers any sealed pair). **Mutation-checked 2026-10-03:** making the lookahead twin report the lookahead made that test fail. Caveat: the twins change the *world* the tape records (the pattern the oracle case set); they prove the grading discriminates, not that the platform's own battery detects a lookahead, which `crates/api/tests/a_leakage_check_is_named_only_by_running_it.rs` covers.

Previous marker `PARTIAL`, 2026-09-16, re-checked 2026-09-23 and confirmed correctly `PARTIAL` — the mutation claim the first audit declined to test has now been checked directly. Exactly one of five honesty cases has a broken twin (`a_planted_oracle_rewarded_is_the_broken_version`, asserted by `the_honesty_suite_fails_when_its_mechanism_is_removed`); the other four have no such case. Worse, two of the five mechanisms this task's own body names have no case at all, broken or otherwise: no seeded-lookahead-caught-by-the-leakage-battery case, and no null-comparison-not-skippable-before-a-gate case.

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
`DONE` 2026-10-03 (MEMORY track) - The gate is now applied to a scorecard a PR actually produces. `cases::replay_scorecards` builds one per profile from the open replay set; `evals/results/replay-baseline.json` is the committed baseline (regenerated only by the ignored `write_the_replay_baseline`); `the_scorecard_a_pr_produces_does_not_regress_against_the_committed_baseline` runs `scorecard::gate` on every `cargo test --lib`. Acceptance: `a_regression_in_the_7b_routing_suite_blocks_even_if_the_frontier_improves` (real cards), `a_three_point_move_at_n_of_twelve_is_underpowered_and_does_not_gate` and `a_brand_new_profile_cannot_pass_by_having_nothing_to_regress_against` (scorecard.rs). **Scope, said plainly:** this is the replay half. The stochastic live suites still have no scorecard, so `evals/results/` holds only the replay baseline; live n >= 30 gating waits on H-2. Added `Scorecard::utility_rate` (S-9.10).

Previous marker `PARTIAL`, 2026-09-16, re-checked 2026-09-23 and confirmed correctly `PARTIAL` — `eval/scorecard.rs` (880 lines, 15 tests) holds the scorecard and the non-inferiority gate. The CI wiring the first audit deferred has now been checked and does not block a regressing PR. The deterministic half works: `every_case_passes_on_the_harness_it_was_written_against` runs under `cargo test --workspace --lib`. The stochastic half cannot run at all — `evals/results/` is empty, so no baseline scorecard has ever existed. CI's `eval-graders` job only runs `pytest evals/tests` (the gate's own arithmetic, not a scorecard) and a `--dry-run`. `check_gate` is never applied to a PR's actual scorecard, and the Rust `scorecard::gate` has exactly one production caller (`context/fit.rs:214`) — not CI.

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
`PARTIAL` 2026-10-04 (S-CATALOGUE track) - `eval_state_is_recorded_and_granted` run on a scratch DB: 5/5 pass (`a_row_exists_per_model_with_ten_runs_and_none_below`, `the_platform_role_can_use_both_tables` under `SET LOCAL ROLE app_role`, the re-draw and losing-policy tests). Unchanged gap: a calibration row needs ten runs with provider-counted prompt tokens and this box has one.
`PARTIAL` 2026-10-04 (F-LIFECYCLE track) - Loop-level budget finished: `remaining_body_bytes` converted tokens to bytes at a literal 4 while the estimator read the calibrated figure, so a model at 2.5 bytes/token was admitted 60% too many bytes; it now uses `context::current_bytes_per_token()` (`the_body_cap_in_bytes_follows_the_calibrated_bytes_per_token`). `eval_store::spawn_eval_jobs` (platform `main.rs`) refreshes calibration daily; the assembler already reads the row. `Scorecard::over_estimate_rate` + gate rule `MIN_OVER_ESTIMATE_RATE = 0.95` (`a_token_estimate_that_under_estimates_blocks_and_an_unmeasured_one_does_not`). Not met: a calibration row needs ten runs with provider-counted prompt tokens; this box has 55 runs and 1 with a token pair.
`PARTIAL` 2026-10-03 (MEMORY track) - Built: `context::calibrate_bytes_per_token` (last 50 runs, >= 10 or no answer, clamp [2.5, 5.0], prior 4.0), `over_estimate_rate`, migration `0177` (`agent_token_calibration`, granted to `app_role`, tested under `SET LOCAL ROLE app_role`), and `api::eval_store::{refresh_calibration, bytes_per_token, worst_over_estimate_rate}`. Tests: `context::tests::{a_calibrated_estimate_over_estimates_on_at_least_95_percent_of_runs, calibration_refuses_below_ten_runs_and_uses_the_prior, calibration_is_clamped_to_the_documented_range}`, `eval_state_is_recorded_and_granted::a_row_exists_per_model_with_ten_runs_and_none_below`. **ADR-S4-12-1:** the spec says *median*; its acceptance says the over-estimate rate stays >= 95%. A median over-estimates on half of all runs by construction, so the two cannot both hold; the 5th percentile of per-run bytes-per-token is used (rejected: the median). **Still open (outside this track):** `eval_store` needs `pub mod eval_store;` in `crates/api/src/lib.rs`; a caller of `refresh_calibration` (a daily spawn in `apps/platform/src/main.rs`); and the assembler in `crates/api/src/agent/local_driver.rs` reading `bytes_per_token` instead of the fixed 4.

Previous marker `PARTIAL`, 2026-09-16, re-checked 2026-09-23 and confirmed correctly `PARTIAL`, understated — `context::calibration_factor` is built, is p90 nearest-rank and refuses below ten samples (S-1.18). `migrations/0070_token_calibration.sql` does not exist, and the gap is worse than "no table": `agent_token_calibration` appears in no migration and no Rust file anywhere in the tree — the identifier does not exist. `context::calibration_factor` (`context.rs:560`) has no caller outside its own unit tests, and over-estimate-rate reporting exists nowhere in `crates/harness/`.

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
`PARTIAL` 2026-10-04 (S-CATALOGUE track) - Re-checked, no status change: no local profile names a scorecard row (ADR-S4-05) and none can until a live eval run produces one (H-2); `tier_label`/`measurement` are served by `GET /api/agent/profiles`.
`PARTIAL` 2026-10-04 (F-LIFECYCLE track) - `tier_label` / `measurement` now have a caller: `GET /api/agent/profiles` returns `tier_label` and `measurement`. The 7B profile's `multi_step_evidence.detail` prose is removed (structured fields kept, validated). No local profile names a scorecard row (ADR-S4-05; rejected: pointing at the replay row, which has no hardware), so they report `unmeasured`.
`PARTIAL` 2026-10-03 (MEMORY track) - Added `Profile::scorecard_row`, `Profile::measurement()` and `Profile::tier_label()`: a local profile naming no row loads and reports `unmeasured` (`an_uncalibrated_local_profile_reports_its_tier_as_unmeasured`). **Not done:** `config/profiles/qwen2.5-7b-instruct.yaml` still carries the `multi_step_evidence.detail` prose and no profile names a row (config is outside this track, and no scorecard row exists for the multi-step eval until a live run produces one, H-2); `validate` still refuses a degraded `multi_step` claim without evidence rather than loading it `unmeasured`, which is deliberate until the row exists. Nobody calls `tier_label` yet (the desk reads `agent/routes.rs`).

Previous marker `PARTIAL`, 2026-09-16, re-audited 2026-09-23: the claim above is false, found by a grep that matched the wrong casing. `config/profiles/qwen2.5-7b-instruct.yaml:62` carries a `multi_step_evidence:` block — the snake_case serde field, not the CamelCase type name the first audit grepped for — whose `detail: >` is hand-written prose about one run (`trials: 10`, `recorded: "2026-09-12"`, one named GPU): exactly what this task says to replace. The other two clauses are also unmet: no profile names a scorecard row, and `profile.rs:743` hard-errors rather than loading with a warning and reporting `unmeasured` — and only for `Tier::Degraded`, so `qwen3.6-35b-a3b.yaml:30`'s `tool_calling: "multi_step"` is unevidenced and unchecked. Pointing each local profile at a scorecard row still needs the calibration table from S-4.12.

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
`BLOCKED:HUMAN` 2026-10-03 (MEMORY track) - Re-checked 2026-10-03; unchanged and exactly what remains: (1) an Anthropic credential in the encrypted store (Settings, LLM credentials); (2) one 30-iteration frontier run and one 30-iteration local run recorded, so `Usage` cache fields are populated (AGENT-005 AC-2: cache_read >= 0.8 x prompt over iterations 3-30); (3) `agent_runs.prompt_tokens` and `est_prompt_tokens` then feed `eval_store::refresh_calibration`, which writes the first `agent_token_calibration` rows. Everything downstream of the credential is built (calibration, replay sampler, scorecard).

Previous marker `BLOCKED:HUMAN`, 2026-09-16 — Not started and **not startable**: it needs a real 30-iteration run against a frontier model, and `eval/scorecard.rs:196` records why in the source itself — "a credential the platform does not have (S-4.14, roadmap gate H-2)". This is a human gate, not work. `NOT STARTED` implied somebody could pick it up.

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
`DONE` 2026-10-03 (MEMORY track) - `conflict::decide_write` is now called by the one door every finding goes through: `MemoryStore::put` delegates to `put_checked`, and `local_driver::remember`, `routes/experiments`, `routes/lifecycle` and the preference route all use `put`. Tests (live Postgres, `a_finding_write_consults_the_conflict_guard`): `an_opposing_finding_links_both_rows_and_downgrades_neither` (both rows `conflicts_with` each other, both stay `established`, both returned by `for_subject` so both enter the brief), `overlapping_interval_disagreement_is_not_flagged`, `rewriting_the_same_id_is_an_upsert_not_a_corroboration`. Link and the second write happen in one transaction. **Mutation-checked 2026-10-03:** passing `decide_write` an empty candidate list failed the conflict and corroboration tests.

Previous marker `PARTIAL`, 2026-09-16, re-audited 2026-09-23: the claim above is false. `memory/conflict.rs` (479 lines, 8 tests) implements the detection including the subtle half — an overlapping interval is not flagged as disagreement. But `crates/api/src/routes/memory.rs` **does** exist and is mounted, and migration 0122 **does** supply `conflicts_with` on `agent_memory.entry`. The real blocker is different and worse: `conflict::decide_write` has zero callers outside its own module, and `relate` likewise — the write path (`Effect::Remember` → `MemoryStore::put`) never consults either. `conflicts_with` is a column nothing will ever populate; a genuine contradiction between two `established` findings goes undetected and both enter the brief as agreeing evidence. Clauses 2 and 3 of the acceptance are met (asserted in `conflict.rs`'s own tests and in `brief.rs:592`); clause 1 fails at the system level, not the unit level.

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
`DONE` 2026-10-04 (S-CATALOGUE track) - Every acceptance clause now holds with CI-run evidence. Archived-leaves-the-brief-but-resolves and near-duplicate corroboration (`a_finding_write_consults_the_conflict_guard`), and the monthly report as a stored artefact: `memory_reaches_the_run::the_monthly_report_is_written_once_per_user_and_month_and_is_immutable` was written and never run; run on a scratch DB 2026-10-04, passes (includes the `app_role` check). The retention sweep and report are spawned by `apps/platform/src/main.rs`.
`PARTIAL` 2026-10-04 (F-LIFECYCLE track) - The retention sweep is spawned from `main.rs`. New: the monthly report is an artefact: migration `0172` `agent_memory.monthly_report` (`SELECT, INSERT` to `app_role`), `MemoryStore::publish_monthly_reports` called by the daily sweep (ADR-S4-06). Test `memory_reaches_the_run::the_monthly_report_is_written_once_per_user_and_month_and_is_immutable` is written (includes a `SET LOCAL ROLE app_role` check) but **not run** (needs live Postgres, `--ignored`).
`PARTIAL` 2026-10-03 (MEMORY track) - Built and tested: near-duplicate corroboration at the write path (`a_near_duplicate_increments_corroboration_rather_than_inserting`: no row for the duplicate, `corroborations = 2`, the writing run in `corroborated_by`); `MemoryStore::sweep_retention` applies `memory::retention::retention` to every entry, archiving by flag only (`the_retention_sweep_archives_a_stale_uncited_finding_but_the_id_still_resolves`: leaves `for_subject`, still resolves through `get`); `MemoryStore::store_report` and `GET /api/memory/report` (size, archive rate, cited share). **Still open:** the sweep only runs when something calls `api::memory_store::spawn_retention_sweep(pg.clone())`, one line in `apps/platform/src/main.rs` (orchestrator); until then nothing archives in production. The report is on-demand, not a scheduled monthly artefact.

Previous marker `PARTIAL`, 2026-09-16, re-checked 2026-09-23 and confirmed correctly `PARTIAL`, with clause 1 now verified rather than assumed pending: `memory/retention.rs` (290 lines, 7 tests) has the archive rule and near-duplicate corroboration, and the "archived but still resolves by id" clause is met and CI-run (`brief.rs:907` `an_archived_entry_is_out_of_retrieval_but_the_id_still_exists`; `MemoryStore::get` carries no `archived` filter while `by_kind`/`for_subject` do). Two concrete gaps remain: near-duplicate corroboration is unreachable — `conflict::decide_write`'s `WriteAction::Corroborate` and `conflict::corroborate` have no caller, and `put` is `ON CONFLICT DO UPDATE` on `entry_id`, so a near-duplicate with a new id inserts instead of corroborating; and no monthly report exists — `RetrievalStore::never_cited` is the only ingredient built, and `memory::retention::retention` itself has no caller, so nothing ever actually archives a row today.

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
`DONE` 2026-10-04 (S-CATALOGUE track) - Every clause holds. The standing draw was written and never run; `eval_state_is_recorded_and_granted::the_standing_draw_is_recorded_once_and_asking_again_changes_nothing` and `a_redraw_is_recorded_with_its_requester_and_refused_before_growth` pass on a scratch DB (5/5 in the file). `spawn_eval_jobs` is called from `main.rs`, the sealed set is excluded from PR runs (`the_set_a_pr_runs_contains_no_sealed_case`) and runs in `.github/workflows/eval-nightly.yml`.
`PARTIAL` 2026-10-04 (F-LIFECYCLE track) - Recorder callers exist: `eval_store::record_standing_draw` records `cases::standard_holdout()` once and is spawned by `spawn_eval_jobs` (`main.rs`); `eval_store` is mounted in `lib.rs`. Test `the_standing_draw_is_recorded_once_and_asking_again_changes_nothing` written, **not run** (live Postgres).
`PARTIAL` 2026-10-03 (MEMORY track) - The sealed set no longer runs on a PR. `cases::standard_holdout`, `open_cases` and `sealed_cases` are what CI iterates: `every_case_passes_on_the_harness_it_was_written_against` runs only the open set through `pr_run`, which panics on a sealed case; `the_set_a_pr_runs_contains_no_sealed_case` asserts it on the real corpus (**mutation-checked:** making `open_cases` return everything failed three tests). Sealed cases run in `nightly_*` tests (`#[ignore]`), via the new `.github/workflows/eval-nightly.yml`. Re-draws: migration `0177` `eval_holdout_draw` (append-only, one row per salt, `harness_version`), `eval_store::record_holdout_draw` refuses a re-draw before 50% corpus growth (`a_redraw_is_recorded_with_its_requester_and_refused_before_growth`). Unstable-suite and fitted-to-open-set clauses were already met (`an_unstable_suite_can_neither_block_nor_clear`, `a_version_fitted_to_the_open_set_fails_promotion`). **Still open:** `eval_store` is not reachable until `pub mod eval_store;` is in `crates/api/src/lib.rs`, and nothing calls `record_holdout_draw` for the standing draw yet.

Previous marker `PARTIAL`, 2026-09-16, re-audited 2026-09-23: the note above blamed only the missing migration, and understated it. `eval/holdout.rs` (258 lines, 7 tests) seals the eval holdout, and `a_single_case_suite_seals_nothing_rather_than_sealing_everything` is the degenerate case handled — clause 1 (`FLAKINESS_CEILING`, `SuiteVerdict::Unstable`, `can_gate`) is met and CI-run. But the acceptance's second clause is actively **violated**, not merely unmigrated: `eval::Holdout` has no caller anywhere — every `Holdout` hit in `crates/` is the unrelated backtest/ledger/dataplane holdout type — and CI's `every_case_passes_on_the_harness_it_was_written_against` iterates `cases::all()` with no seal filter. **The sealed set runs on every PR today**, the opposite of the acceptance. `the_open_set_a_pr_runs_excludes_every_sealed_case` is a unit test over a fabricated corpus, not over the set CI actually runs. `migrations/0073_eval_holdout.sql` still does not exist, so a re-draw also cannot be recorded with the version that asked for it.

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
