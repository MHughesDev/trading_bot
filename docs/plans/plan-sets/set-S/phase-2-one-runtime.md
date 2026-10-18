# Phase 2 — One runtime

**Completion: 44% (7 / 16 tasks), plus 9 partial (S-2.5, S-2.6, S-2.8, S-2.9, S-2.11, S-2.12, S-2.14–S-2.16). Nothing is `NOT STARTED`. (2026-10-03: `driver.rs` is deleted and every model runs on the one loop; what keeps the partials open is named in each task: a live-Postgres run of resume, a live frontier credential for S-2.9 (BLOCKED:HUMAN for that clause only), the sub-run lease caller, and a verified hardware price row.)**

**Requirement IDs:** AR-01…AR-16.
**Spec:** [AGENT-010](../../../specs/AGENT-010-agent-runtime.md), ADR-0035.
**Depends on:** Phase 1 (all three tracks).
**Blocks:** Phase 8. Phase 6 consumes its effect log but can start before it.
**Migration range:** 0061–0064 (0170 used for the resume tables, from the 0170–0172 block this track was allocated).
**Migrations shipped:** 0061, 0062 (as `0061_agent_effect_log.sql` — the task’s `agent_events` name was taken by 0036).
**Parallel with:** Phases 6, 7.

The serial, risky phase, and the reason the set is ordered this way. Three runtimes exist;
one survives. Everything here lands behind `agent.single_loop` and is compared against the
old path on real trajectories before the flag flips, because `driver.rs` is the only hosted
path that has ever worked and the local loop has never carried a frontier model.

**Do not start any task here until Phase 1 track B and C are `COMPLETE`.** Reserving
schemas (S-1.11) and `WorkingState` (S-1.13) are what make the native transport's budget
arithmetic and escalation payload meaningful.

---

## Tasks

### S-2.1 Adapters declare what they can do — M
`DONE` 2026-09-15 — `Capabilities` on `ModelAdapter`, checked by `verify`, and the refusal names the
field.

Every one of the eight was an assumption made at a call site, and each fails as a *model-quality*
problem rather than as a configuration error: a `native` profile on a backend with no tool API
gets prose where it expected a tool call and reads as a model that will not follow instructions;
a `select_fill` profile on a backend that cannot constrain gets malformed JSON at a rate that
looks like quantisation damage; a window larger than the backend's is silently left-truncated and
what goes first is the charter.

`Capabilities::none()` is the default, and every field's `false` is the conservative answer —
`max_context: 0` refuses every profile rather than admitting all of them. A default that admitted
would make the check decoration the first time someone adds an adapter and forgets the method.

`supports_constrained_decoding` is now **derived** from `capabilities().grammar` rather than
answered separately. Two sources of truth for one property is one too many: an adapter could
declare `grammar: false` and return `true`, and `verify` would take whichever it asked first.

One thing the fixtures were saying that is not true: four tests gave a `LocalMid` profile the
frontier fixture's 120,000-token window. No local backend offers that, and a fixture claiming one
would make the `max_context` check pass for a configuration that cannot exist. They now use
24,000, which is `qwen3.6-35b-a3b`'s own setting.

