# Phase 1 — Contracts

**Completion: 25% (5 / 20 tasks), plus 2 partial. Track A landed, plus S-1.10; the rest of tracks B and C not started.**

> Status 2026-09-15, track 1. The three contracts the other Set S phases wait on now exist:
> `ToolSpec`/`ToolResult` (`crates/mcp-server/src/spec.rs`), the generated taxonomy and
> profile lists, and `WorkingState` (`crates/harness/src/state.rs`). Track A (S-1.1 … S-1.4)
> is done. Track B (the context contract, S-1.5 … S-1.12) is **not started**. Track C has
> S-1.13's type but none of its wiring, and S-1.14 … S-1.20 are not started.
>
> S-1.10 is done as well, out of track order: phase 1 says track C rebases onto it and track B
> has not started, so leaving the one shared file in tracks B and C unwritten would have blocked
> C for no reason.
>
> Three things a following track must know before touching this phase:
>
> 1. **The catalogue here is 60 tools, not 73.** The thirteen `falsification` and
>    `web`/`skills` tools are untracked and cannot be committed without adding
>    `crates/webfetch` and `crates/skills` to `Cargo.toml` — a do-not-commit file
>    (EXECUTION-BOARD §2.4, gate H-1). Each needs a `ToolSpec` when it lands; the shape
>    does not change. Checked directly: neither file contains `f64` or `f32`, so the
>    money-f64 exemption they have been living under has cost nothing.
> 2. **`spends_trial` and `max_calls_per_run` are declared, not enforced** (ADR-S1-03).
>    Declaring them was necessary so six phases do not each have to revisit their
>    declarations later; the dispatch wrapper that writes the ledger row before the tool
>    runs is AGENT-006 §2.5 and belongs to a later phase.
> 3. **Do not share `CARGO_TARGET_DIR` across worktrees.** EXECUTION-BOARD §2.3 says to; it is
>    wrong, and two tracks found it independently. Artifacts collide between worktrees, so a
>    suite can run against another worktree's code — observed here as a `static_invariants`
>    binary with 23 tests built from a source with 27, and a link against an `mcp-server-lib`
>    with no `spec` module. Everything reported green in this phase was re-run from cold in a
>    private target directory. The `jobs = 2` pin is separately real and still must not be
>    raised. Details in phase 0's S-0.1 triage.

**Requirement IDs:** TC-01…TC-10 (track A), CA-01…CA-09 (track B), RP-01…RP-07 (track C).
**Specs:** [AGENT-006 §2.3–2.4, §3](../../../specs/AGENT-006-tool-catalog.md),
[AGENT-005 §3, §4, §6.1](../../../specs/AGENT-005-context-assembly.md),
[AGENT-008 §2, §3.1–3.2](../../../specs/AGENT-008-reasoning-protocol.md).
**Depends on:** Phase 0.
**Blocks:** Phases 2–8. Nothing else may start until the three contracts exist, because
every later task declares a tool, spends a budget, or reads a working state.
**Migration range:** none.

Three tracks, disjoint in files, safe to run in three worktrees. Track A owns
`crates/mcp-server`, track B owns `crates/llm` + `crates/api/src/agent/prompt.rs`, track C
owns `crates/harness/src/drive.rs`. Both B and C touch `crates/harness/src/context.rs` —
B lands `assemble_reserving` first (S-1.10), C rebases onto it.

---

## Track A — the tool contract

### S-1.1 `ToolSpec`: one declaration per tool — M
`DONE` 2026-09-15 — `crates/mcp-server/src/spec.rs`; all 60 tracked tools declare one in
`tools/<module>::specs()`.

Verified by `mcp-server-lib::spec::tests::every_catalogue_tool_has_exactly_one_spec` and,
adversarially, `a_tool_declared_twice_is_rejected_naming_both_sites` (feeds two declarations of
`get_bars` from different namespaces and asserts the panic names both catalogue indices).
`version` and `status` are present from the outset; `a_deprecated_tool_still_resolves_but_is_exposed_to_nobody`
proves the tombstone — `spec::find` answers for it so an old ticket resolves, `spec::exposed`
does not.

