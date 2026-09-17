# AGENT-005: Context Assembly — one budgeted pipeline for every model tier

**Status:** Proposed (implementation spec; ready to build)
**Date:** 2026-09-14
**Author:** Claude (Fable 5.1), from a code audit of `crates/harness`, `crates/api/src/agent`, `crates/llm`, `crates/mcp-server`
**Supersedes, for the harness path:** AGENT-002 §2 P2/P10 ("no tool schemas in context", "nothing loads mid-session") and BS-007 04 §3 ("the tool list never changes within a session"). See ADR-0034.
**Amends:** AGENT-001 §8.2 (prompt structure), HARNESS_CONFORMANCE item 10 (fixed layout)
**Part II (§13–§30)** redesigns assembly as retrieval over a run store (ADR-0036). Part I is the arithmetic and remains true; Part II becomes `ContextPolicy v2` and ships only by beating Part I on the eval suite.
**Implementer notes:** every section marked **[DO]** is a concrete change with file and line anchors. Line numbers are as of 2026-09-14 and may drift by a few lines. Anchors are on symbols, so search by name if a line has moved.

---

## 0. Why this spec exists (the measured problem)

The platform has two agent drivers and they assemble context in two unrelated ways. Neither one sizes what it sends to the model's *effective* window, and both waste the majority of every call on bytes the model has already read.

| Measured on 2026-09-14 | Hosted driver (`crates/api/src/agent/driver.rs`) | Local driver (`local_driver.rs` + `crates/harness`) |
|---|---|---|
| System prompt per call | 15.5 KB, byte-identical every call, uncached (`prompt.rs:8`, `driver.rs:532`) | same 15.5 KB + 370 B workspace hint, re-sent on **every decode** (`drive.rs:1227`), two decodes per step |
| Of which the authoring guide | 10.7 KB (69%) inlined at `prompt.rs:64`, *also* served by the `get_authoring_guide` tool | same |
| Tool schemas per call | all 56 tools, 35.6 KB ≈ 9–10k tokens, every call (`driver.rs:148`, `:534`) | ≤ 6 routed, flat (`drive.rs:1070`) — correct |
| Static overhead per call | **≈ 51 KB ≈ 13k tokens** | ≈ 16 KB charter + schemas |
| Share of a 7B's 14,000-token input budget taken by the charter alone | n/a | **28%** (declared share: 8%, `context.rs:59`) |
| Prompt caching | none (`llm/src/anthropic.rs:85-123` emits no `cache_control`) | n/a |
| Routing / `search_tools` | none | yes |
| Prior-turn conclusions | computed (`manager.rs:253`), passed (`driver.rs:33`), **never read** | prepended to goal (`local_driver.rs:506`) |
| Tool result cap | 32 KB per result, unbounded count (`driver.rs:44`) | profile `max_tool_result_bytes`, overflow → scratchpad handle (`drive.rs:1951`) |
| Compaction | at 95% of budget, old results → `"[elided — was N bytes]"` with **no handle** (`driver.rs:300`) | drops only blocks that carry a `reference` (`context.rs:310`) — correct |
| Tool schemas counted against the budget | no | **no** — nothing ever pushes `Section::Tools`; the 20% share is dead (`drive.rs` has no `Section::Tools` push) |
| Findings | n/a | each `record_finding` pushes a *new* block re-rendering **all** findings → quadratic (`drive.rs:1751`) |
| Model summarisation | none | `needs_model_summary` is set (`context.rs:389`) and never consumed |
| Tokenizer | none; `len/4` (`context.rs:123`); provider usage is read after the fact and never fed back | same |
| Prompt tells the agent to use tools it does not have | yes: "BUILD IT WITH THE DRAFT BUILDER — new_strategy_draft…" (`prompt.rs:31`) while `INTERNAL_AGENT_TOOLS` excludes the builder (`mcp-server/src/lib.rs:495`) | same |

Consequences: a 50-iteration hosted research session re-reads ~650k tokens of unchanged prefix at full price; a 7B model gets roughly half the working state its profile promises; and the harness's own budget arithmetic is wrong in the direction that produces silent host-offload and left-truncation on Ollama (`local_driver.rs:690` sets `num_ctx` to the full budget the schemas were never counted against).

## 1. Principles (normative)

- **P1 — One budget, honestly counted.** Every byte that reaches the model is charged against `profile.context.effective_budget_tokens − reserve_for_output` before the working state is sized: charter, tool schemas (however transported), the opening message, the transcript. Nothing rides for free.
- **P2 — Stable-first layering.** The request is assembled most-stable-first: tools → charter → opening message → history → per-step subtask. Anything that varies per turn or per session lives at the end. This is what prompt caches key on and what keeps local prompts reproducible.
- **P3 — Route, never delete.** The catalogue may be any size. The model sees a slice sized by its profile. The slice is chosen by routing (namespaces + deterministic search over the goal), extended by `search_tools`, and never silently truncated in alphabetical order.
- **P4 — Frontier is a profile, not a code path.** The hosted driver uses the same registry, the same exposure function, the same reservation arithmetic and the same elision-with-handle as the local loop. Where it differs (native multi-turn transcript, prompt caching, larger budgets) the difference is a profile value or a transport detail, not a second assembler.
- **P5 — Rigor in the tool, not the model.** A small model never computes a budget, summarises a transcript, or decides what to drop. The harness does those deterministically; the model chooses tools and reads structured output. A large model gets the same guarantees and may additionally be asked to summarise.
- **P6 — Loss is loud and recoverable.** Anything cut from context leaves a handle the model can re-read (`work/<name>-step<N>.txt`). "[elided — was N bytes]" with no handle is forbidden.
- **P7 — Measure with the provider's count.** Every call records `prompt_tokens` as the provider reports it and the harness's own estimate for the same request. The ratio is persisted per model and used to calibrate the estimator. The 4-bytes-per-token constant is a starting value, not a fact.

## 2. The context model

### 2.1 Layers, in prefix order

| # | Layer | Stability | Owner | Size rule |
|---|---|---|---|---|
| L0 | **Tool schemas** | session-stable, **append-only** on frontier; per-step on local | `harness::registry` | ≤ `profile.tools.max_exposed_per_step` entries; total bytes counted (§4) |
| L1 | **Charter** | byte-stable across all sessions of a build | `api::agent::prompt::charter()` | ≤ 3,200 B, test-enforced (§3.1) |
| L2 | **Workspace map** | byte-stable | `workspace_tools::areas_hint()` | ≤ 600 B |
| L3 | **Opening message** | per turn | `prompt::opening_message()` | prior-turn conclusions + run constraints + goal (§3.2) |
| L4 | **Working state** | per call | hosted: provider transcript; local: `context` blocks | what is left after L0–L3 are reserved (§4) |
| L5 | **Subtask / anchor** | per step (local only) | `drive.rs` `anchor()` | unchanged |

L0 comes before L1 because that is the order Anthropic reads the cache prefix (tools, system, messages) and OpenAI keys automatic prefix caching the same way. Any change to L0 invalidates L1–L4's cache; that is why L0 is append-only on the frontier tier (§5.2).

### 2.2 Budget arithmetic (both drivers)

```
input_budget   = effective_budget_tokens − reserve_for_output          (exists: profile.rs:508)
reserved       = est(charter + workspace_map) + est(exposed schemas as sent)
body_budget    = max(input_budget − reserved, input_budget × MIN_BODY_SHARE)   MIN_BODY_SHARE = 0.25
trigger        = body_budget × COMPACTION_TRIGGER                        (exists: 0.95, context.rs:111)
section shares = shares of body_budget, not of input_budget
```

`est()` is `context::estimate_tokens` scaled by the per-model calibration factor from §7; until a factor exists it is `len/4`.