Verified adversarially, as the task asks: `a_profile_needing_a_grammar_is_refused_by_a_backend_without_one`,
`a_profile_decoding_native_is_refused_by_a_backend_without_tool_calls`,
`a_window_larger_than_the_backend_accepts_is_refused_with_both_numbers` (the message carries both
numbers, because "this configuration is impossible" is not actionable and "asks 200000, accepts
32768" is), and `an_adapter_that_declares_nothing_admits_nothing`.

`Capabilities { native_tools, parallel_tools, grammar, prompt_cache, num_ctx_settable,
keep_alive, max_context, reports_usage }` on `ModelAdapter` (`adapter.rs:20`), checked by
`verify` (`adapter.rs:64`) against the profile at load. Implementations for Anthropic,
OpenAI, Ollama (grammar via `format`, and `tools`+`format` do **not** compose —
LOCAL_TIER_FINDINGS §2) and vLLM.

**Files:** `crates/harness/src/adapter.rs`, `crates/llm/src/*.rs`.
**Acceptance:** a profile asking `decode: native` on an adapter without `native_tools`, or
`select_fill` without `grammar`, fails to load naming the field. Adversarial: the refusal
is asserted, not just the happy path.
**Traces:** AGENT-010 §3; §14.9.

### S-2.2 The native transport — M
`DONE` 2026-09-16 — the adapter owns the difference, and the difference is exactly one thing.

`native` keeps a provider transcript for the run (`crates/harness/src/transcript.rs`),
append-only and elided at step boundaries; `select_fill` stays single-turn over the assembled
body. Both are driven by the same effects.

Re-rendering a body on every native call would have thrown away the cache S-1.8's breakpoints
exist to earn — a prefix is reusable only if its bytes do not change — so the transcript is what
makes that task pay rather than a second copy of the same idea.

The transcript is **provider-neutral** and lives in `crates/harness`, which does not depend on
`crates/llm`: the loop is sans-IO, knowing about a wire format is knowing about IO, and the point
of the neutrality is that adding a provider is an impl in the driver rather than a change in the
harness. `transcript_messages` does that mapping.

Tool results rejoin the conversation **paired with the call that asked for them**, and the id
comes from the assistant turn rather than being invented: most providers reject a result whose
`tool_call_id` matches no call in the thread.

The acceptance is asserted as `both_transports_ask_the_world_for_the_same_things`, and what it
excludes is the argument rather than a weakening. The two transports differ in exactly one place
by design — `select_fill` spends two model calls per step where `native` spends one, because a
grammar cannot pin a choice and an object at the same time — so `CallModel` is excluded and
everything else is compared: the tools run, the approvals sought, the outcome reached. Those are
what a guarantee is about, and they are identical.

`decode: native` means the adapter keeps a provider transcript for the run (append-only
messages, the S-1.8 breakpoints); `select_fill` stays single-turn over the assembled body.
Both are driven by the same effects; the adapter owns the difference.

**Files:** `crates/harness/src/{adapter,drive}.rs`, `crates/api/src/agent/local_driver.rs`.
**Acceptance:** a frontier run and a 7B run of the same lookup goal produce trajectories
whose effect *kinds* are identical, differing only in `CallModel` payloads and exposure
size.
**Traces:** AGENT-010 §3, P1; §14.2.

### S-2.3 Two profile fields, and readers that prove they mean something — S
`DONE` 2026-09-15 — both fields, both read, and `search_reserve_slots` changes what is exposed.

`tools.exposure_scope: Session|Step` and `tools.search_reserve_slots`. Frontier is `session`/4,
both locals are `step`/1, exactly as the task specifies.

The reserve is what makes the escape hatch usable. Without it a full slice means `search_tools`
finds the right tool and cannot be given it without evicting something, so using the hatch costs
the tool that prompted it and the model is no better off than before it searched. It is held back
from the **routed fill only, never from `pinned`** — a tool the model explicitly asked for is the
thing the reserve exists to make room for, and reserving a slot against it would be the same bug
in a new place. Capped at a third of the budget, because a reserve larger than that starves the
fill and leaves a step that can only search.

Three existing tests asserted `exposure == max_exposed_per_step` and now come in one (or four)
under. All three were updated to assert **both** halves — the cap is never exceeded, *and* the
reserve is actually held — rather than being relaxed to `<=`. A reserve that quietly stopped being
held would restore the original bug and nothing would notice.

Verified by `every_profile_field_is_enforced_somewhere` (which now reads both fields),
`exposure_never_exceeds_the_profile_budget`, `the_reserve_is_never_held_back_against_a_pinned_tool`
and `a_flooded_catalogue_still_respects_the_exposure_budget`.

`exposure_scope` has a reader in conformance but **does not yet change behaviour** — the
session-scoped, append-only slice is S-2.4, which is the next task and names it.

`tools.exposure_scope: Session|Step` and `tools.search_reserve_slots`. The harness crate's
own rule is that a profile value nothing enforces is a bug, and
`crates/harness/tests/conformance.rs` fails when a field has no reader.

**Files:** `crates/harness/src/profile.rs`, `config/profiles/*.yaml`,
`crates/harness/tests/conformance.rs`.
**Acceptance:** `cargo test -p harness --test conformance` green with a reader for each new
field; frontier = `session`/4, locals = `step`/1.
**Traces:** AGENT-005 §5.1; AGENT-010 §14.9.

### S-2.4 Session-scoped exposure — M
`DONE` 2026-09-15 — routed once per turn, append-only after, and a full slice says so.

The order is the part that is easy to get wrong and expensive to get wrong. A cached prefix
matches by **bytes**, so re-sorting the tools array invalidates it exactly as thoroughly as
removing an entry — a slice that re-rendered whatever routing returned each step would pay the
full prompt on all thirty iterations while looking, from the outside, like caching. So the kept
order is appended to rather than rebuilt, and each call's array is a strict prefix of the next.

Mid-turn eviction is forbidden and a full slice reports `exposure_full` instead. Dropping a tool
to make room for a better one is a trade that looks free at the moment it is made and is paid for
by every later call in the turn.

One exception, and it is not a hole: a tool the S-1.20 breaker opened leaves even a held slice.
The reason it is gone is that it does not work, and holding a broken tool to protect a cache is
protecting the wrong thing.

Verified by `a_session_slice_is_append_only_and_each_order_is_a_prefix_of_the_next` (six steps,
every earlier name still present), `a_searched_tool_joins_the_session_slice_in_one_round_trip`
(the goal never says "skill"; a search for "run python" puts `run_skill` in the slice on the very
next call), `a_full_session_slice_reports_rather_than_evicting` (three searches against a
five-slot budget: the cap holds, nothing that was there leaves, and `exposure_full` is recorded),
and `a_step_scoped_profile_still_reroutes_every_step`.

Routed once per turn from the opening message, then **append-only**: `search_tools` adds,
and only at the cap does the least-recently-used non-core tool get evicted (one cache
rewrite, logged). This is AGENT-005 §5.2 satisfied in `crates/harness` instead of in the
file AGENT-010 deletes.

**Mid-turn eviction is forbidden.** Tools are the first layer of the cached prefix, so
dropping one invalidates the cache for the system prompt and the whole transcript behind it —
on a 30-iteration run that costs more than the missing tool. A full slice returns "the slice
is full for this turn" and records `exposure_full`; eviction happens only at a turn boundary,
where the prefix is rebuilt anyway.

**Files:** `crates/harness/src/registry.rs`, `crates/harness/src/drive.rs`.

**Acceptance:** exposure never exceeds `max_exposed_per_step`; within a turn the tools array
is **append-only** and its order is a strict prefix of the previous call's; a goal that never
mentions "skill" plus a search for "run python" adds the tool in one round-trip; a full slice
reports `exposure_full` rather than evicting.
**Traces:** AGENT-005 §5.2; §10.3.

### S-2.5 `agent_events`: the effect log is the run — L
`PARTIAL` 2026-09-15 — the log exists, is written before every effect, and is proved against live

**2026-10-05 (S-RUNTIME) - PARTIAL.** Run against a migrated Postgres: `pg_runtime_live::a_real_turn_leaves_a_log_that_replays_and_telemetry_that_reads_back` drives a real `AgentManager` turn (fake Anthropic provider, tool call then `finish_task`) and asserts the run's `agent_effect_log` projects to a trajectory for which `harness::trace::is_replayable` is true, with the tool call's arguments present and the `ExecuteTool` effect logged before its answer. **Still missing:** a sample of runs against a real model; one scripted run is not a sample.
Postgres. The dual-write to `agent_messages` is unchanged and the SDK driver is not wired.

**2026-10-03 (RUNTIME track) — PARTIAL.** The SDK driver this task said was "not wired" no longer exists (S-2.10), so there is one writer: `local_driver`. New this
pass: a `state` row carrying the `WorkingState` hash is written in front of **every** input (`Pump::feed`), plus a
`RunSpec` row at admission (everything replay rebuilds the loop from), and `Spend`, `Turn`, `Opening`, `Compact` and
`Suspended` state rows. Replay checks the hash, so a rebuilt loop that took the same actions for a different reason is
refused (`a_log_that_does_not_replay_to_the_same_state_is_refused`). **Still missing:** `harness::trace::is_replayable`
over a sample of *live* runs: it needs real runs against a real model, and none was available to this track. The
write-ahead ordering and the dual-write with `agent_messages` are unchanged.

**The name in this task is already taken.** AGENT-010 §5 and this task both say `agent_events`;
migration **0036** created a table of that name in 2026 — an SSE event stream keyed on
`session_id` that the UI reads to draw a live session. Two tables with one name is not a
collision anyone notices until a query joins the wrong one, so the table is
**`agent_effect_log`** (migration 0061), which is also what it is. Recorded here rather than
substituted silently.

**The ordering is the whole value and it is the part that is easy to get wrong.** A row written
*after* an effect records what happened, which the transcript already does. A row written
*before* records what was about to happen — and that is the only thing that distinguishes a crash
between the decision and the act from a crash after it, which is the only question resume
actually asks. So `rec.effects(&effects)` runs before any of them is executed, and
`rec.input(...)` before the answer is fed back.

Rows are keyed on the **variant name** (`ExecuteTool`, `ModelReply`), not on
`agent_messages`'s presentational kinds. `ExecuteTool` carries its arguments **in full**: they are
what a resume must not run twice, and a hash cannot answer "was this the same call".

Write failures are logged and swallowed, which is the opposite of the audit sink's direction and
deliberately so: the audit trail gates dispatch because a look nobody counted is what this
platform claims it cannot take, whereas a lost effect row costs a *resume* — the run in front of
the user still works. Killing a live run to protect its resumability trades the certain thing for
the contingent one.

Verified against live Postgres by six tests in `crates/api/tests/pg_agent_effect_log.rs`:
`an_effect_is_recorded_before_the_input_that_answers_it`,
`an_interrupted_write_is_visible_as_a_trailing_effect` (the trailing-effect shape S-2.6 reads,
with `idempotent: false` so a resume knows not to re-run it), `the_effect_log_refuses_to_be_edited`
(append-only, both UPDATE and DELETE), `two_events_cannot_share_a_sequence_number`,
`a_row_that_is_neither_a_decision_nor_an_answer_is_refused`, and
`an_event_with_no_harness_version_is_refused`.

**Not done:** `state_snapshot` rows at step boundaries (the `direction = 'state'` case is in the
CHECK and nothing writes one yet), and the **SDK driver is not wired** — only `local_driver.rs`
writes the log. `is_replayable` is about `harness::trace` spans and is unchanged by this task.
**That is why this is `PARTIAL`.**

Replace the untyped `agent_messages(kind, content_json)` with
`agent_events(run_id, seq, kind, payload_json, created_at)` keyed on the `Effect`/`Input`/
`Note` variant names, plus `state_snapshot` rows at each step boundary. Dual-write with
`agent_messages` first so the UI keeps working (AGENT-010 §13 step 1).

**Files:** `migrations/0061_agent_events.sql`, `crates/harness/src/trace.rs`,
`crates/api/src/agent/*.rs`.
**Acceptance:** every effect is persisted **before** it is executed and every resulting input
before it is fed back; `harness::trace::is_replayable` returns true for a sample of live
runs.
**Traces:** AGENT-010 §5, P3; §14.3. AGENT-009 §2.

### S-2.6 Resume from the log — M
`PARTIAL` 2026-09-16 — the decision is built and tested; the replay that acts on it is not wired,

**2026-10-05 (S-RUNTIME) - PARTIAL.** The database half now runs against a migrated Postgres: five `resume_pg_*` tests in `local_driver.rs` (`an_interrupted_write_is_not_repeated_and_the_model_is_told`, `a_write_that_landed_is_answered_from_the_durable_record`, `a_mid_step_run_under_another_harness_is_interrupted_in_the_database`, `an_incompatible_log_attaches_the_job_result`, `a_suspended_run_resumes_on_job_done_across_a_version_change`), covering `load_log`, `interrupt`, the idempotency read and the status writes; the platform fake counts requests and none for `create_experiment` is made. Mutation-checked: ignoring the durable record fails the landed-write test. **Still missing:** no process was actually killed; the log is cut by construction, not by SIGKILL.
and the run is still marked failed.

**2026-10-03 (RUNTIME track) — PARTIAL.** The replay is built and acts. `local_driver::resume` rebuilds the loop from the log (`RunSpec` + inputs, `replay()`),
checking **every batch it produces against the batch the log recorded** and every `WorkingState` hash, then continues:
an outstanding read, model call or approval picks up where it was; an outstanding **write** is answered from the durable
`agent_idempotency` record (migration 0170) or told "interrupted; not retried because it writes", never re-run; a
suspended run resumes on `JobDone` and survives a `harness_version` change when the effect-log schema matches
(`decide_for_resume`, `EFFECT_LOG_SCHEMA`); anything else is marked `interrupted` with its working state kept and the
job's result attached to the timeline. `AgentManager::recover_orphans` now calls it for every orphan instead of failing
them. Evidence: `a_resumed_run_does_not_repeat_an_interrupted_create_experiment` (a log cut after the approved
`create_experiment`; both the no-record and the recorded-result branches issue zero `create_experiment` effects),
`a_log_that_does_not_replay_to_the_same_state_is_refused`, `a_log_ending_on_an_input_replays_to_an_unlogged_batch`,
`every_input_the_pump_writes_reads_back`, and in `agent::resume` `a_suspended_run_resumes_across_a_harness_version_change`,
`a_mid_step_run_under_another_harness_is_still_interrupted`, `a_trailing_state_row_does_not_mask_the_outstanding_effect`.
**Still missing, which is why this is not DONE:** the database half of `resume()` (`load_log`, `spawn_resume`,
`interrupt`, the idempotency read) has not been run against a migrated Postgres, and no process was actually killed:
the "kill the API" acceptance is simulated by replaying a truncated log. The platform DB and a migrated scratch DB were
not available here (pgvector is absent on 5433).

**2026-10-03 wave 2 (RUNTIME).** A run that used `record_finding` or `finish_task` is now replayable from the trace: `Note::CoreTool` carries `args` and `describe_effect` writes them (S-4.8), and the trace projection test `a_core_tool_note_survives_the_round_trip_through_the_log` asserts `is_replayable` where it used to pin the break.

**What exists.** `crates/api/src/agent/resume.rs` decides what a restart should do with one run,
from its effect log alone, as a **pure function over rows**. That is where the logic belongs: it
is what stands between a restart and a duplicated write, and a pure function can be tested
against the exact shapes that matter without a process to kill.

Three cases, and the third is the one the whole mechanism exists for:

* the log ends on an **input** — the loop had the world's answer and was about to think; replaying
  the inputs rebuilds exactly that;
* it ends on an **idempotent effect** — decided, possibly done, safe either way; re-run it;
* it ends on a **non-idempotent effect** — decided, and *unknowable* whether it happened.

The third is not retried. It cannot be known from here whether the write landed, and re-running
`create_experiment` risks a second experiment — which is a second counted trial, and that corrupts
the one number every selection-adjusted statistic on this platform depends on. The run resumes
with a `ToolError` the model reads, saying the call was interrupted and not retried **because it
writes**. The model then checks, or reports that it cannot. Strictly better than both
alternatives: failing loses the work, retrying silently corrupts the evidence.

An effect whose `idempotent` was never recorded is treated as a write. Being wrong that way costs
one message to the model; being wrong the other way costs a duplicated write nobody can see.

A version mismatch **interrupts with the working state preserved** rather than replaying. A run
that looks resumed and is not is worse than one that says it stopped.

**What does not exist, and the run is still failed on restart.** Replaying inputs through a fresh
`Loop` needs the driver to accept a pre-seeded loop, which it does not. A recovery that half-works
is worse than one that says it did not, so `recover_orphans` now *reports* what the log says about
each orphan — including how many ended on an interrupted write — and then marks them failed as
before. That diagnosis is worth having on its own: it is how anyone finds out whether finishing
this task is worth the work.

The **suspended-run** half (a resume contract carrying the effect-log schema version, surviving a
`harness_version` change, the job result attached even on an incompatible schema) is not started.

Verified by eight unit tests in `agent::resume` — including
`an_interrupted_write_is_not_retried`, `an_effect_with_no_recorded_safety_is_treated_as_a_write`,
`a_trailing_note_does_not_mask_the_outstanding_effect` and
`a_log_from_another_harness_interrupts_rather_than_pretending` — and by
`the_resume_decision_reads_an_interrupted_write_out_of_the_database`, which covers the seam the
unit tests cannot: that `idempotent` survives the JSONB round trip. If it did not, every
interrupted write would look retryable and nothing else would notice.

**That is why this is `PARTIAL`.**

On restart, a `running` run replays its inputs through a fresh `Loop` of the same
`harness_version`, then continues. An `ExecuteTool` with no recorded result is re-executed
only if the tool is idempotent (derived from `Risk::Read`); otherwise it becomes a
`ToolError("interrupted; not retried because it writes")` the model sees. A version
mismatch marks the run `interrupted` with the working state preserved.

A **suspended** run is the exception: it holds no process, so a deploy inside a 40-minute
sweep is likely rather than exceptional. `Effect::Await` writes a resume contract carrying the
effect-log *schema* version, which changes far less often than `harness_version`; a compatible
schema resumes across a version change. Only an incompatible one interrupts, and even then the
job result is attached to the conversation as an artifact so the compute is never lost.

**Files:** `crates/api/src/agent/driver.rs`, `crates/harness/src/drive.rs`.

**Acceptance:** killing the API mid-run and restarting resumes with **no duplicated write
effect**, asserted on a run containing `create_experiment`; a suspended run survives a
`harness_version` change and resumes on `JobDone`; an incompatible schema still attaches the
job result.
**Traces:** AGENT-010 §5; §14.3.

### S-2.7 Every adapter through the proxy — S
`DONE` 2026-09-16 — the three behaviours now apply to every caller; they are shared code rather
than a shared HTTP hop.

**2026-10-03 (RUNTIME track) — DONE.** S-2.10 deleted the hosted driver and the SDK container no longer hosts the agent, so there is no second internal path
left to collapse: the one driver applies the shared 1-hour cache TTL (`crates/llm`), reads cache usage into
`agent_runs`, and refuses an over-budget request with `context_budget_exceeded` before sending it
(`call_model`, via `llm_proxy::estimate_request_tokens`). The standing decision is unchanged: the policy is one shared
function, not an HTTP hop of the platform to itself. Evidence: `every_breakpoint_lives_an_hour` (llm) and S-1.9's cache
usage tests; the refusal itself is exercised only by the budget arithmetic tests, not by a provider call.

The task says "route all of them through the proxy". The proxy is an HTTP endpoint, and routing
the internal drivers through it would mean the platform making HTTP calls to itself for every
model call — a round trip, a serialisation, and a second place for a request to be malformed, to
buy something a shared function buys for nothing. What the task is actually asking for is that
the policy exist **once**, and that is what this does.

* **The 1-hour cache TTL** moved to `crates/llm/src/anthropic.rs`, where the breakpoints are set,
  with the `extended-cache-ttl-2025-04-11` beta beside it. Every caller gets it now, which the
  proxy could not do: the proxy only sees what the container's CLI sends, and the internal
  drivers do not pass through it. A five-minute TTL re-pays for the same prefix every step, and a
  step that waits on a tool can easily exceed five minutes — so S-1.8's breakpoints were being
  placed and then expiring before they paid.
* **Usage with cache fields** reaches the run end to end as of S-1.9, and lands in
  `agent_runs.prompt_tokens` / `cache_read_tokens` as of S-2.8.
* **The budget refusal** now fires on the internal path too, using the proxy's own
  `estimate_request_tokens`. With S-1.10 and S-1.11 in place it should never fire — the assembler
  budgets the body against the window and `ask` sends what it budgeted — so reaching it means the
  arithmetic and what is sent have drifted apart. **It is a regression alarm, not a safety net,**
  and it refuses rather than truncating for exactly that reason: a silent truncation would hide
  the drift and take the charter with it.

Verified by `every_breakpoint_lives_an_hour`, and by S-1.9's cache-usage tests.

**Not done:** the container still reaches the proxy over HTTP and the internal drivers do not, so
there are two code paths even though there is now one policy. Collapsing them is S-2.10's
territory. **That is why this is `PARTIAL`.**

The internal drivers call providers directly today; only the container goes through
`crates/api/src/llm_proxy.rs`. Route all of them so the 1-hour cache TTL rewrite, the usage
recording and the budget refusal apply once. With S-1.10/S-1.11 in place the proxy's
`context_budget_exceeded` should never fire — it becomes the regression alarm.

**Files:** `crates/api/src/llm_proxy.rs`, `crates/api/src/agent/manager.rs`,
`crates/llm/src/lib.rs`.
**Acceptance:** a run's `Usage` carries cache fields end to end; an over-budget request is
refused with `context_budget_exceeded` rather than truncated.
**Traces:** AGENT-010 §3; AGENT-005 §6.1, §7.

### S-2.8 Columns the later phases read — S
`PARTIAL` 2026-09-16 — eight of the ten are written by the local driver; `trials_spent` and

**2026-10-05 (S-RUNTIME) - PARTIAL.** The live-Postgres check exists and passes: the same `pg_runtime_live` turn asserts `harness_version`, `profile_name`, `adapter`, `transport`, `outcome`, `prompt_tokens`, `est_prompt_tokens`, `cache_read_tokens` and `trials_spent` (0, not null, for a read) are written by the driver, `prompt_tokens >= tokens_in`, and `n_eff_after` was written (1.0). **Still missing:** `est_prompt_tokens` against a provider's own count (the fake provider reports its own usage), and a run that spends trials.
`n_eff_after` have columns and no writer.

**2026-10-03 (RUNTIME track) — PARTIAL.** The tenth column now has a writer: `Recorder::finish` reads the tenant's N_eff from the ledger (`n_eff_async`, the same
computation the gate worker uses) and writes `n_eff_after`, leaving it null when the ledger cannot be read (an invented
N_eff is worse than none). The SDK driver, which wrote none of the ten, is gone. **Not verified:** a live-Postgres run
asserting all ten populate (`a_finished_run_carries_its_telemetry` is `#[ignore]` and needs the platform DB); only the
compile and the unit paths ran.