**Partial, and named as such:** a ticket does not yet record the tool version. That is a
`crates/ledger` change to the `TrialTicket` path, which this track did not touch because the
envelope work already put it in `ledger::audit` and two tracks editing one crate is what
EXECUTION-BOARD §2.2 asks us not to do. The field exists and is stable; stamping it is a
one-line read of `spec::find(tool).version`.

The type in AGENT-006 §2.3, in a new `crates/mcp-server/src/spec.rs`. Then migrate the
catalogue file by file to `tools/<ns>.rs::specs() -> Vec<ToolSpec>`, keeping the existing
hand-written arrays alive beside it until S-1.3 deletes them.

**Files:** `crates/mcp-server/src/spec.rs`, `crates/mcp-server/src/tools/*.rs`.
Include `version` and `status` from the outset (§2.7): a trial ticket cites a tool, and a
tool whose behaviour changes without a version bump silently makes every earlier ticket
incomparable. Stamp `version` into the ticket beside the existing `tool_schema_hash`, and
make `Deprecated` a tombstone that leaves the catalogue resolvable for old evidence while
being excluded from every profile's exposure.

**Files:** `crates/mcp-server/src/spec.rs`, `crates/mcp-server/src/tools/*.rs`,
`crates/ledger` (ticket stamping).
**Acceptance:** every catalogue tool has exactly one `ToolSpec`; a tool declared twice is
rejected by a test naming both sites; a ticket records the tool version that produced it; a
deprecated tool still resolves for an old ticket and appears in no profile's exposure.
**Traces:** AGENT-006 §2.3, §2.7; §10.1.

### S-1.2 `ToolResult`: answers, not data — M
`PARTIAL` 2026-09-15 — the type, the renderer and the gates exist; no typed tool returns one
yet.

`ToolResult {summary, ids, n, se, detail, handle}` with `render_within(max_bytes, handle_hint)`.
Two decisions worth carrying forward:

- The cap is the **effective** one, `min(profile cap, remaining body)` at the moment the result
  arrives — S-1.18's first defect, fixed here rather than later, because the signature would
  otherwise have had to change. Verified by
  `a_result_over_the_smallest_cap_always_carries_a_handle` (6,144 B, the smallest profile):
  `detail` is dropped whole and replaced by a handle, never truncated in place.
- A cap too small even for the summary still returns the summary, truncated on a char
  boundary. `a_cap_smaller_than_the_summary_truncates_rather_than_returning_nothing`. A tool
  that answers with nothing is indistinguishable from one that failed, and the model retries it.

Summary templates: `render_summary` over declared `detail_fields`, with
`a_template_naming_an_undeclared_field_is_rejected` (adversarial) and the build-time gate
`a_summary_template_only_names_declared_fields`. `a_numeric_field_without_a_value_renders_n_unknown`
covers the "(n unknown)" rule.

**Missing:** migrating the 60 typed handlers to return `ToolResult`. They still return ad-hoc
`Value`. The ≤ 400-token summary bound therefore has nothing to measure on the reference
dataset yet.

`{summary, ids, n, se, detail, handle}` with `render(profile)` applying the profile's
`max_tool_result_bytes` and setting `handle` on overflow. Migrate typed tools to return it.

The `summary` is **rendered from a declared template** over the result's own fields, never
hand-written: sixty hand-written prose blobs would drift from the numbers they describe, and
a summary that disagrees with its `detail` is worse than none — it is the part a flat-tier
model reads first and a hurried human reads only.

**Files:** `crates/mcp-server/src/spec.rs`, `tools/*.rs`.
**Acceptance:** `summary` ≤ 400 tokens on the reference dataset; a template referencing a
field the schema does not define **fails the build**; a result over the smallest profile's
6,144-byte cap always carries a `handle`; a numeric field without `n` renders "(n unknown)".
**Traces:** AGENT-006 §2.4, P3; §10.9.

