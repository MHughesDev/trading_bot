# Harness conformance report

Against `AGENT_HARNESS_GUIDE_v3.md`. Required by the guide's operating procedure
step 5: before declaring done, walk Parts 8 and 16 and state which items are
**satisfied**, **deferred** (with reason), or **N/A** for this archetype.

**Decision record:** [ADR-0031](adr/0031-harness-conformance-capability-profiles.md),
extended by [ADR-0032](adr/0032-local-tier-governor-loop.md) (the local tier).
**Archetype (procedure step 2):** A5 background/autonomous, with A3 knowledge-work
and A6 sub-agents switched on. Declared in `config/profiles/*.yaml`.
**Target tiers:** `frontier` and `local_mid` both active. The local tier's reference
model is `qwen3.6-35b-a3b` on vLLM; the deployment target is a single RTX 3090, then
two over NVLink (D-18). Ollama is supported at the degraded tier only.

---

## Where this application sits in the build order (Part 17)

The retrofit rule says: locate the current phase, fix gaps in **that** phase before
adding later-phase features.

This platform had a curious shape — Phase 4 and 5 features (sub-agents, background
execution with checkpointing, HITL approvals, containment) built on **Phase 1 and 2
gaps**: no tool registry, no exposure budget, no validation ladder, no context
budget, no capability profile. It had the autonomy before it had the floor.

So the work went to Phases 1–2 first, per the retrofit rule:

| Phase | State |
|---|---|
| 1 — Minimal reliable loop | **Now built**: Model Adapter with a live canary, validation ladder, hard context budget with fixed layout, trace, and — for local tiers — our own sans-IO state-machine loop (`harness::drive`). The loop remains the SDK's for `frontier` (ADR-0024). |
| 2 — Scale the tool layer | **Now built**: namespacing, exposure budget, `search_tools`, schema flattener, profiles as config, tool-result capping. |
| 3 — Environment & memory | **Partial**: isolation tier and deterministic compaction done; workspace contract built but not yet mounted; `remember` and skills not built. |
| 4 — Orchestration & autonomy | **Was already here**: background execution, checkpointing, HITL approvals, sub-agents. |
| 5 — Hardening | **Now built**: trifecta audit, provenance tagging, failure-injection evals, cost budgets, degradation ladder as profile deltas, replayable traces. |

---

## Part 8 — core checklist (Parts 0–7)

| # | Item | State | Where / why |
|---|---|---|---|
| 1 | Six harness components as separable modules (1.1) | **Satisfied** | `crates/harness/{registry,context,adapter,policy,provenance,trace}.rs`, plus `drive.rs` — the Execution Loop, now a sans-IO state machine of our own for local tiers. The SDK remains the loop for `frontier` (ADR-0024). |
| 2 | Profiles are config, loaded at startup; conservative default (1.2) | **Satisfied** | `config/profiles/*.yaml`, loaded in `AppState::new`. Unknown model → lowest tier present, with a warning. |
| 3 | Catalogue namespaced; router + `search_tools` (2.2) | **Satisfied** | `mcp-server/src/taxonomy.rs` (11 namespaces), `ToolRegistry::expose`, `ToolRegistry::search`. |
| 4 | Per-step exposure ≤ budget, enforced in code (2.1) | **Satisfied** | `expose()` truncates deterministically; `a_flooded_catalogue_still_respects_the_exposure_budget`. |
| 5 | Canonical schemas + automatic flattener (2.4) | **Satisfied** | `registry::flatten` — lifts nesting, collapses `oneOf`, caps optionals at 2, keeps enums. |
| 6 | Tool results capped, summarised, referenced (2.3) | **Satisfied** | `context::cap_tool_result` at `max_tool_result_bytes`; dropped blocks leave a handle. |
| 7 | Constrained decoding for local; startup canary (3.1) | **Satisfied** | `ChatRequest::schema` → Ollama `format` / vLLM `guided_json` / OpenAI `response_format`. The canary (`harness::canary`) probes at startup with **adversarial** prompts and distinguishes `Honoured` / `Rejected` / `Ignored`. Every constrained reply is re-validated against the schema it was sent, with no coercion; one violation is `Degradation::ConstraintIgnored` and fences the session. |
| 8 | Validation ladder with bounded retries (3.2) | **Satisfied** | `validation.rs`, five rungs. Bounded by `max_retries_per_call`; `RetryBudget::is_stuck` catches non-landing corrections. |
| 9 | Native chat/tool templates only (3.3) | **Satisfied** | The SDK uses its own. `verify()` refuses any adapter reporting otherwise, at every tier. |
| 10 | Hard context budget + fixed layout + deterministic compaction (4.1–4.3) | **Satisfied** | Compaction triggers at 95% of the usable window (`COMPACTION_TRIGGER`), not on every assembly — applying the per-section shares unconditionally truncated prompts that fit (ADR-0033 §4). `context::assemble` enforces all three for platform-assembled prompts; the **LLM proxy** enforces `effective_budget_tokens` on every upstream call, including the SDK's own. See the note below. |
| 11 | Scratchpad + read/search primitives (4.3) | **Satisfied** | Every conversation gets a workspace (ADR-0033) with `write_file` / `read_file` / `list_files` / `delete_file` over the five areas, path-canonicalised by `harness::workspace`. `search_scratchpad` is still not a first-class tool; `list_files` plus `read_file` covers the case it was for. |
| 12 | State-machine loop; harness-owned termination; `finish_task` (5.1) | **Satisfied** | `harness::drive::Loop` is an explicit 14-state machine. `finish_task` with required `result` + `evidence` is the termination contract at **both** tiers — the frontier driver's `FINAL:` prose contract was removed (ADR-0032 §7), and a run that stops without calling it is recorded `failed`, not `completed`. |
| 13 | Planner–executor for local tiers (5.2) | **Satisfied** | `harness::drive` plans first when `mode: planner_executor`, constrains the plan's namespace field to an enum of the catalogue, and routes each step from the plan. `the_loop_carries_every_profile_value_onto_the_wire`. |
| 14 | Stuck detection + failure budgets + resume (5.3) | **Satisfied** | `RetryBudget`, job lease reaper, SDK resume, checkpointed sessions. |
| 15 | Eval suite ships with the app; promotion by evidence (6.2) | **Satisfied** | `evals/` + the auditor suite in CI + `crates/harness/tests/failure_injection.rs`. |
| 16 | Degradation ladder as profile deltas, not forks (7) | **Satisfied** | `the_degradation_ladder_is_visible_as_profile_deltas` reads all six rungs off the two shipped profiles. |
| 17 | Every model-visible string written as model-facing UX (2.3) | **Satisfied** | Every `Rejection`, `Ruling` and `PathError` carries one short sentence and a `fix`; tested. |