Migration 0062. None of these is a reporting column — each answers a question that cannot be
answered any other way once the run is over:

* **`prompt_tokens` is not `tokens_in`.** With caching on a 40,000-token prompt of which 39,000
  was a hit bills 1,000. `tokens_in` answers "what did this cost" and cannot answer "how big was
  the prompt", and a reader who conflates them believes a budget has headroom it does not have.
  The live test asserts they differ.
* **`est_prompt_tokens` is the other half of S-1.18's ratio.** The estimator's calibration is
  `est / actual`, and neither number survives the run without a column. Accumulated **per call**
  rather than as a run total, because a total hides a systematic under-estimate on long steps
  behind an over-estimate on short ones.
* **`profile_name` beside `profile_id`**, which are not the same thing: the id is what the loader
  keys on, the name is what a person reads, and an id renamed underneath silently changes what a
  historical row meant.
* **`transport`** is the decode style, which is what actually differs between two runs of the
  same model — `select_fill` is two constrained calls per step, `native` is one tool call — and
  an eval that averages them is measuring the mixture rather than either.
* **`outcome`** is the loop's own word, deliberately not the UI's `status`, which collapses
  `Refused` and `Escalated` into "refused" because the timeline draws them the same. An eval that
  cannot tell an escalation from a refusal is measuring the wrong thing: one is the run declining,
  the other is the run asking for a bigger model.