### S-1.3 Generate the four arrays from the specs — M
`DONE` 2026-09-15 — `tool_definitions_for`, the taxonomy and the internal-agent profile all
render from `spec::catalog()`; `stamp()` and both hand-written arrays are deleted.

Verified by `cargo test -p mcp-server-lib --lib taxonomy` (the CI job): `every_tool_is_classified`,
`the_taxonomy_has_no_entries_for_tools_that_do_not_exist` and `no_tool_is_classified_twice` now
run against generated rows. `every_namespace_is_canonical` is new and asserts §2.1's
twenty-two — a tool declaring `strategies` instead of `strategy` would otherwise create a
namespace of one that no plan step routes to.

The envelope lists are **reconciled, not generated**: `ledger` cannot depend on `mcp-server`
without inverting the dependency. `invariants::every_tool_envelope_matches_the_ledger` asserts
agreement in both directions and pins the gated catalogue set to exactly
`{arm_automation, promote_model_version}`, so widening it is a decision rather than a diff.
S-0.2's envelope test still passes.

`TAXONOMY`, `INTERNAL_AGENT_TOOLS`, the four envelope lists and the guide's tool-notes
index all derive from `ToolSpec`. Delete the hand-written arrays. Keep the three taxonomy
tests running against the generated output throughout.

**Files:** `crates/mcp-server/src/{taxonomy,lib}.rs`, `crates/ledger/src/audit.rs`.
**Acceptance:** `cargo test -p mcp-server-lib --lib taxonomy` green (it is a CI job);
`every_tool_is_classified`, `the_taxonomy_has_no_entries_for_tools_that_do_not_exist` and
`no_tool_is_classified_twice` pass against generated arrays; S-0.2's envelope test still
passes with the lists generated rather than typed.
**Traces:** AGENT-006 §2.3 P2, §2.6; §10.1, §10.2.

### S-1.4 The mechanical gates: description budget, namespace cap, dispatch coverage — M
`DONE` 2026-09-15 — five gates, all in `cargo test -p mcp-server-lib --lib`.

`no_tool_descriptor_exceeds_the_rich_budget` (≤ 1,200 B),
`every_first_sentence_fits_the_flat_budget` (≤ 400 B),
`no_namespace_holds_more_than_sixteen_tools`,
`every_dispatch_arm_has_a_spec_and_every_spec_has_an_arm`,
`every_notes_section_resolves_to_a_heading`.

Meeting the budget moved ~7.4 KB of prose out of thirteen descriptions into a new
`## Tool notes` section of `crates/mcp-server/docs/authoring_guide.md`, verbatim. The
over-budget set on the tracked catalogue was `create_experiment` (1,405 B after the first
pass), `train_model` (1,376) and `run_sweep` (1,290) — and most of those bytes were *parameter*
descriptions, not the tool description. AGENT-005 §5.4 measures the whole rendered descriptor,
so trimming only the description would not have met it; that is worth knowing before the
thirteen blocked tools are re-measured.

**Partial:** `get_authoring_guide(section, tool?)` is **not** implemented — the guide is still
served whole, so a 7B fetching one ≤ 4 KB section cannot yet. The sections and headings the
parameter would select over now exist, which is the part that had to come first. The flat bound
is asserted on the first sentence rather than on `registry::render(_, Flat)`.

Three tests that cannot be argued past, plus the rewrite they force: the ten over-budget
descriptions (measured: `create_skill` 2,123 B … `fetch_url` 1,212 B, and `run_skill`'s
5.2 KB of source) keep their first sentence and move the rest to `## Tool notes` in the
guide. Add `get_authoring_guide(section, tool?)` so a 7B fetches one ≤ 4 KB section.

**Files:** `crates/mcp-server/src/tools/*.rs`, `crates/mcp-server/docs/authoring_guide.md`,
`crates/mcp-server/src/lib.rs`.
**Acceptance:** every description ≤ 1,200 B rich and ≤ 400 B flat; no namespace > 16 tools
(`crates/api/src/agent/bridge.rs:355` still green); every dispatch arm has a spec and every
spec has an arm; every `notes_section` resolves to a heading.
**Traces:** AGENT-006 §3.1, §3.2, §8; §10.3, §10.4. AGENT-005 §5.4.

