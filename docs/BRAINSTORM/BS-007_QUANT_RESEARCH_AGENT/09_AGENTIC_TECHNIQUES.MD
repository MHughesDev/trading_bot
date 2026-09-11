# BS-007 / 09 — Agentic techniques to build into the agent (research, 2026-09-10)

Status: research summary + adoption proposals. Each technique lists its source and
evidence, then **how it lands in this platform** (component names from 08). Evidence
labels: **[Prod]** used in a shipped production system · **[Bench]** benchmark result ·
**[Paper]** single study · **[Idea]** my design synthesis.

---

## 0. Top 10, in adoption order

| # | Technique | Where it lands | Why first |
|---|---|---|---|
| 1 | **State lives on disk, not in context** (initializer + plan/progress files + git + boot check) | Project workspace template, session bootstrap | Long runs fail without it; costs almost nothing |
| 2 | **Code execution over tools** (tools as SDK modules, filter data in the sandbox) | `tbot` SDK + container | Reported ~98.7% token cut in Anthropic's example; matches our Parquet-in-workspace design |
| 3 | **Proposal ⟂ audit ⟂ evaluation ⟂ holdout** separation, with the auditor itself tested on planted violations | Data API cutoff, Gate 0 auditor, eval suite | The one design choice the quant-agent literature converges on |
| 4 | **Matched-budget random-search baseline** for every campaign | Campaign engine, eval suite | A 2026 PIT-audited study found an adaptive agent only *tied* random search |
| 5 | **Structured process memory** (AlphaMemo: edit motifs → residual outcomes, confidence-gated, negative veto) | Research memory (L2) | Memory that learns *which moves work*, without chasing lucky outcomes |
| 6 | **Evolving playbook** (ACE: itemised bullets; generator → reflector → curator) | `.claude/skills/playbook` + curator job | Avoids "context collapse" of rewritten summaries |
| 7 | **Tree search over solutions** (AIDE / AI-Scientist-v2) for model and strategy code | Research loop inside the agent; job service | 4× more medals than linear agents on MLE-bench |
| 8 | **Quarantined reader for untrusted text** (dual-LLM / plan-then-execute) | News/social/web tools | Prompt injection via news is the agent's main attack surface |
| 9 | **Sleep-time compute** while jobs run | Background subagent during `jobs wait` | Long backtests/training leave the agent idle; use that time |
| 10 | **Harness self-improvement from transcripts** (AHE) | Eval suite + transcript analysis loop | Measurable gains from evolving prompts/tools/skills with the model fixed |

---

## 1. Harness and long-horizon operation

