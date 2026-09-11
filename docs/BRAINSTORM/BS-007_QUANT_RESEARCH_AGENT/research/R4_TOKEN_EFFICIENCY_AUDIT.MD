# BS-007 / R4 — Token-efficiency audit (research, 2026-09-11)

> **Informative.** This is an audit of the BS-007 design (then docs 00–12) as if it were
> built, produced by a research agent. It read every design doc plus
> `crates/api/src/agent/driver.rs`, `prompt.rs` and `crates/mcp-server/src/lib.rs`. The
> normative result is **04_CONTEXT_AND_COST.MD**, and the audit's spec changes have been
> applied across 03, 04, 05, 13, 14 and 16. Section references to "03/04/08/09/12" below
> use the **old** numbering; see the map in research/R1's header.
>
> **Pricing basis:** Opus 5 list prices per million tokens: $5 input, $25 output, $0.50
> cache read, $6.25 for a 5-minute cache write, $10 for a 1-hour cache write. **Every
> token and dollar figure is an estimate** built from the design targets and today's
> code. None of it is measured.

---

## TL;DR

- **Most of the design is already efficient.** It keeps data in files, has tools return
  answers rather than raw data, and puts slow work behind jobs. The remaining waste comes
  from two sources: prompt-cache invalidation and resident context. On a coding-agent
  harness, about 85–95% of input tokens are the unchanged start of the prompt re-read
  every turn. So cost ≈ (resident context × turns × $0.50 per million tokens) plus
  rewrites.
- **Rule of thumb:** each 1k resident tokens costs about $0.0005 per turn. That is about
  $0.08 over a 150-turn session and about $0.40 over an 800-turn campaign, before counting
  context rot.
- **Five things in the old spec would break caching if built as written:**
  1. `load_toolset`/`unload_toolset`;
  2. `ResearchState` injected at the top of each turn;
  3. a cheap summariser model;
  4. model routing inside the main loop;
  5. a skill listing and GLOSSARY that are re-ranked or regenerated per session.
- **Job waits collide with the default 5-minute cache lifetime.** The fix is a 1-hour TTL,
  completion notifications instead of polling, and compacting before long waits.
- **The largest steady-state cost is skill evaluation, not user sessions.** About
  1,440 agent runs per A/B cycle, roughly $3–8k. Sequential and paired designs, exposure
  gating and Batch cut this 3–10× with no loss of statistical power.
- **Adding caching naively to today's driver would make it worse.** Eliding one old tool
  result per turn past 400k characters rewrites the cache every turn, the placeholder
  loses the content, and `tokens_in` under-counts.
- **Quality guardrails:**
  - no cost-driven changes to the model or effort;
  - compaction on the same model;
  - summaries keep numbers, n, SE and a handle;
  - every optimisation passes the eval suite on non-inferiority margins, including new
    compaction-amnesia and duplicate-trial tests.

## 1. Baseline: `driver.rs` today

- **Static prefix of about 9–10k tokens, resent uncached every turn:** 38 tool schemas
  (about 5–6k), the embedded authoring guide (about 2.7k) and the protocol text (about
  1.2k).
- **Large results accumulate.** Tool results can each be up to 32 KB, and the transcript
  grows to about 100k tokens before elision starts. Calls run serially.
- **Cost of a 150-turn run:** about 9M uncached input tokens, about $45 plus output. With
  caching it would be about $6–8 in input.

## 2. Per-session token model (estimates)

Assumes about 28k resident tokens after bootstrap. "Naive" means the old spec built
literally.

| Session | Requests | Input tokens (share read from cache) | Output tokens | Optimised $ | Naive $ | Top drivers |
|---|---|---|---|---|---|---|
| a. 2-minute Q&A | 5–8 | 0.15–0.25M (60–75%) | 3–6k | 0.35–0.60 | 1.2–2.0 | Cold prefix write; thinking plus answer; re-reading the prefix |
| b. 3-hour build and evaluation | 150–240 | 12–18M (88–93%) | 150–250k | 15–25 | 35–50 (no cache: 60–90) | Context × turns; rewrites after waits longer than the TTL; output |
| c. Overnight campaign | 1,200–2,000 | 80–150M (88–92%) | 1–2M | 120–220 | 400–800+ | Parallel worker prefixes; orchestrator turns; per-submission checks and tournaments |
| d. In-session training loop | 100–160 | 8–12M (85–90%) | 150–300k | 12–22 | 35–60 | Waits longer than 1 hour; model code output; training logs |
| e. Skills lifecycle | 6–18 replay runs per admission | 10–40M | 0.3–1M | 20–90 per admission; A/B cycle about 3–8k | A/B cycle about 10–25k | Arms × tasks × seeds; replays; library re-evaluation on model upgrades |

