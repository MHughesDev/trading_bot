# AGENT-010: The Agent Runtime — one loop, every model, every tier

> **Status:** **Partially implemented** — Set S phase 2. Re-checked 2026-09-21.
>
> The sans-IO loop, its effect log and the one driver (`crates/api/src/agent/local_driver.rs`) run every turn on every tier; `driver.rs` (the hosted loop) was deleted on 2026-10-03 (S-2.10), and `manager.rs` no longer branches on tier. Resume from the effect log, job suspension, `AskHuman` over `NOTIFY`, token/dollar budgets with a report turn, escalation with state, steering and a per-run drain are built (S-2.6, S-2.11, S-2.12, S-2.14, S-2.15); what is not yet proved end to end against a live Postgres or a live frontier credential is named per task in the phase file. One recorded deviation from §13 step 3: there is no `agent.single_loop` flag and no fallback (ADR row in S-2.9).
>
> **The live count is in [`docs/plans/plan-sets/set-S/phase-2-one-runtime.md`](../plans/plan-sets/set-S/phase-2-one-runtime.md), and the programme total is in [`docs/plans/PLAN.md`](../plans/PLAN.md).** It is deliberately not restated here: a number written into a spec is a number nobody recomputes, which is how this line came to say "ready to build" about code that was already serving traffic. `crates/invariants/tests/the_plan_counts_what_the_plans_say.rs` holds those two files honest; nothing holds a hand-copied number here.
**Date:** 2026-09-14
**Author:** Claude (Fable 5.1), from `crates/harness`, `crates/api/src/agent/{manager,driver,local_driver,bridge,orchestrator}.rs`, `crates/api/src/llm_proxy.rs`, `crates/llm`, `crates/jobs`, ADR-0022/0024/0031/0032/0033
**Replaces:** AGENT-001 (agent runtime). Its §5 storage, §6 tokens/scopes, §10 workspace contract, §21 security checklist are carried forward with amendments; its SDK-container architecture (§4, §7–§9, §12–§13) is retired as the research agent's runtime
**Depends on:** AGENT-005 (context), AGENT-006 (catalogue), AGENT-007 (memory), AGENT-008 (protocol), AGENT-009 (evals)

---

## 0. Why this spec exists, and the decision it makes

There are three agent runtimes in this repository:

1. `crates/api/src/agent/driver.rs` — a multi-turn provider loop for hosted models: no routing, no caching, no budgets read, no approvals ("this driver has no pause", `driver.rs:685`), no resume.
2. `crates/api/src/agent/local_driver.rs` + `crates/harness::drive::Loop` — a sans-IO governor loop with routing, constrained decoding, budgets, approvals, a canary, a trace. Used only when the profile's tier is local (`manager.rs:285-335`).
3. `apps/agent-host` in a Docker container on the Claude Agent SDK (ADR-0024) — full Bash, files, sub-agents, compaction, built-in. Claude-only, its image is built by nothing, and its "harness" is the SDK's.

ADR-0024 chose (3) because rebuilding a coding-agent harness in Rust "would not beat existing harnesses". That was right for a coding agent and wrong for this one: the research agent's hard problems — honest trials, budgeted context across a 7B-to-frontier range, typed termination, approval envelopes, replayable trajectories — are exactly the things (2) already does and (3) cannot be made to do from outside a vendor's loop. Meanwhile (1) exists only because (2) was assumed to be for small models.

**Decision:** there is one runtime, the `harness::drive::Loop`, for every tier. The hosted driver is deleted. The container is kept for one job it is genuinely good at — a sandbox with the research Python stack — and becomes the executor behind `run_python` at the `full` code-execution tier, invoked as a tool, not as the agent. The user's requirement ("anything from a 7B to a frontier model should be able to use this agent harness") is met by construction rather than by keeping two loops in step.

## 1. Principles