---

## Track B — the context contract

### S-1.5 Split `system_prompt` into a lean charter — M
`NOT STARTED`

15.5 KB → ≤ 3.2 KB. Out: the five-step research protocol (moves to the guide), the inlined
10.7 KB authoring guide, the run constraints (move to the opening message), and **the
draft-builder instruction, which names tools the internal profile does not expose**
(`prompt.rs:31` vs `lib.rs:495`). In: identity, a ≤ 900 B edge-finding pointer, a ≤ 300 B
tools/`search_tools` paragraph, the termination contract verbatim.

**Files:** `crates/api/src/agent/prompt.rs`.
**Acceptance:** `charter().len() <= 3_200` and `charter() == charter()` (byte-stable); the
charter contains `get_authoring_guide`, `search_tools`, `finish_task`; it does **not**
contain `new_strategy_draft`; no test or prompt names a tool the internal profile lacks.
**Traces:** AGENT-005 §3.1; §10.1.

### S-1.6 The opening message carries everything that varies — S
`NOT STARTED`

`run_constraints()` + `opening_message(prior, constraints, goal)`. Both drivers seed their
first user message with it. This also fixes the hosted driver silently discarding
`DriverParams::prior` (`driver.rs:33`, computed at `manager.rs:253`, never read).

**Files:** `crates/api/src/agent/{prompt,driver,local_driver,manager}.rs`.
**Acceptance:** constraints render into the first user message, never the charter; a turn
with prior conclusions starts with them and ends with "The user now asks:"; a goal with
neither is passed through unchanged.
**Traces:** AGENT-005 §3.2; §10.1.

### S-1.7 Move the research protocol into the served guide — S
`NOT STARTED`

**Files:** `crates/mcp-server/docs/authoring_guide.md`, `crates/mcp-server/src/lib.rs:615`.
Coordinate with S-1.4 — same two files.
**Acceptance:** the guide opens with `## The research protocol`; `get_authoring_guide`'s
description names it; a lookup goal never fetches it.
**Traces:** AGENT-005 §3.3.

### S-1.8 Prompt caching: three breakpoints, in prefix order — S
`NOT STARTED`

`tools` (last entry) → `system` (as a text block) → the last content block of the last
message. Segments under the provider's minimum are ignored by the API, so this is safe on
small prompts.

**Files:** `crates/llm/src/anthropic.rs` (`build_body`).
**Acceptance:** `cache_breakpoints_close_each_prefix_layer` asserts only the last tool, the
system block and the last message carry the marker, and that an earlier user message stays a
plain string.
**Traces:** AGENT-005 §6.1; §10.2.

### S-1.9 `Usage` learns about the cache — S
`NOT STARTED`

`cache_read_input_tokens` + `cache_creation_input_tokens` with `#[serde(default)]`, a
`prompt_tokens()` total, Anthropic parsing both, OpenAI reading
`prompt_tokens_details.cached_tokens`, Ollama zero. Every `Usage { .. }` literal in tests
gains `..Usage::default()`.

**Files:** `crates/llm/src/{types,anthropic,openai,ollama}.rs`,
`crates/api/src/agent/local_driver.rs` (test literals).
**Acceptance:** `prompt_tokens()` equals the sum of all three; a response reporting 900
cached and 10 fresh reports 1,000 total.
**Traces:** AGENT-005 §6.1, §7; §10.2.

### S-1.10 `assemble_reserving` — the budget counts what leaves the body — M
`DONE` 2026-09-15 — `crates/harness/src/context.rs`. Landed by track 1 out of track order,
because phase 1 says track C rebases onto it and track B has not started.

