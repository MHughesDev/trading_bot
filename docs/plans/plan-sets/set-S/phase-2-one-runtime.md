# Phase 2 — One runtime

**Completion: 19% (3 / 16 tasks). S-2.1, S-2.3 and S-2.4 landed 2026-09-15.**

**Requirement IDs:** AR-01…AR-16.
**Spec:** [AGENT-010](../../../specs/AGENT-010-agent-runtime.md), ADR-0035.
**Depends on:** Phase 1 (all three tracks).
**Blocks:** Phase 8. Phase 6 consumes its effect log but can start before it.
**Migration range:** 0061–0064.
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
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

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
`NOT STARTED`

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