The old "≤ 60k tokens per answer" target can't be met as raw summed input on this
harness. It is restated as a cost-weighted ceiling (about $0.60 per Q&A answer).

## 3. Techniques (condensed; adopted in 04)

| # | Technique | Saving (est.) | Quality | Adopted in |
|---|---|---|---|---|
| T1 | Stable layered prompt cache, TTL policy, keep-alive vs compact-before-wait | 2.5–3.7× on agent loops | Neutral | 04 §3–4 |
| T2 | Never change the tool list; defer tools and use tool search | About 85% of tool-definition tokens; removes rewrites | Positive (selection accuracy) | 04 §3, 14 §1 |
| T3 | Diet for resident context (skills, glossary, NOTEBOOK, CLAUDE.md) | 5–15k resident tokens | Positive (context rot; compact skills) | 04 §6, 13 §5, §8 |
| T4 | Server-side same-model compaction with quant-specific instructions | Cheaper than a cheap summariser once caching exists | Positive or neutral | 04 §5 |
| T5 | Tool-result clearing with handles, in large batches | Up to 84% on long searches; rewrites the cache | Positive only if evidence is recorded first | 04 §5 (backstop) |
| T6 | Answers, not data; output guard; `--detail full` escape hatch | 20–65% per result | Neutral if summaries keep n and SE | 04 §6, 14 §4 |
| T7 | Code execution over tool chains | Up to 98.7% on data-heavy chains | Positive to neutral | 14 §1 |
| T8 | Event-driven waits | Removes polling (about $1–2 per long job) plus rewrites | Neutral | 04 §4, 05 §5 |
| T9 | Condensed sub-agent returns; shared-prefix staggered fan-out | About $10–25 per campaign | Critic kept fresh | 03 §9, 04 §8 |
| T10 | Dedup reads, slices, diff edits | About $14 per campaign in output alone | Neutral | 03 §7, 04 §6 |
| T11 | Structured outputs plus deterministic validators; LLM citation pass only on flagged claims | Avoids full-context retries | Positive | 03 §10 |
| T12 | Message Batches for offline single-turn work | 50% | Neutral | 04 §8 |
| T13 | Sequential, paired designs for replays and A/B arms | 3–10× on eval cycles | Neutral (error rates held) | 13 §10, 16 §7 |
| T14 | Effort stable and high; cheaper settings only for mechanical, checked work | — | Positive | 03 §9, 04 §7 |
| T15 | Budgets, token counting, streaming (prevents timeout resends) | — | Positive | 03 §13, 04 §9 |

## 4. Hazards (full list)

