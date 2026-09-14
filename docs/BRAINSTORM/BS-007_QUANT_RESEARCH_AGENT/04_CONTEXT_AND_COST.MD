# BS-007 / 04 — Context and cost: token efficiency without capability loss

**Spec status:** spec-ready draft · **Req prefix:** CX- · **Formal spec:** AGENT-001 §context ·
**Plan set:** L · **Decisions:** D-07, D-11 · **Evidence:**
[research/R4_TOKEN_EFFICIENCY_AUDIT.MD](research/R4_TOKEN_EFFICIENCY_AUDIT.MD). All token
and dollar figures here are estimates from that audit, not measurements.

---

## 1. Principle

**Never trade away capability. Remove waste.**

**Allowed:** anything that sends fewer redundant bytes, avoids cache misses, re-reads or
polling, or curates context so the model reasons better per token. Context rot is real:
every frontier model tested degrades as input grows.

**Never:**
- downgrade the main reasoning model;
- lower effort on hard reasoning;
- truncate information the model needs for its next decision;
- hide tool results it would use;
- skip verification, the Critic, nulls or replays to save tokens.

**Cheaper models** are allowed only for mechanical sub-tasks whose output a deterministic
checker verifies. Each use is flagged as a tradeoff and must pass the §10 non-inferiority
tests.

## 2. Cost model

On a coding-agent harness, 85–95% of input tokens are the cached prefix re-read every
turn. So:

> **Cost ≈ resident context × turns × cache-read price + cache rewrites + output.**
> At Opus-5 list prices, each 1k resident tokens costs about $0.0005 per turn: about
> $0.08 over a 150-turn session and about $0.40 over an 800-turn campaign.

**Estimated per-session cost (optimised):**
- Q&A: about $0.35–0.60;
- 3-hour build: about $15–25;
- overnight campaign: about $120–220;
- training loop: about $12–22;
- skill admission: about $20–90.

Built naively, the same sessions cost 3–5× more (R4 §2). The largest steady-state cost is
**skill and agent evaluation**, not user sessions (§8).

## 3. Prompt layout and caching

- **Layer order** (most stable first): tools → system core → CLAUDE.md (per project
  version) → skill listing → conversation. Put cache breakpoints at layer boundaries.
- **Byte-stable prefix:** no timestamps, cwd, git status or per-session values in the
  system prompt or CLAUDE.md. Move dynamic sections to the first user message (for
  example `excludeDynamicSections` *(verify)*). Serialise any tool lists
  deterministically.
- **The tool list never changes within a session.** There is no `load_toolset` or
  `unload_toolset`. Platform capabilities come through the `tbot` CLI (no schemas in
  context). Any MCP tools present are declared up front and deferred, found through tool
  search.
- **The model and effort never change within a session** (RT-19). Different work goes to
  sub-agents.
- **Skill listing:**
  - deterministic order (tier, then name), frozen per project version;
  - no counts, dates or rankings in descriptions;
  - glossary loaded on demand inside `skill-finder`, not resident;
  - revocations arrive as appended system messages, never as prompt edits (13 §5).
- **Cache-break telemetry:** alert when a turn writes more than 20k tokens to the cache
  outside a planned event (§9).

## 4. TTL and waiting on jobs

- The main loop uses the **1-hour cache TTL** (job waits routinely exceed 5 minutes; a
  rewrite at 90k context costs about $0.55). Sub-agent TTLs are set per role.
- **Waits are event-driven** (05 §5):
  - `tbot jobs wait` prints one final summary line;
  - long waits use `tbot jobs watch` as a background task that re-invokes the agent on
    completion;
  - **no polling loops**.
- **Compact before long waits:** if a wait is expected to exceed the TTL, compact at the
  boundary first. The cold rewrite afterwards is then about 30k tokens instead of about
  150k.
- The agent keeps working while jobs run: data prep for the next hypothesis, or sleep-time
  curation by a sub-agent (03 §9).

## 5. Compaction

- **Server-side, same model.** Summarising with a cheaper model is a false economy once
  caching exists: it reads 150k tokens uncached, and it summarises worse.
- **Trigger** at about 100–150k tokens of context. Keep resident baseline context ≤ 30%
  of the trigger, so compaction doesn't loop.
- **Compaction instructions** (in CLAUDE.md), recall-first. The summary MUST preserve:
  - hypothesis ids and statuses;
  - experiment, job and artifact ids;
  - trial counts and effective N;
  - gate states;
  - rejected ideas and why;
  - the research cutoff and budget state;
  - open questions;
  - the current plan step;
  - every number with its n, SE and handle.