All ten are nullable. A run that crashed before filling them is still a run, and NOT NULL would
mean refusing to record the crash or inventing a value — and an invented `trials_spent = 0` is
worse than a null, because a null cannot be averaged by accident.

Verified by `a_finished_run_carries_its_telemetry` (which asserts `prompt_tokens > tokens_in`
rather than just that both exist), `a_run_that_never_finished_leaves_its_telemetry_null` and
`a_negative_token_count_is_refused`.

**`trials_spent` is now written** (2026-09-16), counted from the catalogue's own `spends_trial`
declaration rather than from a second counter — the declaration is the single place a tool says
whether calling it registers a trial (AGENT-006 §2.5), and a parallel count would be a second
thing to keep in agreement with it.

Counted at **dispatch, not at success**. A trial is registered before the tool runs, so a call
that failed still spent one; counting successes would report a run as having searched less than it
did, and under-reporting trials inflates every selection-adjusted statistic that divides by them.
That is the direction that flatters, which is why
`the_tools_that_spend_trials_say_so_and_the_ones_that_do_not_say_that` asserts both halves —
`backtest_strategy` is a `Search`, `report_kill` is a `Falsification`, `get_bars` is neither —
and `a_harness_owned_tool_is_not_in_the_catalogue_and_spends_nothing` holds the other side.

**Still not done:** `n_eff_after` has a column, a CHECK and an index, and **nothing writes it** —
N_eff lives in `crates/ledger` behind an API the driver does not call, and inventing a number for
it would be worse than the null. Nine of ten. The SDK driver writes none of them. **That is why
this is still `PARTIAL`.**

`agent_runs` gains `harness_version, profile_name, adapter, transport, prompt_tokens,
est_prompt_tokens, cache_read_tokens, trials_spent, n_eff_after, outcome`.

**Files:** `migrations/0062_agent_run_telemetry.sql`, `crates/api/src/agent/*.rs`.
**Acceptance:** every finished run populates all ten; `est_prompt_tokens` is the harness
estimate of the same request the provider counted.
**Traces:** AGENT-010 §4; AGENT-005 §7; AGENT-009 §5.

### S-2.9 Flag the frontier onto the loop, and prove it before flipping — M
`PARTIAL`

**2026-10-05 (S-RUNTIME) - PARTIAL (BLOCKED:HUMAN for the live-credential clause only).** A frontier-profile turn now completes end to end through the real manager and loop against a recorded/fake Anthropic transport (tool call, result, `finish_task`), in `pg_runtime_live`. The live frontier run and the day-of-trajectories shadow comparison still need credential H-2.