**A single tool result may never exceed the room left for it.** `max_tool_result_bytes` is a
per-profile constant and the body budget is computed per call, so the two can disagree: a
32 KiB cap against a body budget that has 8 KiB left admits one result that evicts everything
else in the working state, including the findings the run has established. The effective cap
is therefore `min(profile.max_tool_result_bytes, remaining_body_bytes)`, computed at the
moment the result arrives, and the overflow goes to the scratchpad with a handle exactly as
any other overflow does. The profile constant is a ceiling, not an entitlement.

If `reserved > input_budget × (1 − MIN_BODY_SHARE)` the harness emits `Note::BudgetSqueezed { reserved, input_budget }` and, on a local profile, reduces exposure by one tool per step until it fits or hits the core set. This is the "schemas do not fit this model" failure surfacing instead of a silently empty working state.

## 3. The prompt

### 3.1 [DO] Split `system_prompt` into a lean charter — `crates/api/src/agent/prompt.rs`

Replace `system_prompt(constraints) -> String` with:

```rust
pub fn charter() -> String;                         // byte-stable, ≤ CHARTER_MAX_BYTES
pub const CHARTER_MAX_BYTES: usize = 3_200;
pub fn run_constraints(constraints: &Value) -> Option<String>;
pub fn opening_message(prior: &str, constraints: &Value, goal: &str) -> String;
```

`charter()` contains, in this order, and nothing else:
1. Identity and the "answer the goal you were given" paragraph (keep verbatim from `prompt.rs:10-20`).
2. A **compact** edge-finding section (≤ 900 B) that says: call `get_authoring_guide` first; every Study is a counted trial and the holdout is locked; never pick a parameter value (narrow ranges, the sampler chooses, build on `carried_forward`); one change at a time, versioned slugs; `validate_strategy` before `create_strategy`; no raw backtests, Experiments are the only path; prefer `run_sweep`/`run_study` over polling; a failed gate is a finding.
3. A **tools** section (≤ 300 B): "You see a slice of the catalogue chosen for the task. If the tool you need is not offered, call `search_tools` with what you want to do and it will be added. Do not work around a missing tool with a different one."
4. The termination contract (keep verbatim from `prompt.rs:93-107`).

Remove from the charter:
- the full five-step research protocol (`prompt.rs:21-61`) → moves into the guide (§3.3);
- the inlined authoring guide (`prompt.rs:64`);
- **the draft-builder instruction** (`prompt.rs:31-38`). It names tools the internal profile does not expose (`mcp-server/src/lib.rs:479-481` explains why they were excluded). Replace with "`validate_strategy`, then `create_strategy` with the definition object".
- run constraints (`prompt.rs:66-91`) → `run_constraints()`, rendered into the opening message. Drop the "asset_class: crypto_spot_cex" line and the "your choice — start on 1h" advice; the first is stale (11 asset classes are the target) and the second is protocol, which belongs in the guide.

Tests (in `prompt.rs`):
- `the_charter_stays_small_enough_for_a_7b_window`: `charter().len() <= CHARTER_MAX_BYTES` and `charter() == charter()`.
- `the_charter_points_at_the_guide_and_at_real_tools`: contains `get_authoring_guide`, `search_tools`, `finish_task`, `validate_strategy`; does **not** contain `new_strategy_draft` or `## Strategy definition`.
- `constraints_render_into_the_opening_message_not_the_charter`.

### 3.2 [DO] Opening message

`opening_message(prior, constraints, goal)`:
- `prior` (from `conversations::prior_context`, `manager.rs:253`) first, if non-empty;
- then `run_constraints`, if any;
- then `The user now asks: {goal}`; if neither prior nor constraints, the goal alone.

Both drivers seed their first user message with this. This fixes the hosted driver silently discarding `DriverParams::prior` (`driver.rs:33`, never read).

### 3.3 [DO] Move the research protocol into the served guide — `crates/mcp-server/docs/authoring_guide.md`, `crates/mcp-server/src/lib.rs:615`

Prepend a `## The research protocol` section to the guide (the five steps from `prompt.rs:23-61`, reworded to say `validate_strategy` then `create_strategy`, and to keep the builder mention only in the MCP-client sense the guide already has at step 2). Update the `get_authoring_guide` description to: "Return the research protocol (the edge-finding loop: model, design, experiment, sweep, diagnose, validate) and the full strategy-authoring protocol… Call this once before authoring a strategy or starting a research loop; not needed for lookups."

`get_authoring_guide` is namespace `core`, risk `read`, and is 10.7 KB. On the local tier it enters context as a tool result and is therefore subject to `max_tool_result_bytes` (8,192 / 6,144 B) — it **will be cut**. Two options; do both:
- (a) split the served guide into `get_authoring_guide(section: "protocol" | "definition_format" | "expressions" | "features" | "results")` with a default of `"protocol"`; each section ≤ 4 KB;
- (b) when a section is still over the cap, the harness writes it to `work/authoring_guide-<section>.txt` and returns the head plus the handle, exactly as any other oversized result.

## 4. Budget accounting in the harness — `crates/harness/src/context.rs`, `drive.rs`

### 4.1 [DO] `assemble_reserving`

```rust
pub fn assemble(profile: &Profile, blocks: Vec<Block>) -> Assembled { assemble_reserving(profile, blocks, 0) }
pub fn assemble_reserving(profile: &Profile, blocks: Vec<Block>, reserved_tokens: u32) -> Assembled;
pub const MIN_BODY_SHARE: f32 = 0.25;
```

`assemble_reserving` computes `input_budget = max(full − reserved, full × MIN_BODY_SHARE)` and uses it for the trigger (`context.rs:245`) and the section shares (`context.rs:368`). Add `reserved_tokens: u32` to `CompactionReport` (serde default) so the trace shows it.

Test: with a 10,000-token budget and 4,000 reserved, blocks totalling 6,500 tokens compact (trigger 5,700) where they would not have at 9,500.

### 4.2 [DO] Reserve the schemas in `working_prompt` — `drive.rs:1124`

```rust
let reserved = self.exposed_schemas.iter().map(|s| context::estimate_tokens(&s.to_string())).sum::<u32>();
let assembled = context::assemble_reserving(&self.profile, self.blocks.clone(), reserved);
```

Do **not** also reserve the charter here: it is already a `Section::Charter` block (`drive.rs:621`) and is counted. Instead fix the double-standard the other way:

### 4.3 [DO] Send the *assembled* charter, not the raw one — `drive.rs:1227`

`ask()` currently sends `system: self.task.charter.clone()` while `working_prompt` discards the (possibly truncated) Charter section. Change `working_prompt` to return `(system, body)` where `system` is the Charter section text from `assembled.sections` and `body` is the rest; `ask()` sends that `system`. With the lean charter from §3.1 this will never truncate in practice, but the path that could silently exceed the budget is closed.

### 4.4 [DO] Findings: one block, rewritten — `drive.rs:1751`

Before pushing the `"findings"` block: `self.blocks.retain(|b| b.tagged.source != "findings");`. Add a test in `drive.rs` that after N `record_finding` calls exactly one block has source `findings` and it lists N lines.

### 4.5 [DO] `Section::Tools` and `FewShot` — decide, do not leave dead

- `Section::Tools`: delete the section and its 20% share. Schemas are accounted by §4.2 reservation; a section nothing renders is a lie in the layout. Re-normalise shares: Charter 0.10, Subtask 0.10, FewShot 0.05, WorkingState 0.55, Reference 0.20 (sum 1.0).
- `FewShot`: `few_shot_examples: 2` is declared in both local profiles and read only by `appendix_a.rs`. Either wire it (push two canned select→fill exemplars as a `FewShot` block on `constrained_decoding` profiles, source `"few_shot"`, priority 150) or set it to 0 in the profiles and delete the field. Recommendation: wire it — LOCAL_TIER_FINDINGS §4 shows tool *selection* is where a 7B fails, and two exemplars of the select→fill shape are cheap (≤ 400 tokens) insurance. Conformance test `tests/conformance.rs` must then find a reader.