- **P1 — One loop, two transports.** `Loop::next(Input) -> Vec<Effect>` is the whole agent. A driver pumps effects: model calls go to an adapter, tool calls to the catalogue, human questions to the UI, notes to the trace. The frontier tier differs from the 7B in profile values and in the adapter's transport (`native` multi-turn vs. `select_fill` single-turn), nowhere else.
- **P2 — The container is the thing being contained.** Enforcement (budgets, policy, provenance, routing) runs at the platform above any executor. A sandbox is for code the agent writes, never for the agent's judgement.
- **P3 — Every effect is durable before it is acted on.** The effect log is the run's state; a crash resumes from it; an eval replays it.
- **P4 — Budgets are enforced where they are declared.** A profile value nothing reads is a bug (the harness crate's own rule). `max_usd_per_task`, `max_tokens_per_task`, `max_steps` are read by the loop; there is no wall clock, but there is a no-progress watchdog.
- **P5 — Waiting costs nothing.** Long platform work (sweeps, studies, training, shadow sessions) runs in the job service; the loop suspends on a job id and resumes on completion. Polling is a bug.
- **P6 — Approvals on every tier.** A human question is an effect, not a special case of one driver.
- **P7 — Provider-neutral, capability-declared.** An adapter states what it can do (native tools, grammar, prompt cache, JSON schema, `num_ctx`) and the profile is verified against it before a run starts. No provider name appears in the loop.

## 2. Architecture

```
UI / API  ──turn──▶  Manager (conversation, run, token, workspace)
                          │
                          ▼
                    Driver (pump)  ──▶  Adapter (Anthropic | OpenAI | Ollama | vLLM)  ──▶  LLM proxy (usage, cache TTL, budget)
                          │  ▲
                Effects   │  │ Inputs
                          ▼  │
                  harness::drive::Loop  ──▶  Catalogue dispatch (typed | templated | workflow | fs)
                          │                       └──▶ Job service (sweeps, studies, training, shadow, profiler)
                          ▼                       └──▶ run_python executor (sandbox container, `full` tier only)
                   Effect log (agent_events) ──▶ trace ──▶ evals / UI timeline / resume
```

[DO] Delete `driver.rs`. `manager.rs:285-335` always builds `LocalRunParams` (renamed `RunParams`) and calls the single driver, which is `local_driver.rs` renamed `driver.rs`. The tier decides the adapter and the transport; nothing else branches on tier in `crates/api`.

## 3. The model adapter

[DO] Extend `harness::adapter::ModelAdapter` (`adapter.rs:20`) with a `Capabilities` struct the adapter reports at construction and `verify` (`adapter.rs:64`) checks against the profile:

```rust
pub struct Capabilities {
    pub native_tools: bool, pub parallel_tools: bool,
    pub grammar: GrammarSupport,          // None | JsonSchema | Enum | Both
    pub prompt_cache: CacheSupport,       // None | Automatic | Breakpoints
    pub num_ctx_settable: bool, pub keep_alive: bool,
    pub max_context: u32,                 // the provider's advertised window
    pub reports_usage: UsageSupport,      // None | Totals | WithCache
}
```

Implementations: `AnthropicAdapter` (native, breakpoints, usage with cache — AGENT-005 §6.1), `OpenAiAdapter` (native, automatic cache, `cached_tokens`), `OllamaAdapter` (grammar via `format`, `num_ctx`, `keep_alive`, no native+grammar composition — LOCAL_TIER_FINDINGS §2), `VllmAdapter` (native *or* `guided_json`, `num_ctx`). `for_profile` (`adapter.rs:191`) picks by `profile.provider`. A profile asking for `decode: native` on an adapter without `native_tools`, or `select_fill` without `grammar`, is refused at load.

The two-transport rule: `native` means the adapter keeps a provider transcript for the run (append-only messages, cache breakpoints per AGENT-005 §6.1); `select_fill` means each call is single-turn over the assembled body (today's local path). Both are driven by the same effects; the adapter owns the difference.

[DO] All adapters call through `crates/api/src/llm_proxy.rs` (today only the container does): it applies the 1-hour cache TTL rewrite (`llm_proxy.rs:55`), records `Usage` with cache fields, and refuses a request over the profile's `input_budget_tokens` with `context_budget_exceeded` — which, with AGENT-005 §2.2 in place, should never fire and is therefore the regression alarm.

## 4. Runs, turns, conversations

Carried from ADR-0033: a conversation is a sequence of turns; a turn is a run; each turn gets the conclusions of earlier turns (AGENT-007 §2.1 turn summaries), not their transcripts. `migrations/0041_agent_conversations.sql` stands.

[DO] `agent_runs` gains `harness_version`, `profile_name`, `adapter`, `transport`, `prompt_tokens`, `est_prompt_tokens`, `cache_read_tokens`, `trials_spent`, `n_eff_after`, `outcome` (the `Outcome` variant name) — the columns AGENT-005 §7 and AGENT-009 §4 read. AGENT-013 adds `mode`, `workflow`, `ring`, `stage`, `prereg_id` and `yield_ref`; AGENT-014 adds `session_id`, `agenda_item_id` and `lease_id` (2026-09-14; every Research run has a session row, AGENT-014 §10).

## 5. The effect log (durability and replay)

[DO] Replace the untyped `agent_messages(kind, content_json)` timeline with `agent_events(run_id, seq, kind, payload_json, created_at)` where `kind` is the `Effect`/`Input`/`Note` variant name (`drive.rs:223, :324, :372`) and `payload_json` is the variant's fields, plus `state_snapshot` rows at each step boundary carrying the `WorkingState` hash (AGENT-008 §2). The UI timeline is a view over it; the trace (`trace.rs`) is derived from it; AGENT-009 replays it.

Write-ahead: the driver persists an effect **before** executing it and the resulting input **before** feeding it back. On restart, a run in `running` state is resumed by replaying its inputs through a fresh `Loop` of the same `harness_version` (the loop is deterministic given inputs), then continuing from the last effect: an `ExecuteTool` with no recorded result is re-executed if the tool is idempotent (`ToolDef.idempotent`, derived from `Risk::Read`) and otherwise turned into a `ToolError("interrupted; not retried because it writes")` the model sees; a `CallModel` with no reply is re-issued.

`harness_version` mismatch on resume → the run is marked `interrupted` with the working
state preserved and offered to the user as a new turn ("continue from step 14").

**A suspended run is the exception, and deliberately so.** A run waiting on a 40-minute
sweep (§8) holds no process and no connection, so a deploy inside that window is likely
rather than exceptional. Discarding the sweep because the binary changed underneath it would
make the platform's most expensive operation its least reliable.

- `Effect::Await` writes a **resume contract** into the effect log: the job id, the working
  state, and the effect-log *schema* version, which changes far less often than
  `harness_version`.
- On `Input::JobDone` under a different `harness_version` but a compatible schema version the
  run resumes, and the version change is recorded as a `Note` so an eval replay can see it.
- Only an incompatible schema version interrupts — and even then the **job result is attached
  to the conversation as an artifact with its ids**, so the compute is never lost; the agent
  reads it in the next turn.

### 4.1 One live turn per conversation

A conversation is a sequence of turns and exactly one may be live. A second is rejected with
the running turn's id rather than queued: two runs sharing one conversation workspace
(`agents/<conversation_id>/`) would race on files with no lock, and two `WorkingState`s for
one goal is not something the desk or the turn summary can render. A user who wants parallel
work opens a second conversation; an agent that wants it fans out with sub-runs (§9), which
carry their own budget leases and their own effect logs.

Steering, approving and stopping act on the live turn. This is the runtime's only concurrency
rule and it is enforced at the manager, not by convention.

## 6. Budgets, watchdogs, escalation

[DO] The loop reads and enforces:
- `orchestration.max_steps` (exists), `budgets.max_tokens_per_task`, `budgets.max_usd_per_task` (priced from the adapter's usage and a per-model price table in `config/prices.yaml`). Approaching any of them (≥ 90%) sets `wrapping_up`; reaching one ends the run with `Outcome::Finished { budget_exhausted: true }` after the report turn.
- **No wall clock.** A no-progress watchdog instead: no effect for `orchestration.idle_timeout_s` (default 600 s, excluding suspended waits) → `Degradation::BackendFailure("stalled")`. `max_wall_clock_s` is deleted from the profiles.
- Trials: `trials_spent` and `n_eff` from the ledger are in the `BudgetView` the model sees; a profile may set `budgets.max_trials_per_task`.
- Escalation (`Outcome::Escalated`, `profile.escalate_to`) carries the `WorkingState` and the effect log reference; the manager starts the next run on the target profile with the state as its brief (AGENT-008 §4).

## 7. Approvals and steering

`Effect::AskHuman` is answered by the UI on every tier (the `await_approval` poll in today's `local_driver.rs:749` becomes an event subscription on `agent_events`). Envelopes come from the catalogue (AGENT-006 §2.6). A pending approval suspends the run at zero token cost; the user's answer is an `Input::Approval` and, when it states a preference, an AGENT-007 `user_preference`. The user can also *steer* mid-run: a message on a running turn is queued and delivered as an `Input::Steer { text }` at the next step boundary, rendered into the working state's open questions — this replaces the ad-hoc "nudge" messages the hosted driver injected (`driver.rs:580-593`).

## 8. Long work: suspend on jobs

[DO] `run_sweep`, `run_study`, `train_model`, `run_shadow_session`, and any workflow whose steps include them, return a `job_id` immediately. The loop emits `Effect::Await { job_id }`; the driver persists it and **exits** — the run is `waiting` and holds no task, no model connection, no tokens. The job service's completion (`crates/jobs`, `ResumeSupport`/`CheckpointManifest`) posts `Input::JobDone { job_id, result }` and the manager resumes the run from its effect log (§5). Today's in-driver polling loops (`driver.rs:334`, `:408`) and their 30-second status rows are deleted; the job service emits progress events into `agent_events` itself.

## 9. Sub-runs

A sub-run is a run with a parent, a `BudgetLease` split from the parent's remaining budget (SUBAGENTS-DESIGN §6: the lease is not `Clone`; it is spent by exactly one run), depth ≤ `max_subagent_depth` (1), its own effect log, and a `Delegation` trace event on both sides. Three roles, all expressed as tasks on the same loop with a role-specific charter tail and catalogue slice:
- **critic** — fresh context, brief = evidence bundle only, tools = read-only + `report.*`, never the parent transcript (AGENT-008 §3.4 rung 8);
- **worker** — a byte-identical brief per worker (cache-friendly), used by workflows that fan out (e.g. `precapital_battery`'s red team);
- **reader** — no tools; summarises a handle into ≤ N tokens (AGENT-005 §4.6 on capable tiers).
The parent receives a sub-run's `Outcome` as an `Input::SubRunDone`.

**The campaign's HYPOTHESIZE is a worker sub-run** (AGENT-013 §8.5, added 2026-09-14). When a run in the `optimise` workflow is suspended on a `JobKind::Campaign` job, the campaign driver's HYPOTHESIZE phase dispatches a `worker` sub-run of that parent — the `ResearchRun` child worker HANDOFF §2 lists as missing — with a projection of the parent's context store (AGENT-005 §29) and a lease from the campaign's budget, whose sole output is one `Move` with a hypothesis and parameter ranges. The driver logs it as a `propose` decision and continues; the parent stays suspended until the campaign's terminal event. The platform owns the phases; the agent owns what to try.

## 10. Code execution

`run_python` (AGENT-006 §5.9, AGENT-005 §8):
- `code_execution: templated` and `denied` tiers: the skill runner subprocess on the host, templates only (no free-form script).
- `code_execution: full`: the **container** from ADR-0024, repurposed: `infra/agent-image` (research stack, non-root, read-only, internal network) runs a small executor service that accepts `{script, payload, data_path}` and returns the `ToolResult`, with the same limits as the skill runner scaled up (300 s, 4 GB). The container is started per conversation, mounts that conversation's workspace, and is torn down with it. The Claude Agent SDK is removed from the image; the agent is not in the container, its code is.
- Either way the call is an ordinary `ExecuteTool` effect with `Risk::Write`, recorded, budgeted, capped.

[DO] Build the image in CI (it is built by nothing today) and add `docker-compose` service `agent-sandbox`.

### 10.1 Draining: a deploy with live runs

Resume (§5) makes a *crash* survivable; a deploy is not a crash, and treating it as one throws
away work that did not need to be lost. On `SIGTERM` the platform drains:

1. Stop accepting new turns; the API answers `503` with the expected drain duration, and the
   desk shows it rather than failing a click.
2. Runs waiting on a job or a human are already suspended and need nothing — they are the
   common case, and they resume across the version change under §5's schema rule.
3. Runs mid-step finish the current step, write their effect, and suspend at the boundary.
   A step is bounded (one model call plus one tool call), so this is seconds, not minutes.
4. A run still executing a tool after the drain deadline is left `running` and recovered by
   §5's resume on the next boot. It is not killed mid-write.
5. The monitor (COMP-007) is told, so a rolling deploy does not read as a platform heartbeat
   loss and trip a global halt.

The measure of this working is that a deploy in the middle of a research session costs the
user one status line, not a lost turn.

### 12.1 The agent can ask how it is doing

An agent that cannot tell whether it is degraded reports confidently from a degraded state,
which is the failure this whole set exists to prevent — and every fact it needs is already in
the platform. One read-only tool, `get_self_status()`, returns: the active profile and tier;
whether that tier is `measured` or `unmeasured` (AGENT-009 §5); the decode ladder in use and
whether the canary passed; the exposure budget and how full the slice is; budget remaining
across steps, tokens, dollars and trials; the calibrated bytes-per-token and its sample size;
any open circuit breaker (AGENT-008 §3.7); the monitor and kill-switch state; and any
`Degradation` recorded this run.

It exists so the agent can say "I am on an unmeasured local tier with two tools left in the
slice and 8% of budget, so I am going to answer narrowly and stop" — which is a better
research partner than one that discovers each of those facts by hitting them. It is also what
the desk's tier badge and the escalation decision read, so the human and the model are looking
at the same numbers.

## 11. Security envelope (carried, amended)

Carried from AGENT-001 §21 and the code: per-run service tokens with a 7-day expiry (`manager.rs`), no `place_order` tool, `Risk::Outbound` limited to the two web tools with `webfetch::egress` deny-by-default, provenance tagging of tool output (`provenance.rs`), workspace lexical containment and quotas (`workspace.rs:166`), envelopes for §15 actions (fixed per AGENT-006 §2.6). Amended: the trifecta statement in every profile is regenerated from the catalogue (which tools are outbound, which read private data) rather than hand-written prose; the conformance test compares them.

## 12. Observability

Every `agent_events` row is a trace span (`trace.rs`); the UI timeline, the eval trajectory and the cost dashboard are three views of one table. Per run: tokens by layer (charter / schemas / body / output), cache hit rate, cost, steps, trials, N_eff, findings, negative results, retrievals cited, approvals asked, outcome. Per model (from AGENT-009 §5): calibration and scorecard. The Meridian governance page shows tier status (`measured | unmeasured`) from the calibration table.

## 13. Migration

| Step | Change | Safety |
|---|---|---|
| 1 | Add `agent_events`; dual-write with `agent_messages`; UI reads the new table | no behaviour change |
| 2 | `Capabilities` on adapters; route all adapters through `llm_proxy` | the proxy's budget refusal is a no-op alarm until AGENT-005 lands |
| 3 | Frontier profiles run on the harness loop with the `native` transport behind a feature flag `agent.single_loop`; `driver.rs` kept as fallback for one release; AGENT-009 shadow replay compares outcomes | rollback = flag |
| 4 | Delete `driver.rs`; `Effect::Await` + job suspension; approvals on all tiers | needs jobs completion events |
| 5 | Budgets/watchdog/escalation-with-state; sub-runs with leases | |
| 6 | Container repurposed as the `run_python` executor; image in CI; SDK removed | the old agent-host is archived, not deleted, for one release |

## 14. Acceptance

> **Form note (Set S task S-0.4, 2026-10-03).** Converted from a plain numbered
> list to the repository's acceptance form so `docs/procedures/execute-plan.md`
> step 3c can close the chain requirement -> criterion -> evidence. No criterion
> was added, removed, reworded or softened; each kept its position and its number
> (`1a` is `AC-1a`), so item *n* of the old list is `AC-n`. An unticked box with
> `Verified by: [--]` means nothing has been proved, which is the honest default.
> The form is the one AGENT-005, AGENT-006 and AGENT-008 use.

- [x] AC-1: `crates/api/src/agent` contains one driver; `manager.rs` has no tier branch. - Verified by: `api::agent::manager::tests::{the_agent_module_contains_one_driver,the_manager_does_not_branch_on_tier,no_runtime_code_writes_an_unrecoverable_elision}`; `crates/api/src/agent/driver.rs` is deleted (S-2.10) and `manager.rs` always builds `LocalRunParams`. No feature flag (ADR row in phase 2, S-2.9: rejected the `agent.single_loop` flag, which would keep an un-budgeted, un-resumable second loop alive on the platform's most expensive path). Written, not re-run in this pass (needs live Postgres or a full crate build); CI runs it. *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-2: A frontier run and a 7B run of the same lookup goal produce trajectories with identical effect *kinds* differing only in `CallModel` payloads and exposure sizes. - Verified by: `harness/tests/one_working_state.rs::both_transports_ask_the_world_for_the_same_things` (native and select transports produce the same effect kinds), `harness::eval::cases::the_0_of_10_eval_bug_exists_as_a_passing_replay_case` replayed on both `ProfileKind::Degraded7b` and `ProfileKind::Frontier`, and the frontier-profile replay `api::agent::local_driver::tests::a_resumed_run_does_not_repeat_an_interrupted_create_experiment`. **Partial / BLOCKED:HUMAN for exactly one clause:** a *live* frontier run and 7B run of the same lookup goal compared trajectory to trajectory needs a provider credential (gate H-2). *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-3: Killing the API process mid-run and restarting resumes the run from the effect log with no duplicated write effect (asserted on a run that includes `create_experiment`). - Verified by: `api::agent::local_driver::tests::{a_resumed_run_does_not_repeat_an_interrupted_create_experiment,a_log_that_does_not_replay_to_the_same_state_is_refused,a_log_ending_on_an_input_replays_to_an_unlogged_batch}` (a log cut after the approved `create_experiment`: both the no-record and the recorded-result branches issue zero `create_experiment` effects), `api::agent::resume::tests::{an_interrupted_write_is_not_retried,an_effect_with_no_recorded_safety_is_treated_as_a_write,a_suspended_run_resumes_across_a_harness_version_change}`, and `api/tests/pg_agent_effect_log.rs::{an_effect_is_recorded_before_the_input_that_answers_it,an_interrupted_write_is_visible_as_a_trailing_effect,the_resume_decision_reads_an_interrupted_write_out_of_the_database}`. **Partial:** the kill is *simulated by replaying a truncated log*; no process was killed and the database half of `resume()` (`load_log`, `spawn_resume`, the idempotency read of migration `0170`) has not been run against a migrated Postgres (S-2.6 `PARTIAL`). *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-4: A run waiting on a 40-minute sweep holds no model connection and adds zero tokens until the job completes. - Verified by: `harness/tests/waiting_costs_nothing.rs::{a_run_waiting_on_a_job_emits_no_model_call,nothing_short_of_the_job_finishing_restarts_a_waiting_run,another_jobs_result_does_not_resume_this_run,the_job_result_is_the_tool_call_s_result_arriving_late}` (the proof is the *absence* of a `CallModel` effect while suspended) and the driver half: a call still running returns a handle, the loop suspends on `Effect::Await`, `local_driver` writes the resume contract and exits, and `AgentManager`'s watcher resumes it with `Input::JobDone` (`api::agent::local_driver::tests::only_a_started_campaign_suspends_its_run`). **Partial:** the suspend-then-resume path end to end against a database has not run, and `train_model` and `run_study` return their own handles that are not yet mapped to a job id (S-2.11 `PARTIAL`). *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-5: `max_usd_per_task` reached → the run ends after one report turn with `budget_exhausted: true`; no profile contains `max_wall_clock_s`. - Verified by: `harness::drive::tests::{a_spent_token_budget_buys_one_report_turn_and_says_so,a_spent_dollar_budget_wraps_up_and_an_unpriced_call_poisons_the_total,without_a_fresh_price_the_dollar_cap_is_not_enforced}`, `harness/tests/acceptance_and_budget_are_honest.rs::{the_dollar_cap_is_refused_although_the_profile_declares_one,an_unpriced_run_cannot_be_rendered_as_a_figure,the_token_cap_reaches_the_wire}`, and `harness/tests/conformance.rs::every_profile_field_is_enforced_somewhere` (no `max_wall_clock_s` field exists in any profile). **Partial:** the dollar cap cannot fire for a local model, because `config/prices.yaml` has no verified hardware row; the token cap is the enforced one (S-2.12 `PARTIAL`). *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-6: `AskHuman` on a `SealedHoldout` action suspends a frontier run exactly as it suspends a local one. - Verified by: `harness::drive::tests::{the_approval_card_names_the_envelope_and_not_the_tool_again,a_refused_approval_moves_on_instead_of_ending_the_task}`, `mcp-server-lib::gate::tests::a_sealed_holdout_tool_declares_its_envelope`, and `harness/tests/failure_injection.rs::cancelling_while_blocked_on_a_human_still_terminates`; `AskHuman` is the same effect on every tier because there is one loop (`the_manager_does_not_branch_on_tier`). **Partial:** `spend_holdout_look` does not exist as a tool yet (AGENT-006 AC-2), so the `SealedHoldout` suspension is proved for the envelope declaration and the loop's approval path, not for a frontier run suspended on that action. *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-7: Escalation from `qwen2.5-7b` to `claude-opus-5` resumes with the working state and does not re-run completed steps. - Verified by: `harness::drive::tests::{a_stuck_tier_with_somewhere_to_go_escalates_carrying_its_state,a_resumed_run_does_not_redo_completed_steps}` (`Stuck` on a profile with `escalate_to` ends `Outcome::Escalated{state}`; the manager starts the target with `Loop::resuming_from`; trials count once). **Partial:** a live `qwen2.5-7b` to `claude-opus-5` escalation needs a credential (gate H-2); the control-plane *Escalate* button is struck from the desk because a mid-run tier change has budget consequences and no route (S-6.6). *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-8: `run_python` on the `full` tier executes in the sandbox container with the conversation's workspace mounted and no provider credential present. - Verified by: `invariants::code_execution_is_tiered::{the_gated_namespace_is_the_one_the_tool_is_declared_in,only_a_full_tier_is_offered_the_code_tool}`, `mcp-server-lib::tools::code::tests::{free_form_code_is_not_a_read,the_sandbox_cannot_run_the_simulator_so_it_spends_no_trial}`, and the `agent-sandbox-image` CI job (a credential in the environment stops the container starting; a `data_path` outside the workspace is refused; a plain script runs). **Partial (ADR-S8-06):** the per-conversation container *launcher* is a deployment contract (`TBOT_SANDBOX_CMD`), not a component here; with it unset the tool refuses rather than falling back to the host. *(Filled 2026-10-04, S-8.9.)*
- [x] AC-9: Every adapter's `Capabilities` are verified against its profile at load; a mismatched profile fails to load with the field named. - Verified by: `harness::adapter::tests::{the_sdk_adapter_verifies_against_a_frontier_profile,a_local_profile_on_the_sdk_adapter_is_refused,the_local_adapter_refuses_rather_than_pretending,a_probed_backend_that_honoured_every_probe_verifies_a_local_profile,a_profile_needing_a_grammar_is_refused_by_a_backend_without_one,a_profile_decoding_native_is_refused_by_a_backend_without_tool_calls,a_window_larger_than_the_backend_accepts_is_refused_with_both_numbers,an_adapter_that_declares_nothing_admits_nothing,a_backend_that_ignored_the_grammar_fails_verification}`: the refusal names the field and both numbers. Ran green with the harness lib in this pass. *(Filled 2026-10-04, S-8.9.)*
- [ ] AC-10: A campaign advances through HYPOTHESIZE via a worker sub-run of the suspended parent whose context excludes the parent's reasoning, and AT-59 (resume is a pure fold) still passes with the agent in the loop. *(Added 2026-09-14, AGENT-013 §8.5.)* - Verified by: `api/tests/campaign_hypothesize_recorded.rs::{a_campaign_hypothesizes_through_the_worker_sub_run,a_refused_proposal_gets_one_retry_carrying_the_validators_reason,two_refusals_end_the_generation_with_a_typed_reason_and_no_third_call}`, `api::agent::subrun::tests::{the_assembled_brief_is_a_worker_projection_without_the_reasoning,a_worker_under_the_wrong_projection_is_refused,a_persisted_lease_refuses_past_its_steps_and_tokens_and_a_missing_axis_is_not_zero}` (the context excludes the parent's reasoning by construction: a projection, not a filter) and `api::agent::local_driver::tests::only_a_started_campaign_suspends_its_run`. **Partial:** a campaign advancing through HYPOTHESIZE on the reference fixture with a live model has not run, and AT-59 *with the agent in the loop* is asserted for the replay of recorded effects, not a live run (S-10.11 `PARTIAL`). Written, not re-run in this pass (needs live Postgres or a full crate build); CI runs it. *(Filled 2026-10-04, S-8.9.)*
## 15. Non-goals

- Reintroducing a wall clock. A stalled run is caught by the watchdog; a slow one is not a failure.
- Multi-tenant scheduling fairness across users (single-operator platform today).
- Running the agent *inside* the sandbox. That was ADR-0024; this spec reverses it and says why in §0.

## 16. Open questions

- Whether `native` transport on vLLM is reliable enough for `local_high` to skip `select_fill`; AGENT-009's `decode` suite decides per model.
- ~~Per-model pricing for local models (electricity/time) so `max_usd_per_task` means something there; ship as 0 with tokens as the binding budget.~~ **Closed 2026-09-22 by ADR-D-03.** The price table §6 names (`config/prices.yaml`) carries rows **per hardware class, per second**, as well as per provider model, each with `source_url` and `verified_at` — so a local model is priced by the class it runs on rather than left at zero. **"Ship as 0" is struck**, because zero is not the absence of a price, it is the claim that the run was free: it makes `max_usd_per_task` pass unconditionally and `ask_above_usd` never fire, and both failures look exactly like the gate working. A model with no fresh row prices as **`null`** — "not estimated", never "free" — `max_usd_per_task` is enforced only against a fresh row, and tokens remain the binding budget there, which was the sound half of the original answer. See AGENT-006 §2.4b for the full rule.
- The container-per-conversation lifetime versus a warm pool; measure cold start first.