**2026-10-03 (RUNTIME track) — PARTIAL.** **ADR row.** The task asks for an `agent.single_loop` flag with `driver.rs` kept as the fallback for one release; S-2.10
asks for `driver.rs` to be deleted and `manager.rs` to have no tier branch. Both cannot hold. Chosen: **no flag, one
loop for every model** (alternative rejected: the flag, which would keep a second, un-budgeted, un-resumable loop alive
on the exact path the platform's most expensive runs take, to guard against a regression that the native transport's
loop-level tests already cover). Hosted models with no shipped profile run under the shipped frontier profile with their
model id swapped in (`AgentManager::profile_for`) and the run records that it did; an unknown *local* model still falls
back conservatively or is refused. Proved before flipping with the recorded/fake inputs the platform has: the native
transport against the loop (`both_transports_ask_the_world_for_the_same_things`, the new `runtime_tests`), replay of a
frontier-profile run (`a_resumed_run_does_not_repeat_an_interrupted_create_experiment` runs under `claude-opus-5`), and
the pump's native thread handling (`prepare_native`: unanswered calls answered, rejections and directives delivered as
turns). **BLOCKED:HUMAN for exactly one clause:** "a frontier research turn completes end to end" and the day-of-real-
trajectories shadow comparison need a live frontier credential (H-2) and S-4.8's shadow replay.

`agent.single_loop` runs frontier profiles on the harness loop with the `native` transport.
Keep `driver.rs` as the fallback for one release. Use the shadow replay (S-4.8) to compare
outcomes on real trajectories; a blocking difference needs a human's note before the flag
flips.

**Files:** `crates/api/src/agent/manager.rs`, `config/default.toml`.
**Acceptance:** with the flag on, a frontier research turn completes end to end; the shadow
comparison over a day of real trajectories shows no missing-tool or different-verdict
differences.
**Traces:** AGENT-010 §13 step 3; MASTER §8 risks.

### S-2.10 Delete `driver.rs` — M
`DONE`

**2026-10-03 (RUNTIME track) — DONE.** `crates/api/src/agent/driver.rs` is deleted. `manager.rs` always builds `LocalRunParams` and calls
`local_driver::run`; its tier branch is gone (`profile_for` picks a profile, not a path). Workspace `fs` tools are
registered on every tier (`bridge::registry_with_workspace`), the canary-or-adapter-verify runs for every tier, and
elision always leaves a `work/transcript/...` handle that the driver writes before it can be needed
(`result_handle`). `truncate_result` moved to `local_driver` (the auditor suite still reaches it through
`agent::driver`, a one-line alias in `agent/mod.rs`, because `crates/api/src/auditor.rs` is outside this track).
Evidence: `the_agent_module_contains_one_driver`, `the_manager_does_not_branch_on_tier`,
`no_runtime_code_writes_an_unrecoverable_elision`, and the paused-run guard now names the single driver.

The hosted driver goes. `manager.rs` loses its tier branch. Workspace `fs` tools are
registered on every tier (`bridge::registry_with_workspace` when a conversation workspace
exists), and elision always leaves a `work/…` handle instead of
`"[elided — was N bytes]"`, which is unrecoverable and forbidden by AGENT-005 P6.

**Files:** delete `crates/api/src/agent/driver.rs`; `crates/api/src/agent/{manager,bridge,
local_driver}.rs`.
**Acceptance:** `crates/api/src/agent` contains one driver; `manager.rs` has no tier branch;
no transcript anywhere can read `[elided` without a handle when a workspace exists.
**Traces:** AGENT-010 §2, §13 step 4; §14.1. AGENT-005 §6.3; §10.6.

### S-2.11 Waiting costs nothing — M
`PARTIAL` 2026-09-16 — the loop half is built and proved; the two driver polling loops stay until