## Part 16 — extended checklist (Parts 9–15)

| # | Item | State | Where / why |
|---|---|---|---|
| 1 | Archetype declared; simplest chosen (9) | **Satisfied** | `archetype: background` in the profile. Not the simplest archetype, but the correct one: sessions genuinely run for hours unattended. |
| 2 | Workspace layout + harness-side path validation (10.1) | **Partial** | `workspace.rs` implements the five areas, lexical traversal defence and read-only `inbox/`+`logs/`. **Not yet mounted**: the container still uses a flat `/workspace`. |
| 3 | Outputs-as-contract via `deliver` (10.1) | **Partial** | `Delivery::validate` enforces outputs-only; the tool is not yet exposed to the agent. |
| 4 | Isolation tier; default-deny network; no secrets; ephemeral (10.2) | **Satisfied** | T1 container, `internal: true` network, `--read-only`, `--cap-drop ALL`, uid 10001, proxy-injected credentials. Verified by running it. |
| 5 | Code execution sandboxed, profile-gated, policy-checked (11) | **Partial** | Sandboxed and profile-gated (`code_execution`). Destructive-shell-pattern denial is not implemented. |
| 6 | Four memory layers separated; `remember` with curation (12) | **Deferred** | Working and episodic exist (context, NOTEBOOK, artifacts). Semantic and procedural (skills) are Set R; `remember` is not built. |
| 7 | Sub-agents: depth 1, typed contracts, least privilege, by reference, shared trace (13) | **Partial** | Depth 1 enforced in the profile; `trace.rs` carries a shared `trace_id` and `Delegation`. Typed contracts and per-sub-agent tool grants are not built. |
| 8 | Trifecta audit recorded; one leg cut (14.1) | **Satisfied** | Recorded in every profile; the outbound leg is cut at the container network. `the_trifecta_decision_is_the_same_at_every_tier`. |
| 9 | Provenance tagging + data-block wrapping (14.2) | **Satisfied** | `provenance.rs`. Unknown sources default to untrusted; content cannot forge its own delimiters. |
| 10 | Risk-tiered permission policy with persistent HITL (14.3) | **Satisfied** | `policy.rs` keyed on risk × archetype × provenance; `approval_requests` persists across restarts and resumes. |
| 11 | Third-party tools/skills pinned, reviewed, scanned (14.4) | **Partial** | SDK pinned at 0.2.152; `leakage_lint::lint_skill` scans skill bodies. No skill-admission scan for credential/exfiltration requests yet (ADR-0028, Set R). |
| 12 | Background execution: checkpoint, resume, cancel, notify (15.1) | **Satisfied** | Job service with leases and a reaper; session inbox for steering and cancel; SSE timeline. |
| 13 | Uniform interface contract decoupled from UI (15.2) | **Satisfied** | `submit`→`job_id`, `status`, `pending_approvals`, `result`. The workspace UI is a skin over it. |
| 14 | Cost/latency budgets enforced (15.3) | **Satisfied** | `budgets.*` in the profile; the LLM proxy checks spend **before** the upstream call. |
| 15 | Full-task replayable tracing (15.4) | **Partial** | `trace.rs` defines the spans and `is_replayable` states the bar. Not every call site emits one yet. |