- NOTEBOOK §1 is re-injected after a compaction, on the next user turn (03 §7; there is
  no `PostCompact` hook in the SDK — AGENT-001 §23).
- **Tool-result clearing** is a backstop, not a routine. If used, clear in large batches
  (each clearing rewrites the cache), keep the last 5–8 results, and never clear Set J
  evaluation results that haven't yet been recorded in NOTEBOOK.

## 6. Resident context and output hygiene

| Item | Budget | Rule |
|---|---|---|
| System core + CLAUDE.md | ≤ 8k tokens | Versioned; changes are harness releases evaluated by 16 |
| Skill descriptions (≈ 40) | ≤ 100 tokens each, ≈ 4k total | Registry enforces with `count_tokens` at admission (13 §8) |
| NOTEBOOK §1 "Current state" | ≤ 2k tokens | Read at start and after compaction, not every turn; edits as diffs |
| Bootstrap output | ≤ 40 lines | — |
| Tool or CLI output per call | Targets in 14 §4; hard cap 8k tokens | Output guard spills larger output to a file and returns a summary plus path (03 §7) |

**Output rules:**
- **Answers, not data:** summaries carry every number needed for the next decision, with
  n, SE and a handle. `--detail full` is the escape hatch. Track the re-fetch rate (§10).
- **Never dump Parquet or JSON** into context. Analyse in code; print summaries.
- **Reads:** use offset and limit slices and grep. The duplicate-read guard returns
  "unchanged since turn N" (03 §7).
- **Edits:** use diff edits, not whole-file rewrites. Output costs 5× input, and a
  300-line rewrite is about 15× the tokens of an edit.
- **Errors:** structured, ≤ 400 bytes (field, rule, fix). Stack traces go to a file.
- **Retries:** after 3 consecutive failures on one manifest family, the hook reminds the
  agent to diagnose instead of retrying.

## 7. Effort and model policy

- Orchestrator and all reasoning sub-agents: the project's pinned model at `high` or
  above for the whole session. Each effort step measurably buys quality on research-shaped
  tasks, so there is no free cut.
- **Flagged tradeoffs** (allowed only with a checker, and must pass §10): the Author
  sub-agent writing SLv2 or a DatasetSpec against the validator; log extraction checked
  against the source.
- If per-message effort changes are ever needed, use the per-message mechanism
  *(verify)*. Never restart the session's cache for it.

## 8. Fan-out, batch and eval economics

- **Fan-out:**
  - parallel workers share a byte-identical prefix (system, tools, brief) with the
    per-worker hypothesis last;
  - start one worker first and the rest once its response streams, so the rest read the
    cache;
  - Summariser and Citation passes are forks of the parent.
- **Message Batches** (50% off, stacks with caching) for offline single-turn work:
  - skill trigger and coexistence evals;
  - model-graded report quality;
  - promotion sanitiser checks;
  - tournament judgments;
  - curator reflection.
  Agent loops can't be batched.
- **Tournaments:** use a Swiss-system format (O(n log n)), not all-pairs Elo.
- **Replays and A/B arms:**
  - paired designs with common random seeds across arms;
  - sequential stopping with alpha spending;
  - leave-one-out only for skills with enough exposure;
  - each arm run only on tasks where the skill is exposed.
  This is an estimated 3–10× cheaper at the same type-I/II error rates (13 §10, 16 §7).
  **Evals always run at production model and effort.**

## 9. Telemetry, budgets and alerts

**Record:**

| Scope | Fields |
|---|---|
| Per model request (at the LLM proxy) | session, project, user, role (main, sub-agent name, curator, replay, eval arm), parent tool-use id, model, effort, TTL, turn index, input, cache-write (5m/1h), cache-read and output tokens, compaction iterations, stop reason, gap since previous request, prefix hash per layer |
| Per tool or CLI call | tokens returned, spilled to file (yes/no), detail level, re-fetched later (yes/no) |
| Per file read | path, hash, tokens, duplicate (yes/no) |
| Per job wait | duration, cache expired (yes/no), rewrite tokens, poll count |
| Per skill | exposed, activated, body tokens |

**Derived metrics:**
- cache hit rate;
- rewrite events;
- cost-weighted tokens (uncached + 1.25× 5m writes + 2× 1h writes + 0.1× reads + 5×
  output);
- context size over time;
- compactions per 100 turns;
- **dollars per correct verdict**.

**Alerts:**
- cache hit rate < 80% on sessions over 20 turns;
- any single result > 8k tokens;
- more than 3 rewrites per hour;
- spend > 80% of budget;
- sub-agent concurrency over the cap;
- daily curator or eval spend over budget.

The Usage and Cost Admin API is the billing source of truth. The proxy's records serve
attribution.