**2026-10-05 (S-RUNTIME) - PARTIAL.** Built and run against a migrated scratch Postgres (every migration, pgvector columns substituted). `train_model` and `run_study` now map to job ids: `local_driver::long_job_handle` yields `training:<model>:<run>` for a still-running `train_model` (driver sets `timeout_seconds` 30, the tool's floor) and `study:<exp>:<study>` for a detached `run_study`; `manager::job_status` polls both (`study_entry`, `snapshot_outcome`); `POST .../studies` accepts `detach: true` (202 at once, in-memory registry, `detached` on the list read; a restart loses a running study and the watcher reports it ended, not succeeded). Evidence: `a_long_tool_still_running_names_the_job_the_watcher_polls`, `a_study_and_a_training_run_resume_their_run_when_they_end`, and live `resume_pg_a_suspended_run_resumes_on_job_done_across_a_version_change` (still-waiting spends zero model calls; resumes on JobDone across a harness-version change). **Still missing:** a real sweep/study/training run end to end, and the in-memory study registry does not survive a restart (the run is told the study ended, not that it succeeded). ADR: rejected making the study route always async (breaks every caller reading the 201 body).
the resume path can replace them.

**2026-10-03 (RUNTIME track) — PARTIAL.** The driver half is done and wired. `run_sweep` and `wait_for_backtest` are dispatched through the ordinary audited
catalogue path with `timeout_seconds` set so they never wait; a call that is still running returns a handle, the loop
suspends (`Effect::Await`), `local_driver` writes the resume contract (`Suspended` state row with the job id and the
effect-log schema), marks the run `waiting_backtest`, and **exits**. `AgentManager`'s job watcher resumes it with
`Input::JobDone` when the platform says the job ended (one cheap read per suspended run per 15 s; no model call, no
connection, zero tokens). The two in-driver polling loops died with `driver.rs`. Honest note on the watcher: sweeps live
in the research manager's memory rather than in `crates/jobs`, so there is no completion event to subscribe to and the
watcher polls the platform's own API; what the spec forbids is a *run* polling, and none does. **Not verified:** the
suspend-then-resume path end to end against a database (same gap as S-2.6); `train_model` and `run_study` return their
own handles and are not yet mapped to a job id.

`crates/harness/tests/waiting_costs_nothing.rs` asserts the thing that actually matters, and
asserts it the only way it can be asserted: a run suspended on `Effect::Await` **emits no model
call at all**. A polling loop produces the right answer too — "the answer arrives" is not the
acceptance, "no tokens are spent while waiting" is — and the only proof of the second is the
absence of the effect. Six tests, including that nothing short of the job finishing restarts the
run and that another job's result does not resume this one.

`local_driver` exits on `Await` rather than polling, and says so plainly.

**Not done, and not deletable yet.** `driver.rs`'s two polling loops (`:400`, `:462`) are what the
task says to remove, and removing them now would break waiting outright: the replacement is the
manager resuming from the effect log on `JobDone`, which needs S-2.6's replay, which is not
wired. A deletion that leaves nothing behind is not a refactor. **That is why this is `PARTIAL`.**

`run_sweep`, `run_study`, `train_model`, `run_shadow_session` and any workflow containing
them return a `job_id`; the loop emits `Effect::Await{job_id}` and the driver **exits**. The
job service's completion posts `Input::JobDone` and the manager resumes from the effect log.
Delete the in-driver polling loops (`driver.rs:334`, `:408`) and their 30-second status rows.

**Files:** `crates/harness/src/drive.rs`, `crates/api/src/agent/driver.rs`, `crates/jobs`.
**Acceptance:** a run waiting on a 40-minute sweep holds no model connection and adds **zero
tokens** until the job completes.
**Traces:** AGENT-010 §8, P5; §14.4.

### S-2.12 Budgets, watchdog, approvals, steering, escalation, sub-runs — L
`PARTIAL` 2026-09-16 — the wall clock is gone and the steer half landed with S-2.14; the budget,

**2026-10-05 (S-RUNTIME) - PARTIAL.** Migration `0235_sub_run_effect_log.sql` adds `agent_lifecycle.sub_run_effect` (append-only trigger, `GRANT SELECT, INSERT` to app_role). `subrun::record_delegation` writes the Delegation on both sides at claim (a `state/Delegation` row in the parent's `agent_effect_log`, projected by `trace_from_effects` to `Event::Delegation`, and the first row of the child's own log, same brief digest); `research_run_worker` logs CallModel/ModelReply/Settled. Running it against Postgres found two production bugs in the claim path, now fixed: `ON CONFLICT (campaign_id, phase, seq)` did not match the partial unique index (needs `WHERE campaign_id IS NOT NULL`, so every fresh claim errored) and `SUM(lease_*)` decoded as NUMERIC into i64. Evidence: `crates/api/tests/pg_subrun_log.rs` (4 live: `a_claim_leases_from_the_parent_and_records_the_delegation_on_both_sides`, `leases_handed_out_never_exceed_what_the_parent_had`, `a_parent_with_no_recorded_cap_hands_out_no_lease_and_never_a_zero`, `the_sub_run_log_is_append_only_and_granted_to_the_application_role` incl. `SET LOCAL ROLE app_role`); mutation-checked (parent Delegation row written under another kind: claim test fails). **Still missing:** the worker is a single constrained call, not a full loop with its own `Loop` effect log, so the child log holds only the Delegation, the model call, the reply and the settlement; the worker's lease consumption is unchanged.
approval, escalation and sub-run halves are not started.

**2026-10-03 (RUNTIME track) — PARTIAL.** Done: **budgets** (`Loop::record_spend`: the driver reports each call's tokens and, only when `config/prices.yaml` has a
fresh verified row, its cost in integer tenths of a cent; reaching 90% of `max_tokens_per_task` or the dollar cap wraps
up, one report turn, then `Finished { budget_exhausted: true }`; a missed report turn is a `BudgetExhausted` fence;
`a_spent_token_budget_buys_one_report_turn_and_says_so`, `a_spent_dollar_budget_wraps_up_and_an_unpriced_call_poisons_the_total`,
`without_a_fresh_price_the_dollar_cap_is_not_enforced`, and `adapter::` price tests incl. `the_shipped_price_table_loads`);
the **idle watchdog** (`idle_timeout_s` now has an enforcement point: a model call with no reply inside it becomes
`ModelError{stalled}`; suspended waits never reach it); **`AskHuman` on every tier over an event subscription**
(migration 0170's `NOTIFY agent_approval` wakes the wait, a 30 s backstop read catches a missed notification, the row is
reused across a resume, and the pause is audited before and after, which also closes the "no `audit_event` written"
item); **escalation with state** (`Stuck` on a profile with `escalate_to` ends `Outcome::Escalated{state}`; the manager
starts the target with `Loop::resuming_from`, completed steps stay done and trials count once;
`a_stuck_tier_with_somewhere_to_go_escalates_carrying_its_state`, `a_resumed_run_does_not_redo_completed_steps`); the
**steer** half (S-2.14). A non-`Clone` **`BudgetLease`** (`harness::subrun`) exists and `Loop::with_budget_lease`
consumes it, narrowing steps/tokens/trials and never widening past the profile. **Missing:** the lease has no caller
(sub-runs are launched by `campaign_driver`, outside this track), there is no `budget_leases` table, so no sub-run
effect log or `Delegation` pairing is persisted, and the dollar cap cannot fire for local models until a hardware price
row is verified (there is none).

**2026-10-03 wave 2 (RUNTIME).** `BudgetLease` now has a caller: `agent::subrun::claim` (the one place a worker sub-run is claimed, reached by `campaign_driver` at HYPOTHESIZE) splits a lease from the parent run's remaining budget (its `RunSpec` cap, minus `agent_runs` spend, minus leases already handed out) **in the same transaction as the claim, under an advisory lock on the parent**, and persists it in migration 0171's `sub_run.lease_*` columns. An unfunded parent refuses the claim (`BriefError::Unfunded`) and leaves no row; a parent with no recorded cap gets a NULL lease, never a zero. `Claim` is no longer `Clone`. Also `Loop::with_trial_cap` / `with_budget_lease` (a lease narrows, never widens). **Still missing:** the worker's own loop does not yet consume the lease (that is `research_run_worker`, outside this track), settled leases are conservatively still counted in full, and the claim path has no live-Postgres test (the migration alone was validated in a scratch DB).

**No wall clock — and, corrected 2026-09-27, nothing replaced it.**
`orchestration.idle_timeout_s` has **no enforcement point anywhere in the
workspace**. Its only reader is `tests/conformance.rs:364`, which asserts
`idle_timeout_s >= 60` — a *range* check over a field nothing acts on, which is
exactly the shape S-10.6's own note calls out as passing while nothing read the
value. **A run today has no timeout of any kind except `max_steps`.**

The watchdog cannot live in the loop: `drive.rs` is sans-IO and
`the_loop_names_no_runtime_no_client_and_no_provider` forbids `SystemTime`, and a
run hung inside a model call or a tool call is not calling the loop at all. It
belongs in the driver, which is `crates/api`.

The original note read as follows, and the "replaced by" was the false part.
`budgets.max_wall_clock_s` is deleted from every shipped profile and ignored
by the loop, nominally in favour of `orchestration.idle_timeout_s` (600 s). The reason it had to go is that
S-2.11 made the common case unmeasurable by it: a run suspended on a forty-minute sweep has been
*alive* for forty minutes and *idle* for none of them, and a wall clock cannot tell the two
apart. Killing it at the hour mark destroys work proceeding exactly as designed, and raising the
limit to accommodate it means a genuinely hung run takes four hours to notice. Idleness answers
the question the wall clock was being asked, and can answer it correctly, because a suspended
wait is not idle time.

The field is kept on the struct, `#[serde(default)]`, ignored. Removing it outright would fail
every deployment carrying a config written before today, for a field nothing reads. Conformance
asserts it is **zero** on every shipped profile rather than merely unused, so re-adding one is a
test failure rather than a quiet regression.

**`Input::Steer`** landed with S-2.14, which is where the classification belonged.

**Not started:** pricing from `config/prices.yaml` and the 90%-of-budget wrap-up; `AskHuman` over
an event subscription instead of the 5-second DB poll; `Outcome::Escalated` carrying the
`WorkingState`; and the sub-run `BudgetLease` with migration 0063. Four of the six. **That is why
this is `PARTIAL`.**

The loop reads `max_tokens_per_task` and `max_usd_per_task` (priced from a new
`config/prices.yaml`), wrapping up at 90% and ending after the report turn. **No wall
clock** — delete `max_wall_clock_s` from the profiles and replace it with an idle watchdog
(`orchestration.idle_timeout_s`, default 600 s, excluding suspended waits). `AskHuman` is
answered by the UI on every tier via an `agent_events` subscription rather than the 5-second
DB poll. A message sent to a running turn becomes `Input::Steer` at the next step boundary,
replacing the hosted driver's ad-hoc nudges. `Outcome::Escalated` carries the
`WorkingState`. Sub-runs get a non-`Clone` `BudgetLease`, depth ≤ 1, their own effect log
and a `Delegation` trace on both sides.

**Files:** `crates/harness/src/{drive,profile}.rs`, `crates/api/src/agent/*.rs`,
`config/profiles/*.yaml`, `config/prices.yaml`, `migrations/0063_budget_leases.sql`.
**Acceptance:** `max_usd_per_task` reached → one report turn then
`Finished{budget_exhausted: true}`; no profile contains `max_wall_clock_s`; `AskHuman` on a
`SealedHoldout` action suspends a frontier run exactly as it suspends a local one;
escalation from `qwen2.5-7b` to `claude-opus-5` resumes with the working state and does not
re-run completed steps (verified by trial count).
**Traces:** AGENT-010 §6, §7, §9; §14.5, §14.6, §14.7. AGENT-008 §4.

### S-2.13 The native transcript is managed, not merely appended — M
`DONE` 2026-09-16 — landed with S-2.2, because shipping S-2.2 without it would have been the
regression this task names.

An append-only transcript grows without bound and a thirty-step run exceeds any window: *a
regression hidden inside an improvement*, exactly as written. So the transcript appends **and**
elides, under four rules, each because the obvious alternative is wrong:

1. **The opening is never elided.** It carries the goal, and a transcript that dropped it would
   have the model working on whatever the most recent tool result implies — drift with extra
   steps.
2. **Tool results before assistant prose.** A result is reproducible (call it again) and the
   model's reasoning is not, so taking the cheaper one first is the only ordering that loses less.
3. **The last three results stay.** The model is usually acting on what it just read, and eliding
   that is how a run forgets the answer it was about to use.
4. **Everything elided leaves a `work/…` handle.** AGENT-005 P6 forbids `[elided — was N bytes]`:
   a placeholder that cannot be fetched back is a permanent hole the model cannot tell from a tool
   that returned nothing.

A tool **call** is never elided, only the prose beside it — a transcript missing a call leaves its
result answering nothing, and providers reject that outright.

Elision happens at **step boundaries**, because moving a cache breakpoint invalidates everything
after it: once per step costs one re-read, per call costs the cache entirely. And it emits the
same `CompactionReport` as the block path, so one metric covers both transports — two compaction
metrics is two numbers that disagree with no way to say which is real.

Verified by eight tests, including `a_two_hundred_step_transcript_stays_under_budget` (the
regression itself), `the_opening_message_is_never_elided`,
`a_reproducible_result_is_elided_before_irreproducible_reasoning`,
`the_most_recent_results_are_protected`, `every_elided_body_says_where_to_read_the_rest`,
`a_tool_call_is_never_elided_only_the_prose_around_it` and
`a_transcript_that_fits_is_left_alone` — compaction that ran anyway would move the breakpoint for
nothing, which costs the whole prefix.

One thing worth recording about those tests: they were first written against a 3,000-token window
and **passed for the wrong reason.** The frontier profile reserves 16,000 for output, so any
window below that leaves an input budget of zero and every test passes by eliding everything. The
fixture now leaves a small but positive budget, which is what forces the choices these rules are
about.

`driver.rs` held the only transcript elision on the hosted path and S-2.10 deletes it. Without
this the native transport grows an unbounded provider-side transcript — a regression hidden
inside an improvement. The context manager sizes it with the calibrated estimator, elides
oldest-first (tool results before assistant prose, never the opening message, never the last
few results), leaves a `work/…` handle for everything cut, and batches elision at step
boundaries so the moved cache breakpoint is paid once rather than per call.

**Files:** `crates/harness/src/context.rs`, `crates/harness/src/adapter.rs`.
**Acceptance:** a 200-step native run's transcript stays under budget; every elided body has a
readable handle; elision emits the same `CompactionReport` as the block path, so one metric
covers both transports.
**Traces:** AGENT-005 §6.4; §10.6.

### S-2.14 Steering that changes the goal; one live turn per conversation — M
`PARTIAL` 2026-09-16 — both halves are built; nothing delivers a steer to a running loop yet.

**2026-10-05 (S-RUNTIME) - PARTIAL.** Route to loop verified against Postgres: `pg_runtime_live::a_second_turn_names_the_live_run_and_a_steer_reaches_it` (a plain second turn is refused naming the live run; a `steer: true` turn returns the live run's id and the log gains a `Steer` input). **Still missing:** the HTTP route itself under a running turn (the manager is called directly).

**2026-10-03 (RUNTIME track) — PARTIAL.** Both halves now meet: `POST /api/agent/conversations/{id}/messages` accepts `steer: true` (plus optional `mode`,
`workflow`; `{text}` alone is unchanged), `AgentManager::send_turn` queues the text on the live run's `RunControl`, and
the pump delivers it as `Input::Steer` at the next step boundary before a model call (a native model is also handed it
as a thread turn). A redirection on a planner-executor profile now re-plans for the new question
(`a_redirection_on_a_planner_profile_asks_for_a_new_plan`). Evidence:
`steers_queue_in_order_and_a_stopping_run_refuses_them`,
`the_message_body_accepts_text_alone_and_the_optional_fields`. **Not verified:** the route end to end with a running
turn; a steer for a run this process is not driving (suspended or on another replica) is refused with a message saying
so rather than queued.

**Steering.** `state::Steer::classify` reads a mid-run message deterministically, before any
model call — a steer that needed a model call to interpret would cost a step to read and could
be misread, and what is being interpreted is a sentence the user just typed.

Ambiguity resolves to `Redirection`, and the asymmetry is the whole design: an unnecessary
re-plan costs one step, a missed one costs a run aimed at the wrong question. `Comment` is
therefore the narrow case and has to be *recognised* — treating "looking good" as a redirection
would re-plan a healthy run. The model is told which reading was taken, so a wrong call is one
sentence to correct rather than something to discover at the end.

A redirection **keeps the findings and drops the plan**, and that split is the point. A finding
was true about the *data*, and changing the question does not make it false; throwing findings
away on every steer would teach the user not to steer. The plan is the opposite — it was built
for the old question and every remaining step serves it. The old goal and the reason go into
`decided`, so the run can say what it was doing and why it stopped, and so that re-asking the old
question is answered by §3.6's correction rather than by starting over.

A steer is accepted in **any non-terminal state**. One that only landed between steps would
arrive after the step it was meant to stop.

**One live turn per conversation** was already enforced at the manager; what was missing is that
the rejection now **names the run**. All three things a caller can do about it — watch it, stop
it, or decide it is stale — need the id, and "this conversation is still working" gives them
none of them.

Verified by `a_redirection_replans_and_keeps_what_was_established`,
`a_constraint_narrows_without_dropping_the_plan`,
`an_ambiguous_steer_is_read_as_a_redirection_and_says_so` and
`encouragement_is_not_a_new_goal`.

**Not done:** no route or driver path delivers `Input::Steer` to a running loop. The loop accepts
it, classifies it correctly and acts on it; nothing sends one. **That is why this is `PARTIAL`.**

A steer is classified deterministically as Constraint (narrow, keep the plan), Redirection
(re-classify, rewrite acceptance, skip remaining steps with a reason, re-plan — **keeping
findings and evidence**, which were true about the data, not the goal) or Comment. Ambiguous
steers are treated as Redirection and the model is told so: an unnecessary re-plan costs one
step, a missed one costs a run aimed at the wrong question.

Separately and in the same place: **exactly one live turn per conversation**, enforced at the
manager. A second is rejected with the running turn's id, because two runs sharing one
conversation workspace would race on files with no lock and two working states for one goal
is not something the desk or the turn summary can render.

**Files:** `crates/harness/src/{drive,state}.rs`, `crates/api/src/agent/manager.rs`.
**Acceptance:** a redirection steer re-plans and preserves findings; the old goal and the
reason survive in `decided`; a second concurrent turn is rejected naming the live run.
**Traces:** AGENT-008 §3.4a; AGENT-010 §4.1.

### S-2.15 Draining: a deploy with live runs — M
`PARTIAL` 2026-09-16 — the platform drains on a signal; the monitor is not told.

**2026-10-05 (S-RUNTIME) - PARTIAL.** The monitor half is verified: `pg_runtime_live::a_drain_opens_a_window_the_monitor_reads_and_honours` drains a manager, reads the window back through `MonitorStore::open_windows`, and asserts `monitor::platform::assess` returns `Suppressed` with it and `Halt` without. **Still missing:** a real SIGTERM against a running platform (the binary must not be started here).

**2026-10-03 (RUNTIME track) — PARTIAL.** Two gaps closed. A drain now **detaches** every in-flight run at its next step boundary, still `running` with its next
effect already in the log, so the next boot resumes it (S-2.6) instead of failing it; and it opens a COMP-007
maintenance window (`MaintenanceWindow::open("platform:deploy", ...)`, 15 min, written through `MonitorStore`) so a
rolling deploy is a recorded window rather than a heartbeat loss. Also new: `AgentManager::drain_run` /
`resume_drained` for one run (session children). **Not verified:** a real SIGTERM against a running platform, and the
window being honoured by the monitor service (the window is written; the monitor's reading of it is `set-S/phase-7`'s).
A run blocked inside an approval wait or a long model call notices the detach when it returns, not before.

There was no shutdown path at all: the process simply stopped, and `recover_orphans` marked every
unfinished run `failed` on the way back up. A deploy during a research session cost the session.

Now `SIGTERM` (and Ctrl-C, because a drain path only exercised in production is a drain path
nobody has tested) closes the door and waits. New turns are refused with **503 and a duration**,
not 409 `agent_busy` — the two are different instructions, and a desk that cannot tell them apart
shows "at capacity" through every deploy until the message means nothing to anyone.

The three cases are handled differently on purpose: a **suspended** run needs nothing; a
**mid-step** run finishes its step, which is one model call plus one tool call and therefore
seconds; a run still **inside a tool** past the thirty-second deadline is **left `running` and
recovered by resume, never killed**. A tool killed mid-write is exactly the duplicated-write case
the effect log exists to avoid, and a deploy is not a good enough reason to create one.

`drain` returns how many were still going, so the caller reports what it observed rather than
claiming a clean stop it did not.

Verified by `a_draining_platform_says_how_long_rather_than_saying_busy` (which asserts the two
messages differ, not merely that one exists) and
`draining_an_idle_manager_is_immediate_and_says_so` — including that a drain does not reopen,
since a reopened one would accept a turn the process is about to stop serving.

**Not done:** COMP-007 is not told, so a rolling deploy still reads to the monitor as a heartbeat
loss and can trip a global halt. That needs the monitor's maintenance-window API, which is
`set-S/phase-7`'s territory rather than this one. **That is why this is `PARTIAL`.**

Resume makes a *crash* survivable; a deploy is not a crash, and treating it as one throws away
work that did not need to be lost. On `SIGTERM`: refuse new turns with a `503` naming the
expected duration (the desk shows it rather than failing a click); suspended runs need nothing
and resume across the version change under S-2.6's schema rule; mid-step runs finish the
current step, write the effect and suspend at the boundary (a step is one model call plus one
tool call, so seconds); a run still inside a tool past the deadline is left `running` and
recovered by resume, never killed mid-write. Tell the monitor, so a rolling deploy does not
read as a heartbeat loss and trip a global halt.

**Files:** `crates/api/src/agent/manager.rs`, `apps/platform/src/main.rs`.
**Acceptance:** a deploy during a research session costs one status line, not a turn; no run
is killed mid-write; COMP-007 records a maintenance window rather than a platform trip.
**Traces:** AGENT-010 §10.1; COMP-007 §4.3.

### S-2.16 Cost is an approval axis — S
`PARTIAL` 2026-09-16 — the axis exists and refuses correctly; the estimator that feeds it is

**2026-10-05 (S-RUNTIME) - PARTIAL, unchanged.** `config/prices.yaml` still ships `hardware: []`; a row needs a measured draw and price for this box, and an invented one would make `ask_above_usd` fire on a number nobody measured.
S-3.19 and does not.

**2026-10-03 (RUNTIME track) — PARTIAL.** The estimator now exists and is reached. The loop takes a `CostEstimator` (`Loop::with_cost_estimator`) and the cost
axis reads it; `local_driver::cost_basis` builds one from this tenant's *measured* throughput
(`/api/agent/cost/estimate`, S-3.19) and a fresh hardware price row for `TBOT_HARDWARE_CLASS`, and prices only calls that
spend a trial over a named window. The approval card now carries the estimate and the remaining dollar budget
(`the_cost_axis_fires_from_the_drivers_estimator`: a $42 call with $60 left asks and shows both). **Why still PARTIAL:**
`config/prices.yaml` ships no hardware row (nobody has measured this box), so in production `estimated_usd` stays `None`
until one is added, and a frontier model's provider calls carry no per-call estimate at all.

Cost is now its own gate, above the untrusted-content escalation and below the approval
envelopes. The position is the argument: an envelope is a statement about **what** the action is
and outranks everything; cost is a statement about what it **spends**, which is worth asking about
even on an action nobody would otherwise pause.

Reversibility is deliberately not consulted. It is about whether the *state* can be put back and
says nothing about whether the money can — a sweep that spends a day of budget and finds nothing
is perfectly reversible and perfectly wasted. `an_expensive_reversible_call_asks_anyway` is the
case the task exists for, and `a_cheap_irreversible_call_still_asks` holds the other half: the
axis is added to the existing gates rather than replacing them.

The card carries the estimate **beside the remaining budget**, because $42 is a different question
with $60 left than with $45, and a human asked to approve a number with nothing to compare it to
will approve it.

`estimated_usd: None` means **not estimated, never free**. Treating unknown as zero would be a
cost gate that reads as present in the config and silently never fires — worse than no gate,
because the config would be lying.
`an_unestimated_call_does_not_silently_pass_the_cost_gate_as_free` holds that.

`ask_above_usd` is `$5` on the frontier profile and absent on both locals, which is the honest
reading for a tier where a call costs electricity rather than dollars.

**Not done:** nothing supplies `estimated_usd` yet. `estimate_cost` is S-3.19, this task's
declared dependency, so every caller passes `None` today and the axis does not fire in practice.
The mechanism is built and tested; it is not yet reached. **That is why this is `PARTIAL`.**

The checkpoint policy gates on reversibility and confidence, which is right and incomplete: a
reversible action can still be expensive, and "reversible" is no comfort after a sweep has
spent a day of budget. A call whose `estimate_cost` exceeds `budgets.ask_above_usd` (default
20% of the task budget, or any single call over $5) requires a human **whatever its
reversibility**, and the approval card shows the estimate beside the remaining budget.

**Depends on:** S-3.19 for `estimate_cost`.
**Files:** `crates/harness/src/policy.rs`, `config/profiles/*.yaml`.
**Acceptance:** an expensive reversible call asks; a cheap irreversible one still asks; the
approval card carries the estimate and the remaining budget.
**Traces:** AGENT-008 §4; AGENT-006 §2.4b.

---

## Exit criteria

- One driver. `manager.rs` has no tier branch and `driver.rs` does not exist.
- A killed process resumes with no duplicated write effect.
- A 40-minute sweep costs zero tokens while it runs.
- Approvals, steering and escalation-with-state work on every tier.
- `cargo test -p harness --test conformance` and `--test failure_injection` green; the
  shadow comparison shows no blocking difference against the old hosted path.