Verified by `a_reservation_moves_the_compaction_point` — the spec's worked example run as
written: the same 6,500 tokens of blocks pass through untouched against a 10,000-token window
and compact once 4,000 are reserved. `the_body_keeps_its_floor_when_a_reservation_would_take_everything`
is the adversarial one: reserving 9,000, 10,000, 25,000 and `u32::MAX` all leave the body
exactly 2,500 tokens and a non-empty prompt. Without the floor, a profile whose schemas happen
to be large hands the body a budget near zero and the model reads a full description of its
tools and nothing about its task — which is the wrong thing to keep.
`assemble_is_assemble_reserving_with_nothing_reserved` pins the delegation, so no existing
caller changed behaviour.

`CompactionReport` gained **two** fields, not one: `reserved_tokens` and
`body_budget_tokens`. The second is what makes an over-reservation diagnosable — when it equals
`window × MIN_BODY_SHARE` the floor bound and the caller should expose fewer schemas. "The
prompt was compacted" and "the prompt was compacted because the schemas took two thirds of the
window" are different findings and only the second says what to change.

`assemble_reserving(profile, blocks, reserved_tokens)` with `MIN_BODY_SHARE = 0.25`;
`assemble` delegates with 0. `reserved_tokens` joins `CompactionReport`.

**Files:** `crates/harness/src/context.rs`.
**Acceptance:** with a 10,000-token budget and 4,000 reserved, blocks totalling 6,500 tokens
compact where at 9,500 they would not; the floor holds when a reservation would otherwise
take the whole window.
**Traces:** AGENT-005 §4.1; §10.4.

### S-1.11 Reserve the schemas; send the assembled charter — M
`NOT STARTED`

`working_prompt` reserves `exposed_schemas`. Separately, `ask()` currently sends the raw
`task.charter` (`drive.rs:1227`) while `working_prompt` discards the possibly-truncated
Charter section — close that by returning `(system, body)` and sending the assembled one.

**Files:** `crates/harness/src/drive.rs`.
**Acceptance:** on `qwen2.5-7b` with six flat schemas,
`est(system) + est(schemas) + est(body) ≤ 14,000` and `body_budget ≥ 3,500`, asserted
against the real `config/profiles/qwen2.5-7b-instruct.yaml`.
**Traces:** AGENT-005 §4.2, §4.3; §10.4.

### S-1.12 Findings stop growing quadratically; the dead section goes — S
`NOT STARTED`

`record_finding` rewrites one block instead of pushing a new one holding all findings
(`drive.rs:1751`). Delete `Section::Tools` (nothing ever rendered it) and renormalise the
shares to Charter .10 / Subtask .10 / FewShot .05 / WorkingState .55 / Reference .20.

**Files:** `crates/harness/src/{drive,context}.rs`.
**Acceptance:** after 20 `record_finding` calls exactly one block has source `findings` and
it lists 20 lines; `the_section_budget_shares_do_not_exceed_the_whole` still passes.
**Traces:** AGENT-005 §4.4, §4.5; §10.5.

---

## Track C — the reasoning contract

### S-1.13 `WorkingState`: the model reads state instead of rebuilding it — L
`PARTIAL` 2026-09-15 — the type is landed; `Loop` does not own one.

`crates/harness/src/state.rs` carries every field AGENT-008 §2 names, including AGENT-013's
lifecycle fields, with two renderers: a ≤ 400-token flat card bounded by construction and a
rich card adding plan, hypothesis and open questions. `anchor()` is derived from the state.

Verified by `the_card_stops_growing_once_it_is_populated` (adversarial: 200 updates move the
card by under 40 bytes — a fixed-size view over an unbounded history, which is exactly what the
nine append sites are not), `a_flat_card_is_bounded_however_long_the_goal_is`,
`an_id_no_tool_returned_cannot_be_cited`, `an_unset_state_authorises_nothing` (the defaults are
the narrowest reading: `Ring::Research`, `Stage::Explore`, `GoalKind::Lookup`, no trials), and
`re_asking_a_decided_question_is_corrected`.