## 10. Quality guardrails and hazards

**Never optimised away:**
- the pinned model and effort;
- behaviour contract B-01…B-12;
- Critic, `final_report` validation, `skill_verify` replays, nulls and the dossier;
- held-out eval tasks and ≥ 3 seeds;
- any content the model needs next.

**Every optimisation ships alone and passes the eval suite (16) on pre-registered
non-inferiority margins:**
- pure-noise FDR ≤ nominal + 2 pp (pass^k);
- planted-edge power within −2 pp (95% CI);
- Q&A accuracy non-inferior;
- identical leakage-trap catch rate;
- uncited-number rate stays 0;
- **plus three tests specific to efficiency changes:**
  - *compaction amnesia:* ids and verdicts planted early are recalled after 2
    compactions;
  - *duplicate trials:* no already-tested manifest is re-run after compaction or resume;
  - *re-fetch rate:* how often `--detail full` follows a summary. Too high means the
    summaries are too thin.

**Hazards to design against** (full table in R4 §4):

| # | Hazard | Guard |
|---|---|---|
| H1 | Tool-list changes mid-session | §3 invariant |
| H2/H3/H4 | Per-turn state injection; re-ranked skill listings; dynamic values in the prefix | §3 byte-stable layers |
| H6/H7 | Polling; waits longer than the TTL | §4 |
| H8/H9/H10 | Data dumps; NOTEBOOK re-read every turn; whole-file reads and rewrites | §6 |
| H11/H15 | Fan-out resending context; runaway sub-agent trees or campaigns | §8, RT-14, budgets |
| H12/H13 | Compaction loops; compaction amnesia causing duplicate trials | §5, RT-17, amnesia test |
| H14 | Retry storms with stack traces in context | §6 |
| H16/H17/H19 | Replays, curator runs and A/B arms scaling with library size | §8, 13 §10 budgets |
| H18 | Skill nudges on every session | At most 1 nudge per session (03 §7) |
| H21 | Model routing inside the main loop | RT-19 |

## 11. Requirements

| ID | Requirement | Pri |
|---|---|---|
| CX-01 | The prompt MUST be layered as in §3, with a byte-stable prefix per project version and no dynamic values in system or CLAUDE.md | MUST |
| CX-02 | The tool set, model and effort MUST NOT change within a session | MUST |
| CX-03 | The main loop MUST use a 1-hour cache TTL; sub-agent TTLs MUST be set per role | MUST |
| CX-04 | Job waits MUST be event-driven; polling loops MUST NOT be used; waits expected to exceed the TTL SHOULD be preceded by compaction | MUST |
| CX-05 | Compaction MUST be same-model, server-side, with the §5 instruction set, and MUST be followed by a NOTEBOOK §1 re-read | MUST |
| CX-06 | Resident budgets in §6 MUST be enforced (registry token cap for skills; output guard for tool output) | MUST |
| CX-07 | Tool and CLI outputs MUST default to summaries carrying n, SE and a handle, with a `--detail full` escape hatch | MUST |
| CX-08 | A duplicate-read guard and diff-based editing SHOULD be the default | SHOULD |
| CX-09 | Cheaper models MAY be used only for §7 flagged tradeoffs, with a deterministic checker | MAY |
| CX-10 | Offline single-turn LLM work MUST use Message Batches where latency allows | MUST |
| CX-11 | Replays and A/B arms MUST use paired, sequential designs at production model and effort | MUST |
| CX-12 | The LLM proxy MUST record the §9 telemetry and raise the §9 alerts | MUST |
| CX-13 | Every efficiency change MUST pass the §10 non-inferiority suite before release | MUST |
| CX-14 | The UI MUST show per-session cost, cache hit rate and dollars per verdict | SHOULD |

## 12. Acceptance criteria

1. A 3-hour build session on the synthetic suite reaches a cache hit rate ≥ 85% and
   records no unplanned rewrite events.
2. A session with a 90-minute training job issues no polling calls, and resumes on
   completion with a cold rewrite ≤ 40k tokens (compacted before the wait).
3. The compaction-amnesia and duplicate-trial tests pass.
4. A tool call returning 50k tokens is spilled to a file; the model receives ≤ 1k tokens
   plus the path.
5. Turning off any single §3–§8 technique changes cost but no quality metric, beyond the
   §10 margins.

## 13. Open questions

1. Typical job durations per kind (sets the keep-alive vs compact-before-wait policy).
2. Cross-user cache sharing: freeze a shared core skill block ahead of per-project skills?
   Recommended: yes.
3. Should the model choice (for example Opus vs Fable class) be measured per workload on
   dollars per verdict? Recommended: yes, as an eval, never as a silent switch.