### 4.6 [DO] Model summarisation for tiers that can do it — `drive.rs`, `local_driver.rs`

When `assembled.compaction.needs_model_summary` is true **and** `profile.tier` is `frontier` or `local_high`: emit `Effect::Summarize { blocks: Vec<Block> }` (the lowest-priority WorkingState blocks that were truncated), the driver runs one model call with a fixed prompt ("Summarise these tool results into ≤ 300 tokens, keeping every id, every number with its n, and every rejected idea with its reason"), and feeds back `Input::Summary { text, replaces: Vec<source> }`, which becomes one WorkingState block at priority 200 with `reference` set to the scratchpad file the originals were written to. For `local_mid`/`local_small`/`degraded`: deterministic only (today's behaviour); the flag is logged.

The content contract for the summary is AGENT-001 §10.2 (ids, n/SE, gate states, rejected ideas, current step). Put that list in the fixed prompt verbatim.

## 5. Tool exposure

### 5.1 Per-tier policy (profile values; the code reads them, nothing hard-codes a tier)

| Profile field | frontier (`claude-opus-5`) | local_high/mid (`qwen3.6-35b`) | local_small/degraded (`qwen2.5-7b`) |
|---|---|---|---|
| `tools.routing` | `dynamic` | `dynamic` | `dynamic` |
| `tools.max_exposed_per_step` | 24 | 8 (raise from 6: two-step decode makes 5 work slots affordable) | 6 |
| `tools.schema_style` | `rich` | `flat` | `flat` |
| **new** `tools.exposure_scope` | `session` | `step` | `step` |
| **new** `tools.search_reserve_slots` | 4 | 1 | 1 |
| `output.constrained_decoding` | false | true | true |
| `orchestration.few_shot_examples` | 0 | 2 | 2 |

`exposure_scope: session` means L0 is computed once per turn from the opening message and only grows (§5.2). `exposure_scope: step` is today's local behaviour (`drive.rs:1006-1090`).

[DO] Add `exposure_scope: ExposureScope { Session, Step }` and `search_reserve_slots: usize` to `ToolPolicy` (`profile.rs:117`), with a conformance-test reader for each (the crate's rule: a profile value nothing enforces fails `tests/conformance.rs`).

### 5.2 [DO] Session-scoped exposure on the hosted driver — `crates/api/src/agent/driver.rs`

Replace `tool_defs()` (`driver.rs:148`) with a `SessionExposure` owned by the run:

```rust
struct SessionExposure {
    registry: ToolRegistry,            // bridge::registry_with_workspace() when a conversation workspace exists, else bridge::registry()
    profile: Profile,
    tools: Vec<ToolDef>,               // L0, in exposure order, append-only
    last_used: HashMap<String, u32>,   // iteration a tool was last called; for eviction only
}
impl SessionExposure {
    fn open(registry, profile, opening_message: &str) -> Self;
    fn defs(&self) -> Vec<llm::ToolDef>;                 // what goes in ChatRequest.tools
    fn search(&mut self, query: &str, iteration: u32) -> SearchOutcome;   // the search_tools handler
    fn note_used(&mut self, name: &str, iteration: u32);
}
```

`open`:
1. `wanted = ["core", "discovery"] ++ namespaces of registry.search(opening_message, 3)` — the same priority order `drive.rs:1006-1048` uses, minus the planner.
2. `budget = max_exposed_per_step − search_reserve_slots`.
3. `expose_pinned(profile_with_budget, &wanted, &[])`; drop `record_finding` (no findings blocks on this path) and replace the registry's `finish_task` with the driver's richer `finish_task_def()` (`driver.rs:178`), keeping its position.
4. Record an `agent_messages` row `kind = "exposure"` with `{namespaces, exposed, budget}` — the hosted equivalent of `Note::Exposure` (`drive.rs:1087`).

`search`:
1. `registry.search(query, 5)`.
2. For each hit not already in `tools`: if `tools.len() < max_exposed_per_step`, **append** (rich render → `llm::ToolDef`).

   **If the slice is full, do not evict mid-turn.** Tools are the *first* layer of the
   cached prefix, so removing one invalidates the cache for the system prompt and the
   entire transcript behind it — on a 30-iteration run that is worse than the tool being
   absent. Instead: return the hits as text with the sentence "the tool slice is full for
   this turn; finish what you are doing or say what to drop", and record
   `kind = "exposure", full = true`. **Eviction happens only at a turn boundary**, where the
   prefix is rebuilt anyway: the next turn's slice is re-routed from the opening message,
   least-recently-used tools drop, and anything the model asked for in the previous turn is
   pinned ahead of namespace filling.

   A slice that fills repeatedly is a routing failure, not a budget failure. It is reported
   as `exposure_full` in the scorecard (AGENT-009 §4) so it is tuned with evidence rather
   than by raising the cap.
3. Return a tool result listing `{name, namespace, one-line description}` for each hit and the sentence "These tools are now offered; call them directly." Never return schemas as text — the model cannot call from prose.
4. `MAX_SEARCHES_PER_ITERATION = 2` (mirror `drive.rs:478`).

Because L0 only ever appends, the Anthropic tools breakpoint (§6.1) and OpenAI's automatic prefix cache stay valid across the session except at the rare eviction.

### 5.3 [DO] Workspace tools on the hosted driver

Register via `bridge::registry_with_workspace()` when `params.conversation_id` is `Some` (build `ws = workspace_tools::workspace_for(&params.workspace_root, conversation_id)` and `ensure(&ws)` exactly as `local_driver.rs:494-495`). Dispatch: if `registry.get(name).namespace` is a workspace namespace (`workspace_tools::is_workspace_tool`), call `workspace_tools::execute(&ws, name, args)` as `local_driver.rs:620-631` does; otherwise `dispatch_tool` as today. This is what makes §6.3's handles readable on the hosted path.

### 5.4 [DO] Schema weight budget — `crates/mcp-server/src/lib.rs` and `tools/*.rs`

Add a test in `mcp-server` that every tool definition's rich `{name, description, inputSchema}` serialises to ≤ 1,200 bytes, and that the flat render (`registry::render(_, Flat)`) is ≤ 400 bytes. Today's offenders (measured, minified): `create_skill` 2,123; `create_experiment` 1,950; `run_sweep` 1,862; `report_kill` 1,658; `run_study` 1,595; `check_tradeable_frontier` 1,560; `train_model` 1,530; `report_evaluator_defect` 1,474; `web_search` 1,450; `fetch_url` 1,212. Fix by moving "how to think about this" prose out of descriptions into the guide (§3.3, a new `## Tool notes` section) and keeping descriptions to *when to use / returns / cost*. The `run_skill` description alone is 5.2 KB of source; it becomes a guide section.

Rule for new tools, enforced by the same test: a description is ≤ 2 sentences that say WHEN to use it; anything longer is documentation and goes in the guide.

## 6. Transport

### 6.1 [DO] Prompt caching — `crates/llm/src/anthropic.rs:85` (`build_body`), `types.rs:130` (`Usage`)

- `system` becomes `[{ "type": "text", "text": …, "cache_control": { "type": "ephemeral" } }]`.
- The **last** tool in `tools` gets `"cache_control": {"type":"ephemeral"}`.
- The last content block of the **last** message gets `cache_control` (a plain-string user message is lifted into a single text block first). Three breakpoints total, in prefix order tools → system → messages. Segments under the provider's minimum (1,024 tokens) are ignored by the API, so this is safe on tiny prompts.
- `Usage` gains `cache_read_input_tokens` and `cache_creation_input_tokens` (`#[serde(default)]`), a `prompt_tokens()` helper = sum of all three, and `anthropic.rs:211` parses them from `/usage/…`. `openai.rs:231` reads `/usage/prompt_tokens_details/cached_tokens` into `cache_read_input_tokens`; `ollama.rs:169` sets both to 0. Every `Usage { .. }` literal in tests uses `..Usage::default()`.
- No beta header is required for `cache_control` on current models; keep `anthropic-version: 2023-06-01`.
- Test: `cache_breakpoints_close_each_prefix_layer` — only the last tool, the system block, and the last message block carry the marker.

The 1-hour TTL rewrite already exists for the container agent in `crates/api/src/llm_proxy.rs:55` (`rewrite_cache_ttl`). Do **not** duplicate it in the client; instead, Phase 3 (§9) routes the internal driver's Anthropic base URL through the same proxy so both agents get the same TTL policy and the same `Usage` accounting.

### 6.2 [DO] Local transport unchanged, but honest — `local_driver.rs:690`

`num_ctx` stays `effective_budget_tokens`. With §4.2 the body is now sized so that charter + schemas + body ≤ `num_ctx − reserve_for_output`; add a debug assertion in `local_driver` that `est(system) + est(prompt) + est(schemas) ≤ num_ctx − reserve_for_output`, logging (not panicking) on violation with the three numbers. That log line is the regression detector for left-truncation.

### 6.3 [DO] Elision keeps a handle on the hosted driver — `driver.rs:276` (`compact_transcript`)

Split into a pure `elision_candidates(messages, limit) -> Vec<usize>` and an async elide step in `drive()`. For each candidate whose content is > 200 B: write the full content to `work/result-<tool_call_id>.txt` in the conversation workspace (when one exists) and replace the content with `[elided — {n} bytes; full text at work/result-{id}.txt — read_file to recover]`. Without a workspace, keep today's marker but also log `kind = "elided_without_handle"` so it is visible that P6 was not met.

Also cap **count**, not only size: keep `KEEP_RECENT_TOOL_RESULTS = 6` but elide any result older than the last 6 whose size exceeds `profile.context.max_tool_result_bytes / 4` as soon as the transcript passes 50% of body budget, so the 95% trigger is reached later and with smaller steps. (Compaction should be gradual pressure, not one cliff.)

### 6.4 [DO] The native transcript is managed, not merely appended

`driver.rs` held the only transcript elision on the hosted path, and AGENT-010 deletes that
file. Without this section the native transport would grow a provider-side transcript with
no bound at all — a regression hidden inside an improvement.

On `decode: native`, the adapter owns an append-only message list, and the **context
manager owns its size**. Before each call:

1. Sum the transcript with the calibrated estimator (§7) and add the reserved layers.
2. Under `COMPACTION_TRIGGER × input_budget`, send it untouched.
3. Over it, elide oldest-first, tool results before assistant prose, **never** the opening
   message and never the last `KEEP_RECENT_TOOL_RESULTS`. Every elided body is written to
   `work/result-<tool_call_id>.txt` and replaced by a line naming the size and the handle —
   P6 applies here exactly as it does to the block path; `"[elided — was N bytes]"` with no
   handle is forbidden on every transport.
4. Elision rewrites history, so the message-layer cache breakpoint moves to the new last
   block and the prefix behind it is re-read once. Elide in **batches at a step boundary**,
   not per message, so that cost is paid once rather than per call.
5. Report it as a `Compacted` note with the same `CompactionReport`, so one metric covers
   both transports.

A run whose transcript needs elision twice in ten steps is emitting results that are too
large; that shows up as `max_tool_result_bytes` pressure in the scorecard and is fixed at
the tool, not here.

## 7. Token estimate calibration (P7)

[DO] Per call, both drivers already receive `Usage`. Record on `agent_runs` two new columns (migration): `est_prompt_tokens BIGINT` (harness estimate of the request just sent: est(system)+est(messages)+est(schemas)) and `prompt_tokens BIGINT` (provider's `prompt_tokens()`). Nightly (or on run finish), compute per `model` the **p90** ratio `prompt_tokens / est_prompt_tokens` over the last 50 runs and store it in a new table `agent_token_calibration(model TEXT PRIMARY KEY, bytes_per_token REAL, sample_n INT, updated_at)`. `context::estimate_tokens` gains a `_with(bytes_per_token: f32)` variant; the drivers pass the calibrated value when one exists (clamped to [2.5, 5.0]). The default for an unseen model is **per provider family, not a single constant**: 4.0 for
hosted English-and-JSON (Anthropic, OpenAI), **3.2 for local open-weight models**, whose
tokenizers fragment JSON punctuation and long identifiers far more than a BPE trained
mostly on prose. A single 4.0 is optimistic exactly where the window is smallest, and an
optimistic estimate is not an estimate — it is an unenforced budget. The family prior is a
`ProviderFamily → f32` table read at profile load and overridden the moment a measured row
exists.

**p90, not the median, and the distinction is the whole point.** P7 promises the estimator is
an over-estimate; calibrating to the median delivers an estimate that is too low on half of
all calls, which is not a budget but a coin flip. The p90 ratio keeps the guarantee at the
cost of leaving a little window unused, and the direction of that error is the safe one: a
slightly small body is a worse answer, a slightly large one is a truncated prompt or a silent
host offload. The `budget` suite (AGENT-009 §3) asserts the over-estimate rate stays ≥ 95%
and reports it, so the choice is measured rather than assumed.

This keeps P7's guarantee — the estimator stays an *over*-estimate unless the provider has shown otherwise for that exact model — and it is the only way the 7B's declared `effective_budget_tokens: 16000 → input 14000` ever gets checked against reality.

## 8. Code execution as the escape valve for catalogue growth

The question behind "why does the agent need so many tools" has an answer already half-built: an agent that can run Python needs far fewer bespoke tools. State of the repo:

- `run_skill` (`crates/skills/src/runner.rs:152`, `apps/skill-runner/skill_runner.py:176`) executes a `def run(payload) -> dict` in a subprocess with numpy/pandas, no network, 30 s / 512 MB limits, POSIX rlimits only (nothing on Windows dev boxes except the wall clock). It is exposed to the internal agent.
- The container agent (`apps/agent-host`, ADR-0024) has full Bash and the whole research stack, but it is Claude-Agent-SDK-only, its image (`infra/agent-image/Dockerfile`) is built by nothing, and it does not route through this harness — so it cannot serve the 7B-to-frontier requirement.
- `profile.orchestration.code_execution: Denied | Templated | Full` exists (`profile.rs:164`) and is enforced (`profile.rs:398`); Appendix A sets Frontier/LocalHigh `Full`, LocalMid `Templated`, LocalSmall/Degraded `Denied`.

[DO] Add one tool, `run_python`, to the harness catalogue (namespace `code`, risk `write`), backed by the **existing** skill runner with the `run(payload)` wrapper removed (a script + a JSON payload on stdin + JSON on stdout), same sandbox, same limits, same run record. Gate it on `code_execution`: `Full` exposes it; `Templated` exposes it with a fixed set of scaffolds (`analysis.describe_returns`, `analysis.event_study`, …, i.e. the unbuilt AGENT-002 §6.3 suite implemented as Python templates the model fills parameters into); `Denied` hides it. Its result obeys `max_tool_result_bytes` and the handle rule like any other. This is how the catalogue stops growing one bespoke tool per statistic: the analysis suite becomes templates, the frontier tier writes its own, and the small tiers get the same rigor from the same code.

Not in this spec: sandboxing `run_python` against intent (ADR-0024's container is the answer there); a REPL/kernel; anything that changes the "no network in a skill" invariant (`taxonomy.rs:285` test).

## 9. Build order

| Phase | Items | Why first |
|---|---|---|
| **1 — stop the bleeding** (≈ 1 day) | §3.1 charter split, §3.2 opening message, §3.3 guide, §6.1 caching, §4.2 reservation, §4.4 findings | Removes ~11 KB from every call at every tier and ~35 KB from every hosted call once §5.2 lands; no behaviour change the model would notice except a shorter prompt |
| **2 — route the frontier** (≈ 2 days) | §5.1 profile fields, §5.2 session exposure + `search_tools` on hosted, §5.3 workspace tools on hosted, §6.3 elision with handles, §4.3, §4.5 | Makes P4 true: one exposure function, one loss rule |
| **3 — measure** (≈ 1 day) | §7 calibration columns and table, §6.2 assertion, route the internal driver through `llm_proxy` for TTL + usage | The numbers that tell us whether phases 1–2 worked, per model |
| **4 — schemas and summaries** (≈ 2 days) | §5.4 description budget and rewrites, §4.6 model summarisation on capable tiers, §4.5 few-shot wiring | Finishes the 7B story: schemas that fit, exemplars for selection, and a working state that does not silently starve |
| **5 — code** (≈ 3 days) | §8 `run_python` + templated analysis suite | Turns catalogue growth from a per-tool cost into a per-template one |

## 10. Acceptance (the whole spec passes when all hold)

> **Form note (Set S task S-0.4, 2026-09-15).** Converted from a plain numbered
> list to the repository's acceptance form so `docs/procedures/execute-plan.md`
> step 3c can close the chain requirement -> criterion -> evidence. No criterion
> was added, removed, reworded or softened; each kept its position, so item *n* of
> the old list is `AC-n`. An unticked box with `Verified by: [--]` means nothing
> has been proved, which is the honest default.
>
> The plan's S-0.4 gives the target form as `6.1.A [ ] ... - Verified by: [--]`.
> The form actually used across the tracked specs (DATA-003, COMP-001..004,
> FEAT-001, SYS-001) is `- [ ] AC-n: ... - Verified by: [--]`, and that is what
> execute-plan step 3c ticks. This uses the tracked form; the plan's example is
> stale.

- [ ] AC-1: `prompt::charter().len() ≤ 3,200` and is byte-stable. No test or prompt names a tool the internal profile does not expose. - Verified by: [--]
- [ ] AC-2: A hosted 30-iteration run with `claude-opus-5` shows, from `Usage`: `cache_read_input_tokens ≥ 0.8 × prompt_tokens()` averaged over iterations 3–30, and uncached `input_tokens` per iteration ≤ 1.3 × the new tokens appended that iteration. - Verified by: [--]
- [ ] AC-3: Hosted L0 never exceeds `max_exposed_per_step`; its order is a prefix of the previous iteration's order except on a logged eviction; `search_tools` finds and appends a tool the initial routing missed in one round-trip (test with a goal that never mentions "skill" and a search for "run python"). - Verified by: [--]
- [ ] AC-4: On `qwen2.5-7b`, for a step with six flat schemas: `est(system) + est(schemas) + est(body) ≤ 14,000` and `body_budget ≥ 3,500` (the 25% floor), asserted in a `local_driver` test using `config/profiles/qwen2.5-7b-instruct.yaml`. - Verified by: [--]
- [ ] AC-5: After 20 `record_finding` calls there is exactly one `findings` block. - Verified by: [--]
- [ ] AC-6: No message in any driver's transcript ever reads `[elided — was N bytes]` without a `work/…` handle when a workspace exists. - Verified by: [--]
- [ ] AC-11: §8's `run_python` escape valve exists, runs outside the platform process, and is offered only on a `full` code-execution tier. *(Added 2026-09-17, S-8.8.)* - Verified by: `invariants::code_execution_is_tiered::{the_gated_namespace_is_the_one_the_tool_is_declared_in,only_a_full_tier_is_offered_the_code_tool}` over all three shipped profiles, `mcp-server-lib::tools::code::tests::{free_form_code_is_not_a_read,the_sandbox_cannot_run_the_simulator_so_it_spends_no_trial}`, and the `agent-sandbox-image` CI job, which builds the image and proves three refusals: a credential in the environment stops the container starting, a `data_path` outside the workspace is refused, and a plain script runs and returns its result. **Partial:** the per-conversation container *launcher* is a deployment contract (`TBOT_SANDBOX_CMD`), not a component here; with it unset the tool refuses rather than falling back to the host (ADR-S8-06).
- [x] AC-7: Every tool definition ≤ 1,200 B rich / ≤ 400 B flat. - Verified by: `mcp-server-lib::catalogue_tests::no_tool_descriptor_exceeds_the_rich_budget` and `every_first_sentence_fits_the_flat_budget`. Meeting it moved ~7.4 KB of prose from thirteen descriptions into the authoring guide's new `## Tool notes`.
- [ ] AC-8: `tests/conformance.rs` has a reader for `exposure_scope`, `search_reserve_slots`, and `few_shot_examples`. - Verified by: [--]
- [ ] AC-9: `agent_runs.prompt_tokens` and `est_prompt_tokens` are populated on every run; `agent_token_calibration` has a row for every model that has completed ≥ 10 runs. - Verified by: [--]
- [ ] AC-10: The 12 doc conflicts listed in ADR-0034 §3 are resolved by that ADR's amendments; `HARNESS_CONFORMANCE.md` item 10 cites this spec. - Verified by: [--]
## 11. Non-goals

- Unifying the hosted multi-turn driver and the sans-IO `Loop` into one execution loop (ADR-0033-scale; this spec makes them share the assembler, not the loop).
- A real tokenizer dependency. Calibration from provider counts (§7) is cheaper and is exact for the model that matters.
- Anthropic's server-side tool-search beta. The append-only L0 emulates it provider-neutrally; revisit if OpenAI/Ollama grow an equivalent.
- Changing `max_tool_result_bytes` values. They are per-profile decisions with rationale in the YAML.

## 12. Open questions for the implementer to record, not solve

- Whether `search_tools` on the hosted tier should also be offered the plan-step router (`registry.search(step.goal)`) once the hosted driver has plan steps; today it has none.
- The right `MIN_BODY_SHARE` for a 7B: 0.25 of 14,000 is 3,500 tokens of working state, which LOCAL_TIER_FINDINGS §8 suggests is enough for a 3-step task and not for a 15-step one. Calibration data (§7) should decide.
- Whether the `Templated` code-execution tier should let the model *see* the template source (helps a 30B, may confuse a 7B). Ship hidden; measure.

---

# Part II — Context as retrieval

**Status of this part:** Proposed, ADR-0036. Part I (§1–§12) is the arithmetic and stays
true. This part changes what assembly *is*. Part I's fixed layout and section shares become
`ContextPolicy v1`, the default and the baseline; nothing here ships until it beats that
baseline on the eval suite (§17).

## 13. Why Part I runs out

Part I made the budget honest. The design underneath it is still a chronological block list
with hard-coded integer priorities, compacted by dedupe → drop → cut-the-middle. Three
properties cap the agent no matter how correct the arithmetic gets.

**It is chronological, not relevant.** A block's position is decided by when it arrived and
by a constant in the source (`255` charter, `250` findings, `215` notices, `100` raw
results). The same ordering serves a one-step lookup and a forty-step sweep. Nothing asks
whether a block relates to what the model is about to do — so on a long run the window fills
with the most *recent* observations rather than the *relevant* ones, and the finding from
step 3 that decides step 30 is gone.

**It degrades lexically.** `truncate_middle` cuts a rendered string at a byte offset. A
500-row table becomes half a table: broken JSON, no row count, no schema, nothing. The useful
degradation — five rows, the column names and "495 more" — is strictly smaller *and* strictly
more informative. Every byte-truncated result is a result the model must either re-fetch or
reason over blindly.

**It is unmeasured.** The priorities, the shares and the trigger are asserted. No signal says
which blocks the model used, so the numbers cannot improve, and in practice they have not
since they were written.

A 7B's whole problem is that its window cannot hold the run. A relevance-selected 3,500-token
body is a categorically different object from the most recent 3,500 tokens, and that
difference is what makes 7B-to-frontier a range rather than a slope.

## 14. The run context store

[DO] Every observation becomes an addressable record in a per-run store
(`crates/harness/src/context/store.rs`), never a line in a buffer:

```rust
pub struct Record {
    pub id: RecordId,                 // stable; what a handle and a citation point at
    pub step: u32,
    pub kind: RecordKind,             // ToolResult | Finding | Correction | PlanChange | Memory | Decision | Note
    pub source: String,               // tool name, or "memory", or "harness"
    pub provenance: Provenance,       // §Part I; untrusted stays fenced through every path here
    pub subject: Subject,             // instrument, strategy, experiment, family — extracted, not parsed from prose
    pub ids: BTreeMap<String,String>, // the evidence ids this record carries
    pub body: Body,                   // TYPED, not a string (§16)
    pub tokens_full: u32,
    pub utility: Utility,             // §19: filled in after the fact
}
```

Records are **never deleted during a run**. They leave the *prompt*, never the store, which is
what makes every later mechanism here recoverable rather than lossy. The store is the same
object the effect log persists (AGENT-010 §5), so a replay reconstructs context exactly.

## 15. Assembly is a query

[DO] Before each call, `select(store, working_state, budget, policy) -> Vec<Rendered>`.

The **query is the working state** (AGENT-008 §2): the goal and its kind, the active plan
step and the tool it names, the hypothesis, the last error, the ids already cited. The score
for each record is a weighted sum of signals that are cheap and structural, not a model call:

| Signal | Why |
|---|---|
| `subject` match with the active step | a step about BTC-USD 1h does not need the ETH profile |
| id overlap with the step's arguments or the hypothesis | the record that produced the id being used is the record most likely needed |
| `kind` prior for this `GoalKind` | a lookup weights `ToolResult`; a research step weights `Finding` and `Correction` |
| recency, with a shallow decay | recent still matters; it stops being the *only* thing that matters |
| citation count within the run | a record the model has already used twice is load-bearing |
| the step's own tool: records this tool produced before | the second `get_bars` call wants the first one's shape |
| `Correction` for the active tool | the model must not repeat a rejection it was just given |
| pinned | the charter, the goal, the manifest and the last error are always selected |

Selection fills the budget in score order, then **re-sorts the winners into the stable layout**
so the prefix stays cache-friendly: pinned first, then by section, then by step ascending. The
model reads a stable, chronological-looking prompt; the harness chose it by relevance. Both
properties matter and they are not in tension.

Under the compaction trigger with everything fitting, selection is a no-op and every record is
rendered — so a short run behaves exactly as it does today, and the machinery only engages
under the pressure it exists for.

## 16. Records degrade semantically

[DO] `Body` is typed, and every variant declares how to shrink. Degradation walks the ladder
until the record fits; **byte truncation is not on the ladder and `truncate_middle` is deleted
from the record path.**

| `Body` | Ladder, cheapest loss first |
|---|---|
| `Table { schema, rows }` | drop low-salience columns → sample rows (head + tail + extremes) → aggregate to summary statistics → schema + row count alone |
| `Series { points }` | decimate by stride → downsample to a fixed budget of points → min/max/mean/last + n |
| `Stat { fields }` | drop `detail` → keep `summary` + ids + n/se → keep the number and its id alone |
| `Text { body }` | keep first and last paragraph with an elision marker → first sentence → title alone |
| `Json { value }` | prune by declared key salience → keep required keys → keep the ids alone |

Every degraded render carries `degraded: { from_tokens, to_tokens, how }` so the model is told
what it is looking at, and the handle to the full record. **A record that reaches the bottom
of its ladder and still does not fit is dropped, not cut** — and it stays in the manifest.

This is where `ToolResult` (AGENT-006 §2.4) pays off: `summary`/`ids`/`n`/`detail`/`handle` is
already the `Stat` ladder, so most of the catalogue gets semantic degradation for free.

## 17. The manifest: removing content without removing knowledge

[DO] A small, **always-rendered, never-degraded** index of the entire store — one line per
record: id, step, kind, source, subject, size, and whether it is currently in the prompt.

```
CONTEXT (41 records, 12 shown)
  r07 s3  ToolResult  get_bars        BTC-USD 1h 2024-01→06   4.1k  [elided]
  r11 s5  Finding     record_finding  BTC-USD ema-cross        180  [shown]
  r19 s8  Correction  validate_strategy  when-names-expression 90   [shown]
  …
```

Roughly 12 tokens a record; a hundred-record run costs ~1.2k tokens and never grows
unbounded because the manifest itself degrades by grouping (`s3–s7 · 5 × ToolResult get_bars ·
19k · elided`).

This is the difference between an agent that *forgot* and one that *knows where that is*. The
failure mode of any retrieval system is the miss; the manifest converts a silent miss into a
visible, recoverable `expand` (§18). It is also why §15 may be aggressive: the cost of leaving
something out is now one line and one optional call, not a lost observation.

## 18. The model can pull

[DO] Two core tools, both harness-owned, neither spending a trial:

- `expand(record_id, level = full | more)` — brings a degraded or elided record back at higher
  fidelity for this step and the next two, then it returns to normal scoring. Re-expansion is
  charged to the budget like anything else; if it does not fit, the harness says so and names
  what it would have to drop.
- `focus(subject | topic)` — biases selection toward a subject for the remaining steps of the
  plan step. The model saying "I am working on ETH funding now" is a better relevance signal
  than anything the harness can infer, and it costs one short call.

An automatic rule that needs no tool: a record whose handle is `expand`ed twice is **promoted**
— its score gets a persistent boost for the rest of the run, because the model has told us
twice that our scoring was wrong. Context breathes with the task instead of only shrinking.

## 19. Utility: measure what was actually used

[DO] The loop already knows what the model did next. After each call, attribute every rendered
record against the reply:

- an id from the record appears in the next tool call's arguments, in a `record_finding`, or in
  `finish_task` evidence → **used**;
- the record's tool is called again with arguments derived from it → **used**;
- `expand` on it → **wanted, and we under-rendered it**;
- rendered and never referenced before it left the prompt → **unused**.

Per run this is noise. Aggregated per `(GoalKind, RecordKind, signal bucket)` across the
scorecard corpus it is the first real evidence anyone on this platform has had about what
belongs in a prompt — and the selection weights in §15 are fitted to it rather than declared.
`utility_rate` (used ÷ rendered tokens) joins the scorecard per tier, beside cache hit.

The honest framing: today's priority constants are an untested hypothesis about what a model
needs. This makes them a measured one. That is the same move the platform makes everywhere
else, applied at last to the prompt.

## 20. Budget follows value

[DO] Not every step deserves the same window. The step that is about to spend a trial, advance
a gate or write a strategy is worth more context than the step that is listing instruments —
and today they get identically sized bodies.

`step_value` is derived, not asked for: from the active plan step's tool (`spends_trial`, its
envelope, its `max_calls_per_run`) and the working state (a first attempt after a correction is
worth more than a repeat). The body budget is scaled within bounds — `0.6×` for a lookup step
up to `1.4×` for a trial-spending step, clamped by the profile's hard input budget and the 25%
floor. Total spend per run is unchanged; its distribution is not.

## 21. The prefix is composed, and it is continuous

**Composed charter.** [DO] The charter stops being one string. It is fragments —
identity, termination, tools, plus a per-`GoalKind` fragment — assembled at run start. A
lookup goal never pays for the edge-finding protocol, which is roughly 900 of its 3,200 bytes.
The composition is deterministic and the result is byte-stable **per kind**, so caching is
preserved: there are four prefixes instead of one, each stable.

**Cross-turn continuity.** [DO] Today every turn re-assembles from scratch, so the provider
cache is cold at the start of every turn even though the charter and most of the tool slice are
identical. Turn N+1's prefix is built as a **superset of turn N's**: same charter fragment, the
same tool slice with additions appended (never reordered — Part I §5.2), and the previous
turn's summary appended rather than inserted. A conversation of ten turns pays one cold prefix
instead of ten.

## 22. The policy is an object, and it is evaluated

[DO] Everything in Part II is parameters, and parameters that are not measured become folklore.
They become one versioned object recorded on every run:

```rust
pub struct ContextPolicy {
    pub version: u32,
    pub weights: SelectionWeights,        // §15
    pub kind_priors: BTreeMap<(GoalKind, RecordKind), f32>,
    pub degradation: DegradationOrder,    // §16
    pub manifest_budget: f32,             // §17
    pub step_value_bounds: (f32, f32),    // §20
    pub trigger: f32,                     // Part I
}
```

- `v1` is Part I exactly — fixed layout, fixed shares, recency ordering, byte truncation. It
  is the control.
- A new policy ships only by beating `v1` on the AGENT-009 suites for **every** tier, under the
  interval rules of AGENT-009 §4, with `utility_rate` and cost reported beside pass rates. A
  policy that helps the frontier and hurts the 7B does not ship (AGENT-009 P4).
- The policy version is recorded on every run and in every trajectory, so a replay is exact and
  an eval can A/B two policies over the same tapes at no model cost. **This is the payoff of
  the sans-IO loop:** context policy becomes a thing that can be tuned offline against real
  runs.

## 23. Migration, and what must remain true

Part II lands behind `context.policy` with `v1` as the default, so there is no flag day.
Order: store and manifest (harmless, additive, immediately useful) → semantic degradation
(replaces the worst existing behaviour) → selection behind the policy flag → attribution →
fitted weights → composed charter and cross-turn continuity.

Invariants Part II does not get to break:

1. Budget arithmetic is Part I's, unchanged. Selection changes *what* is rendered, never how
   much is allowed.
2. Loss is recoverable: everything dropped has a manifest line and a handle (P6).
3. Untrusted provenance survives every path — store, degradation, manifest, `expand`.
4. Determinism: same store, same working state, same policy → same prompt. Selection uses no
   model call and no wall-clock.
5. The charter, the goal, the manifest and the last error are always rendered.

## 24. Selection quality: the eight ways a scorer goes wrong

§15 gives a weighted sum. A weighted sum is where retrieval systems fail, and each failure
below is one an accumulation buffer could not have.

**24.1 Diversity, or selection collapses onto one topic.** Top-k by score picks ten records
about the same subject, because whatever made one score highly makes its neighbours score
highly too. That is strictly *worse* than recency, which at least samples across time.
Selection is therefore **maximal-marginal-relevance**: each pick is scored against the query
*minus* a redundancy penalty against what is already selected, penalty computed on
`(subject, kind, source)` overlap and on shared ids. Plus hard floors — at least one
`Correction` if any exists, at least one `Finding` if any exists, at least one record from the
last two steps — so no weighting can starve a whole kind.

**24.2 Hard requirements, derived not scored.** The next plan step names a tool, and that
tool's schema names required arguments. If the step is `run_study{experiment_id}`, the record
that produced that `experiment_id` is not a *candidate* — it is a **requirement**. Selection
derives a must-include set from the step's tool schema and the ids in its partially-filled
arguments, and satisfies it before scoring anything. A scorer that can lose the id the next
call needs is a scorer that will.

**24.3 Signals are normalised or one of them wins.** Recency decay is `[0,1]`, citation count
is unbounded, id overlap is a small integer, subject match is a boolean. Summed raw, the
unbounded one dominates and the weights are decoration. Every signal is normalised to `[0,1]`
against the current store (counts by percentile rank, not by value), and the policy records
the normalisation so a fitted weight means the same thing in two runs.

**24.4 Stability, or relevance eats the cache.** This is the sharpest tension in Part II. If
step N and step N+1 select different sets, the message-layer prefix changes and the provider
cache misses — so a retrieval design that ignores it can cost more than the accumulation
design it replaced, while looking better on every relevance metric. A record rendered on the
previous call carries a **stickiness bonus**, and a record is dropped only when it falls below
a lower threshold than the one that admitted it. Two thresholds, deliberately: hysteresis is
what stops a record oscillating in and out and rewriting the prefix every step.

**24.5 Deterministic tie-break.** Ties break on `(step ascending, record id)`. Never on hash
order, never on iteration order of a map. Determinism is an acceptance criterion (§30.4) and
this is the line where it is usually lost.

**24.6 Cold start is recency, and says so.** At steps 0–1 the working state has a goal and
nothing else: no active tool, no ids, no citations. Selection degenerates to recency plus
pinned, which is correct and is exactly Part I. It is stated here so nobody reads a degenerate
early prompt as a bug.

**24.7 Pin corrections for what is *about* to be called.** §15 pins corrections for the active
tool. The plan names the next two steps, and a correction about a tool the model is about to
reach is worth more than one about the tool it just used. Pin corrections for the active tool
**and** the next planned tool. The cost of dropping one is the model repeating a rejection it
has already been given, which is the most expensive kind of wasted step.

**24.8 Disagreement is not redundancy.** Two records that answer the same question differently
— two data sources, a re-read after a revision, a statistic computed two ways — look like
near-duplicates to the redundancy penalty in §24.1 and exactly one survives, which silently
picks a winner. When two records share a subject and a measured quantity but their values
disagree beyond their stated error, **both are selected and rendered adjacent with the
disagreement named**. Same principle as AGENT-007 P5, applied inside a run, and for the same
reason: the disagreement is the finding.

## 25. Rendering economy: seven cheap wins before any selection happens

Selection decides *what*; these decide *how much it costs to say it*. They compound, and none
of them loses information.

**25.1 Tables are not JSON.** A 200-row result rendered as JSON spends roughly 40% of its
tokens on repeated key names, braces and quotes. The same rows as a header line plus
tab-separated values carry identical information. Tabular bodies render in a compact delimited
form with the schema stated once; `Json` stays JSON because its shape is the content.

**25.2 Shared schemas are stated once.** Ten `get_bars` records have one schema between them.
Rendering hoists a schema shared by two or more selected records into a single preamble and
the records reference it. Structural, lossless, and it scales with exactly the repetition that
makes long runs expensive.

**25.3 Precision follows the error bar.** A mean rendered as `0.0234871629` when its standard
error is `0.004` spends eight tokens asserting digits that are noise, and invites a model to
quote them. Numerics render to significant figures derived from `se` where one exists, and to
a policy default otherwise. This is a token win and an honesty win in the same change.

**25.4 The second observation is a delta.** A run that re-reads the same window, polls a job,
or re-lists instruments produces records that are mostly identical to their predecessors. The
second renders as a **diff against the first** — "as r07, except `close` 42,118 → 42,340; 3
rows changed" — with the full record one `expand` away. On polling-shaped and re-read-shaped
runs this is the single largest saving available, and it is the shape the loop's own
idempotent-repeat guard already detects.

**25.5 Repeated corrections fold.** Three validation failures on the same path are one
correction with a count and the latest instance, not three near-identical blocks at priority
215. Folding also makes the repetition legible: "3rd rejection on `exits[0].distance`" is the
signal that the model needs a worked example (AGENT-011 §5), and the fold is where the harness
notices.

**25.6 Degrade-or-drop is a choice, not a sequence.** §16 walks each record's ladder in turn,
greedily. Greedy is wrong: degrading one large high-value record may free less than dropping
two small low-value ones, and it loses more. Fitting is a small optimisation over
`(value, tokens)` at each ladder step — take the cut with the best value-loss per token
recovered, repeat until it fits. Bounded work over tens of records, and it is the difference
between "the budget fit" and "the budget fit *well*".

**25.7 The harness's own furniture has a budget.** Degraded markers, handles, provenance
marks, the "why you are seeing this" lines (§28.3) and the manifest are all tokens, and they
grow with record count rather than with content. They are counted as one `overhead` line item
capped at 12% of the body, and the cap is enforced by shortening the furniture — never by
dropping content silently to make room for labels about content.

## 26. Store discipline

**26.1 Supersession inside a run.** A second `get_bars` over the same instrument, timeframe
and window supersedes the first; a re-read after a revision supersedes what it revises. The
store links them, selection considers only the head of a supersession chain, and the manifest
shows the chain collapsed with its depth. Without this, §24.1's redundancy penalty spends
effort suppressing what the store should never have offered as two candidates.

**26.2 Relevance decays by kind, not uniformly.** A `Correction` from step 2 is usually dead by
step 30; a `Finding` from step 2 may be the thing that decides step 30. One decay constant for
both is wrong in one direction or the other. Decay half-lives are per `RecordKind` and live in
the policy, so they are fitted with everything else (§22).

**26.3 The store spills.** A 400-step run holds thousands of records and the store is in
memory. Beyond `max_resident_records` the coldest bodies spill to the workspace, leaving the
metadata — id, kind, subject, ids, tokens — resident so scoring and the manifest still work on
the whole store. Spilling is invisible except in latency; `expand` reads it back.

**26.4 Scoring is incremental.** Re-scoring every record on every call is quadratic over a run.
Signals that cannot change between calls (subject, source, kind, step) are computed once at
insert; only the query-dependent part is recomputed. A 400-step run must not spend more wall
clock assembling prompts than waiting for models.

## 27. Budget refinements

**27.1 The output reserve follows the step too.** `reserve_for_output` is one constant per
profile, but the step that writes a strategy or a final report needs far more room than the
step that calls `list_instruments` — and reserving the maximum for every call is why the body
is smaller than it needs to be on most of them. Reserve scales with the step's expected output
shape, taken from the active tool's schema, within the profile's bounds.

**27.2 Untrusted content cannot take the window.** Fetched pages are records like any other,
and a page that matches the subject scores well — so a run that reads three long articles can
fill its body with untrusted text, which is both an economic problem and the precondition for
the injection the provenance fence exists to contain. Untrusted records are capped at a share
of the body (default 25%), enforced in selection, and the cap is reported when it binds.

**27.3 The manifest has a hard ceiling.** §17 degrades by grouping, which bounds it in
practice; the ceiling makes it bounded in principle. At the cap the manifest groups more
aggressively rather than truncating, because a manifest that is cut is a manifest that lies
about what exists.

## 28. Measuring what would have helped

**28.1 Counterfactual ablation.** §19 measures what the model *used*, which is not what it
*needed* — a record can be used because it was there and a better one was not. Over recorded
tapes, re-run a step with one selected record removed and compare the resulting call. A record
whose removal changes nothing was not load-bearing, whatever the attribution said. This is
cheap exactly here and nowhere else: the loop is sans-IO, the tape supplies the model, and an
ablation sweep over a corpus costs no provider call. It is the same move as the platform's
`ablation_sweep` for strategies, pointed at its own prompt.

**28.2 "Unused" and "checked and dismissed" are different.** A record the model read and
correctly ignored is a *success* of selection; one it never needed is a failure; both look
identical as "rendered, not cited". They are separated where evidence allows: an explicit
dismissal in the model's reasoning, a `focus` away from the subject, or an ablation (28.1)
showing the call changes when it is removed. Where they cannot be separated, the metric says
`indeterminate` rather than assuming the flattering reading.

**28.3 Every record says why it is here.** One short clause per rendered record — "matches the
active step's instrument", "produced experiment_id you are using", "3rd rejection on this
path", "disagrees with r07". It costs a few tokens and buys three things: the model can tell
what the harness thinks it is doing, a human debugging a bad prompt can see the scorer's
reasoning without reading code, and the reason is a label the eval suite can group by.

**28.4 The prompt is inspectable.** Every assembly emits a `SelectionReport` — candidates
considered, scores by signal, what was selected, what was degraded and by which ladder step,
what was dropped, which constraints bound. It goes in the trajectory, which makes the desk's
replay scrubber (AGENT-012 §4) able to answer "why did the model not know that?" — the single
most common question about an agent that did something strange, and one no version of this
system could answer before.

## 29. Context for sub-runs

AGENT-010 §9 gives a critic "the evidence bundle only" and a worker "a byte-identical brief".
Neither says how those are assembled, and assembling them by hand would be a second context
pipeline with none of this one's guarantees.

A sub-run's context is a **projection of the parent store**: a filter over records plus a
budget, run through the same selection, degradation and manifest. The critic's projection
excludes the parent's reasoning and corrections by construction — that is what "fresh context"
means mechanically, rather than as an instruction — and carries findings, the evidence they
cite and the dossier. The worker's projection is pinned so every worker's brief is
byte-identical, which is what makes the fan-out cache-friendly. Projections are declared in
the policy and recorded on the sub-run, so a critic that saw too much is visible after the
fact rather than arguable.

## 30. Acceptance (Part II)

1. A record is never byte-truncated; every shrink is a declared ladder step carrying
   `degraded {from, to, how}` and a handle.
2. The manifest lists every record in the store, is never degraded below one line per group,
   and its elided entries are `expand`able.
3. On a forty-step fixture where the deciding observation is at step 3, policy `v2` renders it
   at step 30 and `v1` does not. This is the case the whole part exists for.
4. Selection is deterministic: same inputs, same prompt, asserted by replaying a trajectory
   twice under the same policy version.
5. `utility_rate` is computed per run and aggregated per `(GoalKind, RecordKind)`; the shipped
   weights are fitted to it, and the fit is reproducible from the corpus.
6. A lookup goal's charter omits the edge-finding fragment; each per-kind charter is
   byte-stable and under the ceiling.
7. Across a ten-turn conversation, cold prefix writes happen once, not ten times.
8. `v2` beats `v1` on every tier before promotion, with no tier's suite regressing under
   AGENT-009 §4's intervals.
9. A retrieval miss is recoverable in one step: the fixture where selection omits a needed
   record shows the model finding it in the manifest and calling `expand`.
10. With `context.policy = v1` the assembled prompt is byte-identical to Part I's.
11. Selection is diverse: on a fixture where ten records share a subject, the selected set
    contains at least one `Correction`, one `Finding` and one recent record, and never ten of
    one subject.
12. A required id implied by the next step's tool schema is always present, asserted on a
    fixture where its record scores below the cut.
13. Hysteresis holds: over ten steps with a slowly-changing query, the selected set changes by
    at most the churn bound, and cache-creation tokens do not rise against `v1`.
14. No rendered table is JSON; a shared schema across two selected records is stated once; a
    mean with `se = 0.004` is not rendered to ten decimal places.
15. A re-read of an identical window renders as a delta naming its base record, and the full
    record is one `expand` away.
16. Untrusted records never exceed their share of the body, asserted with a fixture of three
    long fetched pages; the cap is reported when it binds.
17. Overhead (markers, handles, provenance, reasons, manifest) stays within its cap, and the
    cap is met by shortening furniture rather than dropping content.
18. Every rendered record carries a one-clause reason, and every assembly emits a
    `SelectionReport` into the trajectory.
19. Ablation over a tape corpus runs with no provider call and produces a load-bearing ranking
    that differs from the raw citation ranking.
20. A critic sub-run's projection provably excludes the parent's reasoning and corrections.