**Missing, and it is the larger half:** `Loop` does not own a `WorkingState`, the nine
`Section::WorkingState` pushes in `drive.rs` are untouched, and nothing renders this type into a
prompt. Until that lands, AGENT-008 AC-1 is proved of the type and not of the system, which is
why its box is unticked.

Note for whoever wires it: `state::Decision` is deliberately not re-exported at the crate root
because `policy::Decision` already owns that name and means a policy ruling (ADR-S1-04).

The struct in AGENT-008 §2 in a new `crates/harness/src/state.rs`, owned by `Loop`,
replacing the nine ad-hoc `WorkingState` block pushes in `drive.rs` (`:926, :963, :1476,
:1544, :1624, :1751, :1818, :1986, :2033`). Two renderers: a ≤ 400-token card for flat
tiers, the full card for rich. One `Section::Subtask` block at priority 255, rewritten
every step.

**Files:** `crates/harness/src/state.rs`, `crates/harness/src/drive.rs`.
**Acceptance:** no working-state content is appended across steps; the rendered size is
bounded per tier; the anchor text derives from the state rather than being built separately.
**Traces:** AGENT-008 §2; §7.1.

### S-1.14 Plans are made of tools — closes the open planner bug — M
`NOT STARTED`

`PlanStep` gains `kind: Tool{name} | Workflow{name} | Reflect | AskUser`; the plan grammar
enumerates catalogue names rather than free text; a step that cannot bind to a catalogue
entry is rejected at parse with "each step names a tool; if none fits, use search_tools".
Delete the planner's `namespace` field — the catalogue search over the step goal already
outranks it (`drive.rs:1016-1041`).