---

## Deviations, stated (procedure step 6)

**1. The Execution Loop is the Agent SDK's, not ours (1.1).**
ADR-0024 and D-07 fix the harness as the Claude Agent SDK in a container. Everything
the guide calls harness-side moved to the platform, above the container, where the
agent cannot reach it (D-10). The consequence is that components 1 and 4 of the six
are split: the SDK owns the loop, `crates/jobs` owns the state store.
*No eval justifies this; it is a locked decision.*

**2. `max_steps: 400` on the frontier profile (Appendix A: 30).**
Appendix A is calibrated for a task that answers a question. A task here is a
multi-hour research session that legitimately makes hundreds of tool calls. The real
guards are the cost budgets and the trial counter, which raises the significance bar
as the search widens; `max_steps` is the backstop against a loop. *A domain
judgement, not an eval result. The eval suite records steps per task so it can be
tightened on evidence.*

**3. `max_retries_per_call: 3` on frontier (Appendix A: 2).**
One retry budget across all tiers, so the ladder behaves identically everywhere. The
third attempt costs one turn.

---

## Anti-pattern 4, and how it is closed

**"Never send an unbudgeted prompt. No code path may bypass the Context Manager."**

The Agent SDK assembles its own prompts inside the container, so the platform's
Context Manager is not the only assembler. That would leave a code path the budget
does not cover.

The proxy closes it. Every upstream call passes through `llm_proxy::messages`, which
already checks dollar spend before forwarding; it now also estimates input tokens and
**refuses** anything over the profile's `input_budget_tokens` with a
`context_budget_exceeded` and the two numbers.

Refuse rather than truncate, deliberately. Silently trimming a prompt the SDK built
would remove something it believed it had sent, and the resulting answer would read
as a model failure rather than a budget one. A refusal that names the number is a bug
report.

The estimate is ~4 bytes per token over the system prompt, the messages and the tool
schemas — an over-estimate for dense English and JSON, which is the right direction:
a budget enforced with an optimistic estimate is not enforced.

---

## What a reviewer should check first

1. `crates/harness/tests/conformance.rs::every_profile_field_is_enforced_somewhere` —
   the test that makes the profile more than documentation.
2. `crates/mcp-server/src/taxonomy.rs::every_tool_is_classified` — the test that
   stops a new tool reaching a model unclassified.
3. `crates/harness/tests/failure_injection.rs` — whether this system degrades
   correctly, which the guide says is the production-readiness bar.

---

## The local tier (ADR-0032)

Part 3 was recorded as **Deferred** in the first pass, on the honest ground that no
local model was wired and a stub that passed `verify` would be worse than none. It is
now built, and three things about it are worth stating because they were decided by
measurement rather than by reading the guide.

**The decode is two constrained calls, not one.** A single `anyOf` grammar over the
exposed tools produced valid JSON naming the **wrong tool**, 0/2
(`LOCAL_TIER_FINDINGS.md` §4) — output that passes every rung of the ladder and then
does the wrong thing. Splitting it into *select against an enum* then *fill the chosen
schema* gave 3/3. The structural consequence is the one that made it permanent:
**step 1's enum is exactly `ToolRegistry::expose()`'s output, so the exposure budget
IS the decoding grammar** and an unrouted tool is unnameable rather than rejected.
Checklist item 4 is therefore enforced twice, once after the fact and once at
sampling time.

**The loop is sans-IO, and that is a testability decision.** `Loop::next(&mut self,
Input) -> Vec<Effect>` names no runtime, client or provider
(`the_loop_names_no_runtime_no_client_and_no_provider` fails the build if that
changes). The payoff is in `failure_injection.rs`: a backend restarting mid-session, a
model evicted from VRAM, guided decoding falling back on step four, a second GPU that
is not actually pooled — each is a `Vec<Input>` and each runs green on a machine with
no accelerator.

**A constraint violation is evidence, not noise.** Under a grammar the sampler cannot
emit a token the grammar forbids, so a non-conforming reply is not the model failing
to follow instructions — it is proof the grammar was not applied. `validation::conforms`
is deliberately strict where `check_schema` coerces: repairing `"500"` to `500` is
right for a model's guess and wrong for a backend's lie, because the repair destroys
the only evidence available. One violation fences the session.

### What is still deferred here

| Item | State |
|---|---|
| A second GPU, pooled | Not present. Detection reports `pooled: false` unless declared, so the reference model is refused at startup rather than OOMing later. |
| `local_high` promotion | Blocked on evals against the target hardware. Promotion is by evidence (§6.2), not by config edit. |
| One-call decode for larger models | Open (`LOCAL_TIER_FINDINGS.md` §6). Two-step stays regardless: even if a 32B selects correctly in one call, one call gives up the grammar-is-the-budget property. |
