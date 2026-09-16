# Phase 2 — One runtime

**Completion: 31% (5 / 16 tasks), plus 7 partial (S-2.5, S-2.6, S-2.7, S-2.8, S-2.11, S-2.14, S-2.16). Four remain `NOT STARTED`: S-2.9, S-2.10, S-2.12, S-2.15.**

**Requirement IDs:** AR-01…AR-16.
**Spec:** [AGENT-010](../../../specs/AGENT-010-agent-runtime.md), ADR-0035.
**Depends on:** Phase 1 (all three tracks).
**Blocks:** Phase 8. Phase 6 consumes its effect log but can start before it.
**Migration range:** 0061–0064.
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
Postgres. The dual-write to `agent_messages` is unchanged and the SDK driver is not wired.

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
and the run is still marked failed.

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
`PARTIAL` 2026-09-16 — the three behaviours now apply to every caller; they are shared code rather
than a shared HTTP hop.

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
`n_eff_after` have columns and no writer.

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
`NOT STARTED`

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
`NOT STARTED`

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
the resume path can replace them.

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
`NOT STARTED`

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
`NOT STARTED`

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
S-3.19 and does not.

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