**Files:** `crates/harness/src/drive.rs`.
**Acceptance:** a plan containing a non-tool step is rejected with the named correction; the
replay case built from `LOCAL_TIER_FINDINGS §9` ("the planner emitting non-tool steps is
the deeper cause and is still open") passes.
**Traces:** AGENT-008 §3.2; §7.2.

### S-1.15 Decisions are remembered — S
`NOT STARTED`

`WorkingState.decided` records each choice the protocol asked for with its reason and
renders it. Re-asking a decided question returns
`Correction("decided at step 4: …; say why it should change or move on")`.

**Files:** `crates/harness/src/{state,drive}.rs`.
**Acceptance:** re-asking a decided question is corrected with the step number; the local
traces' dominant waste (re-listing instruments, re-choosing timeframes) is reproduced as a
test and stops.
**Traces:** AGENT-008 §3.6; §7.5.

### S-1.16 Classify the goal before planning — M
`NOT STARTED`

Deterministic rules over goal + constraints → `GoalKind{Lookup|Research|Diagnose|Compare}`
and acceptance criteria. `Lookup` gets ≤ 3 read-only tools and no trials. On profiles with
`reflection: true` a single grammar-constrained model call may refine it. This turns the
charter's plea ("many goals are lookups") into a mechanism.

**Files:** `crates/harness/src/{state,drive}.rs`.
**Acceptance:** on `qwen2.5-7b` a lookup goal completes with ≤ 3 tool calls and 0 trials; a
research goal on the same profile reaches a plan whose every step is a catalogue name.
**Traces:** AGENT-008 §3.1; §7.3.

### S-1.17 The decode ladder: truncation, exemplars, and `reflect` — M
`NOT STARTED`

The forward-coverage gap in the first draft of this plan: AGENT-008 §7.7 requires that the
`LOCAL_TIER_FINDINGS §8` truncation fence cannot recur, and no task implemented it — S-4.9
only builds the replay case that would catch it. Also here because they are the same file and
the same decode path: wire `few_shot_examples` (declared in both local profiles, read today
only by `appendix_a.rs`) as two canned select→fill exemplars chosen by the step's namespace,
and add `reflect`.

A truncated `select_fill` reply is re-asked once with `max_tokens` doubled and "answer with
the JSON only"; a truncated native reply gets the existing nudge. **Never fence on
truncation** — that is the bug that made a healthy 7B session unreachable. `reflect` costs a
model call but is **not** counted against `max_steps` (capped at `max_reflections_per_run`,
default 3): a 15-step budget that charges 7% of itself for noticing a wrong turn discourages
noticing.

**Files:** `crates/harness/src/drive.rs`, `crates/harness/src/fewshot/`,
`config/profiles/*.yaml`.
**Acceptance:** a truncated select reply produces a re-ask, not a `Degradation`; `reflect`
does not decrement the step budget and is capped; few-shot blocks appear only on
`select_fill` profiles and are measured in the scorecard before being kept.
**Traces:** AGENT-008 §3.3, §3.5; §7.7.

### S-1.18 Budget correctness: the cap, and the honest estimator — S
`NOT STARTED`

Two arithmetic defects found on review. First, `max_tool_result_bytes` is a per-profile
constant while the body budget is computed per call, so a 32 KiB cap against 8 KiB of
remaining body admits one result that evicts the whole working state including the run's own
findings; the effective cap is `min(profile cap, remaining body)` at the moment the result
arrives. Second, calibrating the estimator to the **median** ratio delivers an under-estimate
on half of all calls, which contradicts the over-estimate guarantee the spec states — use p90.

**Files:** `crates/harness/src/context.rs`, `crates/harness/src/drive.rs`.
**Acceptance:** a result larger than the remaining body is capped to it and carries a handle;
the calibrated factor is the p90 ratio and the `budget` suite reports an over-estimate rate
≥ 95%.
**Traces:** AGENT-005 §2.2, §7; §10.4.

### S-1.19 The error contract, and not writing twice — M
`NOT STARTED`

Half of what a model learns it learns from being told no, and the error shape was never
specified — the UI already renders `error.code — error.fix` while every new tool invents its
own. Typed errors: closed per-namespace `code`, a mandatory `fix` that is **an edit the caller
can make**, `retryable` (the retry budget applies only to `true`; a wrong call is corrected,
not retried), ≤ 400 bytes, no stack traces. Plus idempotency keys on every write and
destructive call, derived from `(run_id, step, tool, canonical(arguments))` using the job
service's existing canonical-JSON hashing, so an in-run retry or a post-crash resume returns
the original result instead of acting twice.

**Files:** `crates/mcp-server/src/spec.rs`, `tools/*.rs`, `crates/jobs/src/manifest.rs`.
**Acceptance:** a free-text error code fails the build; a write replayed with the same key
performs the action once and returns the original result, asserted adversarially; a
non-retryable error consumes no retry budget.
**Traces:** AGENT-006 §2.4a, §2.4b; §10.9.

### S-1.20 Liveness: the two loops the protocol could sustain — S
`NOT STARTED`

Re-planning and retrying both look like progress to the barren-step breaker, so neither was
bounded. `max_replans_per_run` (3; 2 local) — on exhaustion the run wraps up with its working
state intact, because a report of what defeated it is a finding and a fourth plan is drift.
And a broken-tool circuit breaker: three consecutive non-retryable errors from one tool open a
breaker for the rest of the run, the tool leaves exposure, the model is told once, and the
failure is recorded as a `platform_defect` candidate rather than absorbed. An agent burning
fifteen steps on a tool the platform has broken is the platform's fault.

**Files:** `crates/harness/src/drive.rs`, `config/profiles/*.yaml`.
**Acceptance:** a fourth re-plan wraps up instead of continuing; a third consecutive
non-retryable error removes the tool from exposure and notifies once; both are profile values
with conformance readers.
**Traces:** AGENT-008 §3.7; §7.

---

## Exit criteria

- One `ToolSpec` per tool; taxonomy, profiles and envelopes generated; the four mechanical
  gates green in CI.
- The charter is ≤ 3,200 bytes and byte-stable; caching is on; `Usage` reports it.
- A 7B's body budget holds its floor with six schemas reserved, proven against the real
  profile YAML.
- Working state is a first-class object; plans name tools; decisions are not re-made.
- `cargo test -p harness --test conformance` and `--test failure_injection` green (both are
  CI jobs and both read profile fields this phase touches).