### 1.1 Initializer + on-disk state + boot check  [Prod]
Anthropic's long-running harness uses an **initializer agent** once per project: it
expands the goal into a structured, testable list (a JSON feature list with every item
starting "failing", and rules forbidding the agent from deleting requirements), writes an
`init.sh` boot script, and makes a baseline git commit. Every later session reads the
progress file and git log, runs `init.sh` to catch a broken state, works on **one item**,
verifies it end to end, commits, and updates the progress file. The failures this fixed
were premature victory, undocumented progress, and building on a broken state.
([Anthropic](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents),
[reference repo](https://github.com/anthropics/cwc-long-running-agents))

**Here:** the project workspace template gets:
- `RESEARCH_PLAN.json`: hypotheses as items with `status: untested | running | rejected | supported | vaulted`,
  pre-registered null and kill criterion. The agent may add items and change status, but
  never delete them (INV-1 at the plan level).
- `PROGRESS.md` + git history: one commit per completed step, message citing job and experiment ids.
- `bootstrap.sh`: checks platform reachability, data coverage for the project instruments,
  pending jobs (`tbot jobs list --mine`), and the research cutoff, then prints a
  ≤ 40-line status. The session always starts by running it.
- Rule: one hypothesis step per work unit; a step is "done" only when its job result is
  attached.

### 1.2 Context engineering: compaction, tool-result clearing, notes, sub-agents  [Prod]
Anthropic's guidance: compaction (tune the summary prompt for recall first, then
precision), **tool-result clearing** as the lightest form, structured note-taking outside
the window, and sub-agents with clean contexts.
([Anthropic](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents),
[cookbook](https://platform.claude.com/cookbook/tool-use-context-engineering-context-engineering-tools))
Chroma's **context rot** study found every one of 18 frontier models degrades as input
grows, well before the window is full.
([Chroma](https://www.trychroma.com/research/context-rot))

**Here:** keep the working context small on purpose. Data goes to files; results go to
`PROGRESS.md`/`NOTEBOOK.md`; old tool results get cleared; compaction is a backstop. Treat
"tokens in context" as a cost to minimise, not a capacity to fill.

### 1.3 Code execution with MCP: tools as code  [Prod]
Present tools as code modules on the filesystem (`./servers/<server>/<tool>.ts`) or behind a
`search_tools` call. The agent writes code that calls them, filters and joins data inside
the sandbox, persists intermediate results to files, and saves reusable functions as
**skills**. Anthropic's example cut 150k tokens to 2k (98.7%). Caveat: it needs a
sandbox with resource limits and monitoring.
([Anthropic](https://www.anthropic.com/engineering/code-execution-with-mcp),
[Simon Willison's notes](https://simonwillison.net/2025/Nov/4/code-execution-with-mcp/))

**Here:** this is exactly the `tbot` SDK in the container (08 §1). Also generate
`tbot/tools/<group>/<tool>.py` stubs from the MCP tool definitions, so the tool surface and
the SDK never drift. Let the agent save proven analysis code to `skills/` (for example
`skills/event_study_matched.py`), which becomes the project's growing library.

### 1.4 Tool design for agents  [Prod]
From Anthropic's "writing tools for agents": choose fewer, higher-level tools; namespace
them (and evaluate prefix vs suffix naming); return meaningful, human-readable fields
rather than opaque ids; offer a `response_format` enum (concise/detailed); paginate,
filter and truncate with sensible defaults; keep responses under ~25k tokens; and
**improve tools by having an agent analyse eval transcripts**.
([Anthropic](https://www.anthropic.com/engineering/writing-tools-for-agents))

**Here:** add `response_format` to every MCP/SDK read (03 §1 "progressive disclosure"),
return instrument symbols and strategy slugs alongside UUIDs, and namespace tools by
group (`data_*`, `feat_*`, `bt_*`, `exp_*`, `model_*`, `mem_*`).

### 1.5 Deferred tool loading and programmatic tool calling  [Prod]
With ~100 tools, mark most as deferred and let the agent search for them (tool search
appends schemas without breaking the prompt cache). For chains of calls with large
intermediate results, programmatic tool calling runs the chain inside code execution and
returns only the final output. (Anthropic API docs, via the `claude-api` skill.)

**Here:** in the container most calls go through the SDK anyway. For the MCP path
(external clients), ship tool search and keep ≤ 15 always-loaded tools.

---

## 2. Search and exploration

### 2.1 Solution-tree search (AIDE)  [Bench]
Each script is a node in a solution tree. LLM patches create children (draft, debug,
improve), a metric evaluates each node, and a selector picks where to expand next. On
OpenAI's MLE-bench, AIDE's tree search won about 4× more medals than the best linear agent.
([AIDE paper](https://arxiv.org/abs/2502.13138), [code](https://github.com/wecoai/aideml))
AI-Scientist-v2 adds an **experiment-manager agent** over a progressive agentic tree search.
([Sakana](https://arxiv.org/abs/2504.08066))

**Here:** use it for **model development** (Phase 3): each node is a `models/*.py` +
`DatasetSpec` training job, scored on walk-forward CRPS/QLIKE against baselines. Use it
for **Layer 1 strategy code** too, where the node metric is the *sealed* Set J distribution
(worst-5% or median), never a single backtest, so tree search doesn't become argmax
hunting. Store the tree in the job service so the UI can render it.

### 2.2 Evolutionary program search  [Bench]
AlphaEvolve → OpenEvolve (MAP-Elites, diff-based edits) → ShinkaEvolve (bandit-based LLM
ensemble, **novelty-based rejection filter** for sample efficiency) → CodeEvolve (islands +
CVT-MAP-Elites, inspiration-based crossover, meta-prompting; beats OpenEvolve/ShinkaEvolve
on 6 of 9 matched problems, and does it cheaply with an open-weight model).
([CodeEvolve](https://arxiv.org/abs/2510.14150),
[AlphaEvolve overview](https://en.wikipedia.org/wiki/AlphaEvolve))

**Here:** FEAT-003 Phase 2 already specifies islands + MAP-Elites + bandit allocation
(QuantEvolve lineage). Add from this line of work: a **novelty rejection filter** before any
backtest (AST/structure similarity + signal-correlation de-dup), **diff-based moves** on
Strategy Language v2 text, and **inspiration crossover** (show the proposer 2–3 elites from
other cells).

### 2.3 Tournament ranking of hypotheses (AI co-scientist)  [Paper / Prod]
Google's co-scientist runs generate → debate → evolve. A ranking agent holds pairwise
"debates" between hypotheses and maintains Elo ratings, and higher-rated hypotheses get
refined first. Quality rose with more compute.
([Google Research](https://research.google/blog/accelerating-scientific-breakthroughs-with-an-ai-co-scientist/),
[Nature](https://www.nature.com/articles/s41586-026-10644-y))

**Here:** use a cheap Elo tournament over *untested* hypotheses to **prioritise which get
compute**, judged on rationale ("who pays you?"), novelty against research memory, data
feasibility and expected cost. **Never as evidence:** Elo orders the queue, and Set J
judges the result. Log tournament outcomes vs eventual gate results to check whether the
ranker is informative; drop it if it isn't.

### 2.4 Matched-budget random-search baseline  [Paper]
A PIT-audited 2026 study on BTC perpetuals separated proposal, deterministic audit,
evaluation and holdout access. **Under an equal budget the adaptive agent tied random search**,
and every holdout run had positive IC but negative net Sharpe after costs.
([arXiv 2608.25348](https://arxiv.org/abs/2608.25348))

**Here:** every campaign runs a **shadow random-search arm** with the same backtest budget
over the same grammar. The campaign report shows agent vs random on the sealed metric. If
the agent doesn't beat random, the report says so. This is the single best check on
whether the agent's intelligence is adding anything.

---

## 3. Memory and learning across sessions

### 3.1 Structured search-process memory (AlphaMemo)  [Paper]
Memory is attached to **edges** (parent → child edits), not to final factors. Each edge
stores parent context (category, quality bucket, depth), an **AST-diff edit motif**, and the
**residual outcome** (child quality minus what a base prior expected). Retrieval asks: under
similar parent contexts, have these motifs produced gains beyond expectation?
**Confidence gating** mutes sparse/noisy memories. An **asymmetric process veto** lets
reliable negative patterns block actions, while positive patterns only nudge, "reflecting
financial reality where failure modes are more stable than fragile alpha signals".
([arXiv 2606.20625](https://arxiv.org/html/2606.20625v1))

**Here:** this is the right design for L2 research memory's *move* layer. Store
`(parent archetype, regime profile, gate reached) × move (typed, from the Strategy
Language v2 AST diff) → residual on the sealed metric, n, variance`. Negative veto =
"adding an RSI filter to trend families on crypto 1h has failed 9/9 times, don't propose it".
It also complements FEAT-003's regime→strategy prior table.

### 3.2 Evolving playbooks (ACE)  [Paper]
Context as an itemised **playbook** of bullets that a Generator uses, a Reflector
extracts lessons from, and a Curator merges as *delta updates* rather than rewrites. That
avoids **brevity bias** and **context collapse** (iterative rewriting erodes detail).
+10.6% on agent tasks and +8.6% on finance tasks vs baselines.
([arXiv 2510.04618](https://arxiv.org/abs/2510.04618))

**Here:** the diagnostic→move playbook (06 Part E) and pitfall list become a living
`skills/playbook/SKILL.md` of ids'd bullets with `helpful/harmful` counters. A curator
job runs after each session: it proposes add/edit/retire deltas from the session's
findings, and a human (or a gated rule) accepts them. Never regenerate the whole file.

### 3.3 Skills + experience memory loops (FactorMiner, XALPHA)  [Paper]
FactorMiner combines executable **skills** (evaluation as tools) with **experience
memory**, in a retrieve → generate → evaluate → distill loop that reduces redundant search
under the "correlation red sea".
([arXiv 2602.14670](https://arxiv.org/abs/2602.14670))
XALPHA splits planning ("macro brain"), hypothesis→code with **ex-ante tri-alignment**
checks (hypothesis ↔ code logic ↔ financial plausibility), and cross-cycle consolidation.
([arXiv 2607.08332](https://arxiv.org/abs/2607.08332))

**Here:** add a **tri-alignment check** before any job is submitted. A cheap sub-agent reads
the hypothesis, the strategy code and the pre-registered rationale, and flags mismatches
("hypothesis says mean reversion; code enters on breakouts"). It's cheap and catches a
classic LLM failure.

**Agent-created skills** are specified in full in [12_SKILLS.MD](12_SKILLS.MD). That
covers triggers, platform-side verification, the registry and glossary, materialisation
for every user, and quant admission tests. The ACE playbook (§3.2) lives in that
registry as `kind: playbook`.

### 3.4 Sleep-time compute  [Paper / Prod]
Letta's sleep-time agents share memory with the primary agent and reorganise and
consolidate it during idle time, shifting compute off the latency-critical path.
([Letta](https://www.letta.com/blog/sleep-time-compute/),
[docs](https://docs.letta.com/guides/agents/architectures/sleeptime/))

**Here:** our agent is idle whenever a backtest or training job runs. During
`tbot jobs wait`, spawn a low-effort background subagent that updates `NOTEBOOK.md`, drafts
the next hypotheses' data prep, curates playbook deltas, and pre-computes instrument
profiles. The primary context stays clean.

---

## 4. Verification and honesty

### 4.1 Separate proposal from audit, evaluation and holdout  [Paper]
The BTC-perps study (2.4) separated the agent's proposals from a **deterministic auditor**
(event/publication/availability timestamps per stream), evaluation, and holdout access.
The auditor was itself validated: it caught 40/40 planted violations with 0/40 false
rejections, and cut mean false passes on null-signal paths from 0.29 to 0.06.
([arXiv 2608.25348](https://arxiv.org/abs/2608.25348))

**Here:** this is 08 §1's enforcement principle. Add what we lacked: an **auditor test
suite** of planted violations (look-ahead features, overlapping labels, revised-data use,
models trained into the test window, costs off), run in CI against Gate 0.

### 4.2 Constrained generation + AST sandbox (Hubble)  [Paper]
Rather than unconstrained code, Hubble restricts the LLM to a domain operator language,
executes candidates in an AST sandbox through a deterministic pipeline, and feeds back
family-level diagnostics. The result was 100% computational stability.
([arXiv 2604.09601](https://arxiv.org/abs/2604.09601))

**Here:** validates the two-layer design. Strategy Language v2 is the constrained,
AST-evaluated layer, and families (archetypes) give the feedback structure. Layer 1 code
stays available for the long tail, with the truncation test as its safety net.

### 4.3 Citation pass  [Prod]
Anthropic's research system runs a separate **CitationAgent** after synthesis.
([Anthropic](https://www.anthropic.com/engineering/built-multi-agent-research-system))

**Here:** the Stop-hook `final_report` validator (04 §11), plus a citation subagent that
attaches experiment/job/artifact ids to each claim and flags uncited numbers.

### 4.4 Validation regularisers against overfit ideas (AlphaAgent)  [Paper]
Penalise **complexity** (expression-tree size, parameter count), low **originality** (AST
similarity to the existing library), and poor **hypothesis alignment** (LLM-scored).
Reported better decay resistance on CSI 500 and S&P 500.
([arXiv 2502.16789](https://arxiv.org/abs/2502.16789))

**Here:** add complexity and originality penalties to the campaign's *proposal scoring*
(not to Set J's verdict), alongside FEAT-003's `simplify` move.

---

## 5. Multi-agent structure  [Prod]

Anthropic's research system (lead + 3–5 parallel subagents + citation pass) beat
single-agent Opus by 90.2% on their internal eval, at ~15× chat tokens. Token usage
explained ~80% of performance variance. The main lesson: **architecture follows task
structure**, so multi-agent only wins when work splits into independent threads. Early
versions over-spawned. Their prompts use explicit scaling rules (1 agent for simple
lookups, 2–4 for comparisons, 10+ for complex research).
([Anthropic](https://www.anthropic.com/engineering/built-multi-agent-research-system),
[when to use multi-agent](https://claude.com/blog/building-multi-agent-systems-when-and-how-to-use-them))

**Here:**
- Parallelise **across hypotheses** (independent), each subagent on its own git branch and
  Experiment; the lead merges findings.
- Keep one agent for a single hypothesis's sweep→diagnose→move loop (sequential by nature).
- Standing roles: **Critic** (falsify), **Auditor-reader** (tri-alignment, 3.3),
  **Quarantined reader** (6), **Citation** (4.3).
- Put scaling rules in the prompt, and cap concurrent subagents by the job-queue budget.

---

## 6. Security: untrusted text  [Paper]

Six patterns from a Google/Microsoft/IBM/ETH/EPFL paper. Core rule: once an agent has
ingested untrusted input, it must be constrained so that input can't trigger consequential
actions. Patterns: action-selector, **plan-then-execute**, map-reduce, **dual LLM**
(a privileged planner never reads untrusted text; a quarantined worker reads it, holds no
tools, and returns symbolic variables), **code-then-execute** (CaMeL), and context
minimisation.
([arXiv 2506.08837](https://arxiv.org/abs/2506.08837),
[Simon Willison](https://simonwillison.net/2025/Jun/13/prompt-injection-design-patterns/))

**Here:** news, Reddit, web pages and filings are read only by a **quarantined reader
subagent** with no tools and no network. It returns typed extractions (entity, event type,
sentiment, novelty, quote spans) that the main agent consumes as data. The container's
egress allowlist and research-only token cap the blast radius anyway: the agent can't trade.

---

## 7. Improving the harness itself

### 7.1 Agentic Harness Engineering  [Bench]
Hold the model fixed and evolve the harness (prompts, tool descriptions, tool code,
middleware, skills, sub-agents, memory). It rests on three observability pillars: every
component as an editable, revertible file; trajectories distilled into a drill-down
evidence corpus; and every edit paired with a **predicted effect verified next round**.
Ten iterations raised Terminal-Bench 2 pass@1 from 69.7% to 77.0%, above the human-designed
Codex-CLI harness.
([arXiv 2604.25850](https://arxiv.org/abs/2604.25850),
[code](https://github.com/china-qijizhifeng/agentic-harness-engineering))

**Here:** put the system prompt, skills, playbook and tool descriptions in versioned files
(the container already does this). Run the eval suite (04 §9) after each harness change,
with a stated prediction ("adding the tri-alignment check reduces wasted backtests by
≥ 10%"). Keep an edit only if the prediction holds.

### 7.2 Agent evals done right  [Prod]
Anthropic's eval anatomy covers task, trial, agent harness, eval harness, transcript,
outcome, grader and suite. Combine code-based, model-based and human graders. Use **pass@k**
when one success suffices (research) and **pass^k** when consistency matters. Read the
transcripts, because many "failures" turn out to be grader bugs.
([Anthropic](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents),
[summary](https://ai-eval.org/deep-dive/anthropic-demystifying-evals-for-ai-agents))

**Here:** the planted-edge / pure-noise / leakage-trap / Q&A suites get code graders
(detection, false discovery, numeric tolerance), a model grader for report quality, and
monthly human review of a transcript sample. Report pass@k for discovery tasks and pass^k
for "never reports an edge on noise".

---

## 8. Adoption map to 08 phases

| 08 phase | Techniques |
|---|---|
| 1 Agent foundation | 1.1 on-disk state · 1.2 context hygiene · 1.3 code-over-tools · 1.4 tool design · 4.3 citation pass · 6 quarantined reader · 7.2 eval suite |
| 2 Engine truthfulness | 4.1 auditor + planted-violation tests · 4.2 constrained layer (with SLv2 in Phase 4) |
| 3 Models | 2.1 solution-tree search for model code · 3.4 sleep-time during training jobs |
| 4 Strategy Language v2 | 2.2 diff-based moves + novelty filter · 4.4 complexity/originality penalties |
| 6 Knowledge + autonomy | 3.1 process memory · 3.2 evolving playbook · 3.3 tri-alignment · 2.3 tournament prioritisation · 2.4 random-search shadow arm · 5 parallel hypothesis workers · 7.1 harness evolution |