| # | Hazard | Detection | Mitigation |
|---|---|---|---|
| H1 | Tool-group swaps | Cache writes ≈ full context after a swap | Stable tool list |
| H2 | Per-turn state injection | Hit rate < 50% from turn 1 | File-based state; append only when it changes |
| H3 | Skill listing regenerated or re-ranked per session | Listing hash varies across sessions | Deterministic order; frozen per project version |
| H4 | Timestamps, cwd or run constraints in the prompt | Cache diagnostics shows a system-prompt divergence | Dynamic sections go in the first user message |
| H5 | Revocation by editing the system prompt | Rewrite spike after a quarantine | Appended system message |
| H6 | Polling job status | Get/wait calls per job | Events |
| H7 | Waits longer than the TTL | Gap > TTL followed by writes > 20k | 1-hour TTL; compact first |
| H8 | Parquet or JSON in results | Results > 8k tokens | Output guard |
| H9 | NOTEBOOK re-read every turn | Duplicate-read ratio | ≤ 2k state section; reread after compaction only |
| H10 | Whole-file reads and rewrites | Write-vs-Edit ratio | Slices; diffs |
| H11 | Fan-out resending the full context | Writes per spawn | Shared prefix; stagger; caps |
| H12 | Compaction loops | Compactions per 100 turns; post-compaction size ÷ trigger > 0.5 | Resident ≤ 30% of trigger |
| H13 | Compaction amnesia causing duplicate trials | Repeated manifest hashes | Idempotent jobs; amnesia test |
| H14 | Retry storms with stack traces | Consecutive failures | Structured errors; retry reminder |
| H15 | Runaway campaigns and sub-agent trees | Burn rate; tree depth | Budgets; depth 1 |
| H16 | Replays scaling with library size | Replay dollars per admission | Batch; sequential designs; budgets |
| H17 | Unbounded curator runs | Curator input tokens | Structured deltas; caps; Batch |
| H18 | Stop-hook nudges every session | Tokens after the first report | At most 1 nudge |
| H19 | Eval A/B arms | Dollars per cycle | Sequential designs; exposure gating |
| H20 | Today's driver elision and under-counting | Writes every turn | Batch elision; count all usage fields |
| H21 | Model routing in the main loop | Model changes per session | Sub-agents only |

## 5. Open questions raised

1. Billing path (API key vs subscription), which sets TTL defaults.
2. Typical job durations (keep-alive vs compact-before-wait).
3. Cross-user cache sharing with per-project skill sets (the recommendation: a shared
   core block first).
4. Monthly eval budget.
5. Model choice per workload, measured on dollars per verdict: an eval, never a silent
   switch.

## Sources

- **Anthropic API docs:**
  - [Prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
  - [Compaction](https://platform.claude.com/docs/en/build-with-claude/compaction)
  - [Context editing](https://platform.claude.com/docs/en/build-with-claude/context-editing)
  - [Tool search](https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool)
  - [Programmatic tool calling](https://platform.claude.com/docs/en/agents-and-tools/tool-use/programmatic-tool-calling)
  - [Memory tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool)
  - [Batch processing](https://platform.claude.com/docs/en/build-with-claude/batch-processing)
  - [Citations](https://platform.claude.com/docs/en/build-with-claude/citations)
  - [Structured outputs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs)
  - [Token counting](https://platform.claude.com/docs/en/build-with-claude/token-counting)
  - [Cost and intelligence](https://platform.claude.com/docs/en/about-claude/models/optimizing-for-cost-and-intelligence)
- **Claude Code and Agent SDK docs:**
  - [Claude Code prompt caching](https://code.claude.com/docs/en/prompt-caching)
  - [Costs](https://code.claude.com/docs/en/costs)
  - [Skills](https://code.claude.com/docs/en/skills)
  - [Tools reference](https://code.claude.com/docs/en/tools-reference)
  - [Agent SDK cost tracking](https://code.claude.com/docs/en/agent-sdk/cost-tracking)
  - [Agent SDK subagents](https://code.claude.com/docs/en/agent-sdk/subagents)
  - [Agent SDK system prompts](https://code.claude.com/docs/en/agent-sdk/modifying-system-prompts)
- **Anthropic engineering blog:**
  - [Advanced tool use](https://www.anthropic.com/engineering/advanced-tool-use)
  - [Code execution with MCP](https://www.anthropic.com/engineering/code-execution-with-mcp)
  - [Effective context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
  - [Writing tools for agents](https://www.anthropic.com/engineering/writing-tools-for-agents)
  - [Multi-agent research system](https://www.anthropic.com/engineering/built-multi-agent-research-system)
  - [Context management](https://claude.com/blog/context-management)
  - [Prompt caching is everything](https://claude.com/blog/lessons-from-building-claude-code-prompt-caching-is-everything)
- **Research:**
  - [Chroma context rot](https://www.trychroma.com/research/context-rot)
  - [Complexity Trap](https://arxiv.org/abs/2508.21433)
  - [ACON](https://arxiv.org/abs/2510.00615)
  - [Context-Folding](https://arxiv.org/abs/2510.11967)
  - [Masking regime map](https://arxiv.org/abs/2606.00408)
  - [ACM](https://arxiv.org/abs/2607.23809)
  - [SkillsBench](https://arxiv.org/html/2602.12670)
