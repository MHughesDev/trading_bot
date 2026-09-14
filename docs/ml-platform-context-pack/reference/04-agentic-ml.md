# Agentic ML Engineering: Benchmarks, Architectures, Tools, Memory, Safety, Verification

**Scope:** state of the art as of September 2026, oriented toward a platform where AI agents autonomously design, run, and iterate on ML training experiments in quantitative trading.
**Bottom line up front:** the field has converged on a *search-over-a-solution-tree* architecture with typed edit operators, scoped memory, and an execution sandbox. The binding constraints in 2026 are **not** the search algorithm — they are (a) operator quality, (b) the validation→holdout generalization gap, (c) context/compaction discipline, and (d) budget awareness. For quant trading, (b) is existential: the generalization gap that costs 9–13 points of Kaggle medal rate costs *all* of your alpha.

---

## 1. Benchmarks & systems for autonomous ML engineering

### 1.1 The benchmark landscape

| Benchmark | Shape | Notes / status 2026 |
|---|---|---|
| **MLE-bench** (OpenAI, 2410.07095) | 75 Kaggle comps, offline, graded vs. real leaderboards (bronze/silver/gold). "Lite" = 22 low-complexity comps. | The de facto standard. Still the leaderboard everyone reports on. |
| **MLAgentBench** | Research-task scripts (improve a given baseline). | Superseded; weak scaffold, 0.8% medal on MLE-bench. |
| **DSBench** | Data-science analysis + modeling from realistic task descriptions. | Lower ceiling, less used for scaffold research. |
| **MLGym / MLE-Dojo** | Gym-style *environments* (step/reset), designed for RL training of agents, not just eval. | The right shape if you intend to fine-tune your own agent policy. |
| **RE-Bench** (METR) | 7 hand-built AI R&D tasks vs. human expert baselines, time-budgeted. | Key result: agents beat humans at *short* budgets (2h), humans win at 8–32h. Long-horizon is the gap. |
| **METR time horizons (TH1.1, Jan 2026)** | 50%-success task length. | Claude Opus 4.5 ≈ 320 min, GPT-5 ≈ 214 min. Doubling time since 2024 ≈ **89 days** (faster than the 2019–2025 rate of ~196 days). Only 5 of 31 long (8h+) tasks have human baselines — treat long-horizon numbers as soft. |
| **PaperBench** (OpenAI) | Replicate 20 ICML'24 papers against hierarchical rubrics (~8,300 leaf criteria). | Originally ~21% for Claude 3.5 Sonnet + open scaffold. Third-party 2026 leaderboards report 80–93% for frontier models; **treat these as unverified aggregator numbers**, the rubric grading is LLM-judge based and PaperBench's papers are now heavily in-distribution. |
| **AIRS-Bench** (2602.06855, Feb 2026) | 20 tasks from recent SOTA papers; {problem, dataset, metric}, **no baseline code**; Elo + "march of 9s" normalized score. | Unsaturated. Best config (greedy + gpt-oss-120b) ≈ 40% normalized; **only 55% of submissions were even valid**; agents beat human SOTA on 1.58% of attempts. This is the honest picture of "autonomous research." |
| **SkillEvolBench** (2605.24117) | Does episodic experience convert into reusable procedural skills? | See §4 — mostly no. |
| **BAGEN** (2606.00198) | Are agents budget-aware? | See §6. |

### 1.2 MLE-bench leaderboard trajectory (medal rate)

- **2024 baseline:** AIDE + o1-preview = **16.9%** (full 75). AIDE+GPT-4o 8.7%; OpenHands 4.4%; MLAgentBench 0.8%. Scaffold mattered ~10x more than model at that point.
- **R&D-Agent (Microsoft):** 22.4% full bench.
- **ML-Master v1 (2506.16499):** **29.3%** full bench, 93.3% valid-submission rate, in **12h** (half the standard budget). Medium-difficulty 20.2% (vs 9.0% prior).
- **AIRA-dojo (Meta, 2507.02554):** MLE-bench **Lite** 39.6% → **47.7%**; AIRA-MCTS 47.0% / AIRA-greedy 45.5% with DeepSeek-R1; o3 gets 30.9% *gold*. On the full bench, ~31.6% with o3.
- **MLE-STAR (Google, 2506.15692):** MLE-bench-Lite **63% medal, 36% gold** with Gemini — a different scaffold family (web-search-for-SOTA + ablation-driven targeted refinement + self-proposed ensembling + leakage/data-usage checker agents).
- **ML-Master 2.0 (Dec 2025 / 2026, SJTU-SAI / Eigen AI):** **56.44%** on the *full* 75-comp MLE-bench with DeepSeek-V3.2-Speciale in 24h. Low 75.8% / medium 50.9% / high 42.2%. Beats Leeroo (Gemini-3-Pro, 50.7%) and AIRA-dojo (31.6%).
- **pass@k scaling:** MLE-bench's own ablation — GPT-4o 16.9% pass@1 → **34.1% pass@8**. Simply running more independent attempts roughly doubles medal rate. This is the cheapest known win and it is *pure verifier-limited selection*.

### 1.3 What architectures actually score best

Three converging design patterns account for essentially all the 2025→2026 gains:

1. **Tree search over solution nodes with typed edit operators.** AIDE established `Draft / Debug / Improve` over a solution tree; AIRA and ML-Master both added MCTS/UCT with parallel node expansion. ML-Master 2.0's headline jump came from *parallel* MCTS worker expansion rather than sequential node selection.
2. **Scoped, structured memory injected into the reasoning trace — not raw logs.** ML-Master's "steerable reasoning" injects curated parent + sibling insights into the reasoning stream; v2 caps distilled context at **4,096 tokens** and uses a 3-layer *Hierarchical Cognitive Cache* (L1 within-phase execution feedback → L2 cross-phase tactical revision → L3 cross-task strategic heuristics). AIRA independently found the same: *sibling* memory for draft/improve (promotes diversity), *ancestral* memory for debug (prevents oscillation).
3. **Targeted, localized edits instead of whole-file rewrites.** MLE-STAR runs an ablation to find the highest-leverage code block, then iterates *only on that block*. This is the single most underrated result for a quant platform: it makes the diff small, reviewable, and attributable.

### 1.4 What actually limits them

The AIRA paper is the most decision-relevant ablation in the literature:

- **Operators, not search, are the bottleneck.** MCTS and evolutionary search gave **zero gain** over greedy when paired with AIDE's original operator set. Varying the MCTS exploration constant barely moved anything. You cannot buy performance with a fancier search policy if your edit operators are weak. Conversely, with better operators the search policy *does* start to matter (greedy 45.5 → MCTS 47.0).
- **The generalization gap is the dominant loss.** Selecting the final submission by *test* score instead of *validation* score buys **9–13 points** of medal rate (15 points for greedy). Final-node selection alone accounts for 9–11.6 absolute points. Submitting the **top-3 validation nodes instead of top-1 recovers ~10%**. The search is systematically overfitting its own validation signal. **In trading this is the whole ballgame — your search *is* a multiple-testing machine.**
- **Context overflow and accumulated debugging debt.** AIRS-Bench names these explicitly: long agentic traces degrade reasoning, and debug failures compound until the trajectory is unrecoverable.
- **Compute budgeting is absent.** MLE-bench observed agents "rarely verbalized any consideration of how long their produced code would run." BAGEN confirms it quantitatively (§6).
- **Submission/format validity.** 45% invalid on AIRS-Bench; early self-termination on general-purpose scaffolds. Plumbing, not intelligence.
- **Long-horizon returns are real but sublinear.** MLE-bench at 100h vs 24h: most medals arrive in the first couple of hours, then slow accumulation. AIRA at 120h still improved. Budget past ~24h has positive but sharply diminishing marginal value.

---

## 2. Search strategies over experiment space

### 2.1 What the evidence supports

| Strategy | Evidence | Verdict for a quant platform |
|---|---|---|
| **Greedy iteration (AIDE)** | 39.6% Lite baseline; strong floor. | Fine default; the largest generalization gap (15 pts) because it commits early to a validation-overfit branch. |
| **MCTS / UCT over solution nodes** | AIRA-MCTS 47.0% (best) *only with good operators*; ML-Master 1 & 2 both MCTS. | **Recommended.** But its value is mostly *parallelism + diversity*, not clever exploration constants. |
| **Evolutionary / genetic program search** | AIRA-evo slightly below MCTS on MLE-bench. But **ShinkaEvolve** (Sakana, 2509.19349) found SOTA circle-packing in **150 samples** vs AlphaEvolve's thousands, and beat DeepSeek's MoE load-balancing loss in 30 generations. Wins come from: parent sampling that balances exploit/explore, **novelty-based rejection filtering** (embedding + LLM novelty judge kills near-duplicate proposals before you pay for evaluation), and **bandit-based LLM ensemble selection**. | **Recommended for alpha/factor search specifically.** QuantaAlpha (2602.07085) and AlphaAgent (2502.16789) both use evolutionary mutation+crossover over factor expressions with AST-based redundancy filtering. |
| **Best-of-N + verifier** | MLE-bench pass@8 doubles pass@1. Top-3 submission recovers ~10% of the generalization gap. | **Highest ROI single change.** But it is bounded by verifier quality (§7). |
| **Reflection / critique loops** | Mixed. "Self-Correction Illusion" (2606.05976): the inability to self-correct is largely a **chat-template artifact** — relabeling an identical erroneous claim from `<thought>` to a `user`/`tool`/`<memory>` role lifts explicit-correction rates by **23–93 points**. | Use critique, but **present the artifact to the critic as external content** (tool output / memory block), never as the model's own prior thought. Cheapest known reliability fix. |
| **Self-consistency / debate** | Single-agent matches or beats debate/sequential/parallel MAS at **equal thinking-token budgets** (2604.02460); plateau at 1k–2k thinking tokens. MAS only wins under heavy context degradation (70% noise). Homogeneous debate collapses to 3-0 votes ("debate diversity collapse"). | Do **not** buy debate with homogeneous models. If you debate, use **heterogeneous backbones**. |

### 2.2 The operator set matters more than the search

Concrete operator design that is empirically validated:

- `draft` / `improve` / `debug` as the base triple (AIDE).
- `crossover` — recombine two high-scoring nodes (AIRA; QuantaAlpha crossover over trajectory segments).
- **Prompt-adaptive complexity**: emit a complexity cue ("minimal" / "moderate" / "advanced") conditioned on the number of existing sibling nodes. Prevents the search from generating 12 near-identical baselines.
- **Scoped memory per operator type**: siblings for draft/improve (diversity), ancestors for debug (anti-oscillation).
- **Think tokens**: explicit extended reasoning roughly doubles completion tokens but is a net win for reasoning backbones.
- **Ablation-targeted refinement** (MLE-STAR): run a cheap ablation to rank pipeline blocks by contribution, then constrain the next edit to the top block.
- **Novelty rejection before evaluation** (ShinkaEvolve): embed the proposed diff, reject if cosine-similar to an already-evaluated node. In quant this doubles as *multiple-testing control* — you are not allowed to spend a backtest on a near-duplicate factor.

### 2.3 Does anything beat greedy?

Yes, but conditionally. Ranked by expected value per engineering hour for a trading platform:

1. **Best-of-N with a robust selector** (multi-seed, purged-CV, top-k submission) — biggest, cheapest, most transferable win.
2. **Better operators** (targeted block edits, complexity control, scoped memory) — unlocks everything else.
3. **Parallel MCTS with UCT** — mainly buys throughput and branch diversity.
4. **Evolutionary search with novelty filtering** — best fit for factor/feature search where the object under search is a short symbolic expression.
5. Exploration-constant tuning — near-zero value; don't spend time here.

---

## 3. Tool / API design for agents

### 3.1 Granularity: the resolved debate

Anthropic's *Writing effective tools for agents* is explicit: **do not mirror your API endpoints**. Consolidate operations that are always called in sequence into one tool that returns the composite result (`get_customer_context` instead of three list calls). OpenAI's function-calling guide says the same two things: *"combine functions that are always called in sequence"* and *"don't make the model fill arguments you already know."*

The synthesis for an ML-experiment platform:
- **Coarse at the workflow boundary, granular at the decision boundary.** `run_experiment(spec)` is one tool, not five, because the agent never wants to call `allocate_gpu` alone. But `propose_feature`, `backtest`, and `promote_to_paper` are separate because each is a genuine decision point with different risk.
- **Target ≈15–25 tools visible at once.** OpenAI recommends **<20 functions available at the start of a turn**. Beyond that, use progressive disclosure.

### 3.2 Progressive disclosure / tool search (the 2026 standard)

Anthropic's advanced tool use (Nov 2025) gives hard numbers:
- **Tool Search Tool**: ~77K tokens of upfront definitions → **~8.7K** (85% reduction). MCP eval accuracy: Opus 4 **49% → 74%**; Opus 4.5 **79.5% → 88.1%**. Worth it above ~10K tokens of tool definitions or ≥10 tools.
- **Programmatic Tool Calling** (agent writes code that orchestrates tools; intermediate results never enter context): **37% token reduction** on research tasks (43,588 → 27,297); GAIA 46.5% → 51.2%. Use when ≥3 dependent calls, or when only aggregates matter — *exactly* the shape of "run 200 backtests and tell me the top 5."
- **Tool Use Examples** (few-shot invocations in the tool def): complex-parameter accuracy **72% → 90%**.

**Recommendation:** expose a small always-on core (≤12 tools) + a searchable long tail, and make the heavy fan-out operations (sweeps, scans, batch backtests) *programmatic* so the 10,000 rows of results are reduced in a sandbox before hitting context.

### 3.3 Schema, errors, and recoverability

Concrete, evidence-backed rules:

- **Strict/structured schemas + enums.** OpenAI: *"use enums and object structure to prevent invalid states."* Never a free-form string where a 6-value enum will do.
- **Natural-language identifiers over UUIDs.** Anthropic: agents handle names/slugs materially better than cryptic IDs. Use `experiment_id="momo-vol-adj-v3"`, not a UUID.
- **`response_format: "concise" | "detailed"`** as a standard parameter — ~67% token savings in Anthropic's tested case. Concise returns human-readable summaries; detailed adds the IDs needed for downstream calls.
- **Errors must be actionable and must teach the fix.** Bad: `ValidationError: invalid parameter`. Good: `Invalid 'start_date': got "2024-13-01". Expected ISO-8601 date within the loaded universe range 2010-01-04..2026-08-31. Nearest valid: "2024-12-01".` Anthropic's guidance and the AIRS-Bench failure analysis both point at formatting/validity failures as a top-line loss.
- **Truncation must be self-describing and steerable.** Never silently cut. Return `{"rows": [...], "truncated": true, "total": 41320, "next_cursor": "...", "hint": "Filter by sharpe>1.0 or pass group_by='sector' to reduce."}`.
- **Idempotency keys on everything that spends money.** `run_experiment(..., idempotency_key: str)` — a retried tool call after a timeout must not launch a second $400 job. This is the most common real-world money leak.
- **Dry-run by default for expensive/irreversible ops.** `backtest(..., dry_run: bool = true)` returns the resolved config, estimated cost, estimated wall-clock, and data-coverage check without executing. Agents that can cheaply preview stop guessing.
- **Return structured diffs, not whole artifacts.** When an agent edits a pipeline, return a unified diff + a semantic summary of what changed (`{"changed_blocks": ["feature_engineering"], "lines_added": 14, "lines_removed": 3}`). This pairs with MLE-STAR's block-targeted refinement and makes human review tractable.
- **Namespacing.** `exp_*`, `data_*`, `bt_*`, `mem_*`, `gov_*` prefixes. Anthropic explicitly recommends this to delineate boundaries when tool counts grow.

---

## 4. Agent memory architectures

### 4.1 Taxonomy and what the 2026 evidence says

*Anatomy of Agentic Memory* (2602.19320) organizes memory-augmented generation into four families: lightweight semantic (vector top-k), entity-centric/personalized (schema'd records), episodic+reflective (temporal buffers + consolidation), and structured/hierarchical (graph or multi-tier). Its empirical findings are sobering:

- **Benchmark underscaling.** Many memory benchmarks (HotpotQA, MemBench, much of LoCoMo) fit inside a 128K context, so "memory" is measured where memory isn't needed. They propose a **Context Saturation Gap** metric to find genuinely memory-demanding tasks.
- **Metric misalignment.** Lexical F1 penalizes abstractive memory systems for paraphrase; it diverges from semantic quality.
- **Backbone sensitivity / silent failure.** Open-weight backbones hit **30.4%** format-error rates on graph memory operations vs 17.9% — the agent stays fluent while the memory writes fail.
- **Maintenance cost is routinely ignored.** MemoryOS >32s/turn vs SimpleMem <1.1s.

The 2026 benchmark guide (Mem0) reinforces: reported LoCoMo/LongMemEval numbers are **not comparable across papers** (different judge models and post-processing; an independent test scored Zep at 75.1% vs its claimed 94.7%), and scores are almost always reported without token cost (Mem0 ~6.7–7.0K tokens/retrieval vs 25K+ for full context). **BEAM** (2026, 1M/10M-token contexts, 10 capabilities) is the current unsaturated target.

### 4.2 What designs actually improve long-horizon performance

**Works (replicated across independent systems):**

1. **Dual-process episodic + consolidated semantic.** A fixed raw recent buffer (e.g. last N interactions, verbatim) + a growing consolidated natural-language profile. The long-horizon scientific-agent paper (2605.17625) reports 100% accuracy at 100,000 messages with constant latency where full-context crashed at ~10,000; 62% fewer tokens; 70–85% accuracy with 1–2s latency in realistic runs. **Crucially: RAG scored 75–80% on historical retrieval but 0% on "what is the current state" queries.** You need both; they are complementary, not substitutes.
2. **Scoped injection, hard token cap.** ML-Master 2.0 caps distilled memory at 4,096 tokens; AIRA scopes by tree relationship (sibling vs ancestor). Bounded, *typed* memory beats bigger memory.
3. **Tiered timescales.** L1 execution feedback (within run) → L2 tactical revisions (within project) → L3 strategic heuristics (across projects). This is the single most portable idea from ML-Master 2.0.
4. **Conflict resolution with temporal precedence.** Consolidation must overwrite stale facts, not append contradictions. This is the direct fix for stale beliefs.
5. **Raw trajectory retrieval is a strong baseline.** See below — often stronger than distilled skills.

**Doesn't work as advertised:**

- **Skill libraries (Voyager-style) largely fail to transfer.** SkillEvolBench (2605.24117), 180 tasks / 6 environments / 10 model configs: *"Raw-trajectory reuse frequently outperforms distilled skills."* Static curated skills **underperformed the no-skill baseline by −2.44 pts** on average. Larger libraries sometimes *hurt* (skill bloat encoding episode-specific assumptions). Multi-skill composition hit **0%** in some environments. Selective procedural abstraction is unsolved.
- **Naive summarization memory.** AIRA found AIDE's memory operator alone "didn't substantially improve" performance.
- **Consolidation model scaling.** Upgrading the consolidator from GPT-4o-mini to GPT-4o improved accuracy by **0.07%** — consolidation quality is prompt/schema-bound, not capability-bound. Don't spend on a bigger consolidator; spend on a better extraction schema.

### 4.3 Failure modes

- **Memory poisoning.** Systematic study (2606.04329): **66.7% attack success rate** on an aggressive-write agent framework vs 34.3% on a conservative one; weak-signal *fact injection* alone reached **64.5% ASR**. Root causes are architectural: low summarization thresholds, auto-injecting memory snapshots into every system prompt, and **no trust distinction between sources**. Existing prompt-injection defenses are the wrong layer — PromptArmor caught 84.4% of strong-signal but only **42.5%** of weak-signal attacks, because defenses guard the *input boundary* while poisoning happens on the **write path**. For a trading platform the analogue is non-adversarial but identical in mechanism: a bad backtest result or a hallucinated "insight" gets written to L3 and steers every future experiment.
- **Stale beliefs.** "Feature X doesn't work" written in a 2023 regime, still suppressing that branch in 2026. Mitigation: every memory item carries `valid_as_of`, `regime_tags`, and an explicit `evidence_ids` list; L3 heuristics expire (TTL) unless re-confirmed.
- **Overfitting to past cases.** Case-based retrieval biases the agent toward previously-successful families → the search collapses to one region of hypothesis space. Mitigation: novelty-rejection filtering (ShinkaEvolve) and explicit diversity quotas in parent sampling.
- **Silent write failures.** 30% schema-error rates on structured memory writes. Validate every write, and alert on write-failure rate as a first-class metric.

### 4.4 Storage substrate

There is no evidence that graph memory beats vector or relational on *outcome*, and clear evidence it costs more (latency, schema error rate). For experiment outcomes specifically the data is **inherently relational and numeric** — config, metrics, artifacts, lineage. Recommendation:

- **Relational (Postgres) as the system of record** for experiments, runs, metrics, datasets, lineage. This is what you query with "show me every run where Sharpe > 1.2 and max_dd < 8% on 2019–2021."
- **Vector index over *natural-language insight records*** (hypothesis, what changed, what happened, why) for associative recall.
- **A small explicit graph only for lineage/derivation edges** (which node came from which, which factor is an AST-descendant of which) — and derive it from the relational store rather than maintaining a separate graph DB.
- Do not put numeric experiment results into a vector store. Embedding similarity over metrics is noise.

---

## 5. Multi-agent orchestration

### 5.1 What the evidence says

**For multi-agent:** Anthropic's production research system (orchestrator + parallel subagents, Claude Opus 4 lead / Sonnet 4 workers) reported **+90.2%** over single-agent Opus 4 on internal research evals, with parallelization cutting research time up to 90%. But: the system uses **~15× the tokens** of chat (single agent ≈ 4×), and **token usage alone explained 80% of performance variance**. That is the crux — much of the "multi-agent win" is a compute win.

**Against multi-agent:** Under **equal thinking-token budgets**, single-agent matched or beat every MAS variant (sequential, subtask-parallel, parallel-roles, debate, ensemble) across Qwen3/DeepSeek-R1/Gemini 2.5 on FRAMES and MuSiQue (2604.02460). MAS only overtook SAS under **heavy context degradation (70% noise)**. Conclusion from the authors: *"many reported MAS gains are better explained by compute and context effects rather than by inherent architectural superiority."*

**MAST** (2503.13657): 1,600+ annotated traces from 7 frameworks, 14 failure modes in 3 categories — *system design* (bad role specification, disobeying task spec), *inter-agent misalignment* (information withholding, derailment, ignored input), *task verification* (premature termination, absent/incorrect verification). κ=0.88 inter-annotator. The dominant fixable category is verification.

### 5.2 Synthesis: when to go multi-agent

Use multiple agents when **all three** hold:
1. The work is genuinely **parallel** (independent branches, no shared mutable state),
2. The combined information **exceeds one context window**, and
3. Each branch produces a **compressible summary** (subagent returns a report, not its transcript).

Do **not** use multiple agents for: sequential dependent reasoning, shared-context refactors, or anything where you'd be paying 15× tokens for what a single agent with a bigger thinking budget would do.

### 5.3 Recommended topology for an experiment platform

```
Research Director (long-lived, owns hypothesis backlog + L3 memory + budget)
  ├── Experiment Orchestrator (owns one solution tree; runs MCTS; allocates node budget)
  │     ├── Node Worker ×K  (isolated context; one operator application: draft/improve/debug/crossover)
  │     └── Evaluation Service (NOT an agent: deterministic purged-CV, cost model, leakage checks)
  ├── Critic / Red-team agent (independent backbone; sees artifacts as EXTERNAL content)
  └── Governance layer (NOT an agent: budget enforcement, approval gates, audit trail)
```

Key protocol rules:
- **Context isolation:** node workers never see sibling transcripts, only *scoped memory digests* (≤4K tokens, sibling-for-improve / ancestor-for-debug).
- **Handoff = typed artifact, not transcript.** A worker returns `{node_id, diff, metrics, cost, logs_ref, failure_class}`. The orchestrator never ingests raw logs.
- **Blackboard where it earns its keep:** a shared, append-only `experiment_ledger` (relational) that everyone reads and only the evaluation service writes. This gets you blackboard coordination without free-form inter-agent chatter, which is where MAST's misalignment failures live.
- **Heterogeneous critic.** Use a *different model family* for the critic than for generation — homogeneous debate collapses ("debate diversity collapse"; 2502.08788 makes the same argument for heterogeneity).
- **Deterministic verification is not an agent.** MAST's largest fixable failure category is verification; the fix is to make verification code, not conversation.

---

## 6. Governance & safety for agents spending money/compute

### 6.1 The "400 useless runs" failure mode is now measured

**BAGEN** (2606.00198): across 4 environments and 5 frontier agents, task performance and budget-awareness are **nearly uncorrelated (r = 0.35)**. Frontier models are consistently **over-optimistic**: they "continue spending on tasks that are unlikely to succeed, instead of alerting the user early." Budget-interval calibration coverage stayed **below 47%** even after targeted training. The actionable part: **SFT+RL for early stopping saved 28–64% of tokens on failed trajectories**. The paper's framing is the right one — *budget should be an active control signal in the loop, not a post-hoc metric.*

Corroborating: MLE-bench observed agents "rarely verbalized any consideration of how long their produced code would run."

**Implication: do not expect the agent to manage its own budget. Enforce it in the harness.**

### 6.2 Permission models — and approval fatigue

The Claude Code design-space analysis (2604.14228) is the best-documented production permission system:
- **Seven graduated trust modes**, deny-first evaluation, an ML-based auto-mode classifier, seven independent safety layers.
- **Reasoning is separated from enforcement**: the model *proposes* tool_use; the harness *enforces* permissions, sandboxing, execution. The model is never the security boundary.
- **98.4% of code is operational infrastructure**, not reasoning logic.
- Critically: **users approve ~93% of permission prompts** → approval fatigue is real and measured. Auto-approval rates climb from ~20% in early sessions to 40%+ past 750 sessions.

**Design consequence:** do not multiply per-action prompts. Establish a **sandboxed envelope** inside which the agent is free, and gate only the small set of boundary-crossing actions (spend above threshold, promote to paper/live trading, write to L3 memory, touch production data).

### 6.3 Concrete control patterns to implement

1. **Hierarchical budgets with hard preemption.** Per-run, per-node, per-tree, per-day, per-hypothesis. Enforced by the scheduler, checked *before* dispatch, not by the agent. Refuse-and-explain rather than silently kill: return a structured `BudgetExceeded` with remaining allowance and the cheapest viable alternative config — the agent can then replan.
2. **Marginal-value stopping rule.** Track score-improvement-per-dollar over a sliding window of nodes; auto-halt a subtree when the posterior probability of improving the incumbent by >ε falls below a threshold. This is the direct fix for 400 useless runs, and it's the same math as the PRM pruning in §7.
3. **Cost estimation is a tool, and dry-run is free.** Every expensive tool exposes `estimate_cost(spec) -> {usd, gpu_minutes, wall_clock, data_bytes}` and `dry_run=true`.
4. **Idempotency + deduplication at the scheduler.** Hash the resolved config; if an identical (config, data-version, seed) has run, return the cached result and charge $0. In practice this eliminates a surprising fraction of agent spend.
5. **Reversibility by default.** Every run writes to an immutable, content-addressed artifact store; the experiment ledger is append-only; "undo" = repoint a pointer, never delete. Snapshot before any mutation of shared state (universe definitions, feature registry).
6. **Audit trail as a first-class, tamper-evident structure.** The IETF `draft-sharif-agent-audit-trail` is a usable schema even if it never becomes an RFC: per-record `record_id`, `agent_id`, `session_id`, `agent_version`, RFC-3339 `timestamp`, `record_phase` (**pre**/post/concurrent — so you can prove a *denial* prevented an action rather than logging it after), `action_type`, `outcome`, `parent_record_id`, `prev_hash` (SHA-256 chain), `trust_level` L0–L4, plus optional cost estimate and risk score. At trust level L2+, an **independent component writes the records** so the agent cannot suppress its own trail. Designed against EU AI Act Art. 12 (in force Aug 2026). For a trading platform this doubles as your regulatory research-provenance record.
7. **Capability scoping by phase.** Research agents get read-only market data + sandboxed compute. Only a separately-credentialed promotion path can touch paper/live. No agent ever holds broker credentials.
8. **Write-path validation for memory** (the poisoning lesson): every memory write is schema-validated, source-tagged (`trusted_eval` / `agent_generated` / `external_doc`), and agent-generated L3 heuristics require corroboration from ≥N independent runs before they're injected into prompts.

---

## 7. Verifiers & reward models for agent decisions

### 7.1 The core problem for quant

Your validation metric **is** your reward model, and it is a **weak, gameable verifier**. AIRA quantified the cost of trusting it: 9–13 medal points lost to validation-vs-test divergence on Kaggle, where the test set is at least drawn from the same distribution. In trading, the divergence is worse (non-stationarity, regime shift) and the search performs thousands of implicit hypothesis tests.

### 7.2 Process reward models (PRM) vs outcome reward models (ORM)

**DataPRM** (2604.24198) is the closest published analogue to what you need — a PRM for *agentic data analysis*:
- **Environment-aware and generative** (ReAct-style): it can *probe intermediate execution state*, not just read code. This is why it beats LLM-as-judge — it verifies execution, it doesn't inspect statically.
- **Ternary rather than binary reward**: 1.0 strictly correct / **0.5 correctable error** (syntax, wrong column name — recoverable, shouldn't be punished) / 0.0 irrecoverable (logic flaw, hallucination, dead end). The 0.5 class exists precisely because binary PRMs punish healthy exploratory grounding errors while missing **silent errors** (code runs, answer is wrong).
- Trained on ~7,000 step-level annotated instances via diversity-driven trajectory sampling.
- **At 4B params it beats Qwen2.5-Math-PRM-72B and GenPRM-32B** on ScienceAgentBench and DABStep; +7.21% / +11.28% downstream in Best-of-N; beats self-rewarding with a 235B model.

Takeaway: **a small, domain-specific, environment-grounded PRM outperforms a huge generic one and outperforms self-critique.** This is very buildable — you already generate the training data as a side-effect of running experiments.

Related: **SWE-TRACE** (2604.14820) uses rubric-based PRMs + heuristic test-time scaling for long-horizon SWE agents; **AgentPRM** (WWW'26) scores steps by "promise and progress" — i.e. a step is good if it increases the probability of eventual success, which is the right credit-assignment framing for experiment trees.

### 7.3 LLM-as-judge reliability

*Reliability without Validity* (2606.19544), 21 judges, 9 providers, 541,000 judgments:
- **Kappa deflation:** exact-match agreement **overstates chance-corrected agreement by 33.8–41.2 points**. An 85%-agreement judge is ≈ 0.48 Cohen's κ — moderate at best.
- **Rankings don't transfer:** >half of models move ≥4 positions across benchmarks, some 15. JudgeBench discriminates 4.5× more sharply than MT-Bench.
- **Consistency ≠ validity:** test-retest >0.95 coexists with position bias >0.10. Reproducibly wrong.
- Verbosity bias was *not* reproduced (<0.011 correlation) — so the classic "judges prefer long answers" worry is weaker than believed.

Anthropic's eval guidance adds: grade **transcripts and outcomes** (agents claim success they didn't achieve); calibrate model graders against human experts; give judges an explicit "Unknown" out; don't grade *steps* too rigidly because agents find valid unanticipated paths; watch for shared-state leakage between trials.

**Practical rule: never let an LLM judge decide whether a trading strategy is good. LLM judges are for triage — "is this hypothesis novel / is this code plausibly leakage-free / is this failure recoverable" — and every judge gets a periodic human-calibration sample reported in κ, not raw agreement.**

### 7.4 Limits of verifier-guided scaling

Best-of-N scaling is bounded by verifier quality; with imperfect verifiers, more samples eventually *decreases* true quality because you're selecting on verifier noise. "The Verification Horizon" makes this point for coding agents. In finance this manifests as classic backtest overfitting: the more strategies you search, the higher the maximum in-sample Sharpe purely by chance.

**Mandatory countermeasures for a quant platform:**
- **Purged, embargoed walk-forward CV** as the *only* selection signal the search sees.
- **A truly held-out final period the agent can never query** — enforce at the data-access layer, not by prompt. The agent's data tool must physically refuse dates past the embargo.
- **Deflated Sharpe / multiple-testing adjustment** computed from the *actual number of nodes evaluated in the tree*. Your platform knows this number exactly — use it. Report DSR alongside every raw Sharpe.
- **Submit/promote top-k, not top-1** (AIRA: ~10% recovered with k=3).
- **Novelty filtering before evaluation** to reduce the effective number of trials.
- **Regime-stratified scoring** so a strategy that only works in one regime can't win on pooled metrics.

---

## 8. Architectural recommendations (concrete)

1. **Core loop = parallel MCTS over a solution tree**, nodes = complete pipeline versions, edges = typed operators. UCT selection; don't over-tune the exploration constant. Budget nodes, not wall-clock.
2. **Operator set:** `draft`, `improve`, `debug`, `crossover`, `ablate`, `targeted_block_edit`. Add prompt-adaptive complexity cues conditioned on sibling count. Invest here first — AIRA proves search gains are gated on operator quality.
3. **Evaluation is deterministic code, never an agent.** Purged walk-forward CV, leakage checks, cost accounting, DSR. Returns a typed record.
4. **Selection is the highest-leverage subsystem.** Multi-seed, top-k promotion, DSR-adjusted, regime-stratified. Treat "which node do we submit" as a first-class research problem — it's worth 9–13 points in the literature and more in trading.
5. **Memory = 3 tiers + relational ledger.** L1 run feedback (ephemeral), L2 tactical revisions (per-tree, ≤4K tokens injected), L3 cross-project heuristics (corroboration-gated, TTL'd, regime-tagged). System of record is Postgres; vector index only over natural-language insight records. **Skip the skill library** — SkillEvolBench shows raw-trajectory retrieval beats distilled skills; revisit only if you solve selective abstraction.
6. **Critic on a different model family, fed artifacts as external content** (tool/memory role, never `<thought>`) — 23–93 point correction-rate swing for free.
7. **Train a small domain PRM** on your own accumulated step annotations, ternary-labeled, environment-grounded. Use it to prune subtrees early and for Best-of-N. DataPRM shows 4B is enough.
8. **Budget is harness-enforced**, with marginal-value stopping, idempotent dedup, dry-run/estimate on every expensive tool, and refuse-with-alternatives errors.
9. **Sandboxed envelope + few gates.** ~93% prompt-approval means fine-grained prompting is theater. Gate: spend above threshold, promotion to paper/live, L3 memory writes, embargoed-data access.
10. **Tamper-evident, hash-chained audit trail** written by an independent component, with `record_phase: pre` for denials.
11. **Tool surface ≤ ~15 always-on + searchable long tail**, with programmatic tool calling for fan-out operations.
12. **Expect ~15× token cost if you parallelize.** Budget for it; 80% of the multi-agent "win" is compute.

---

## 9. Proposed tool surface (typed signatures)

Namespaced, granular at decision boundaries, coarse at workflow boundaries. `~15` always-on (marked ★), rest behind tool search.

### Data & universe
```ts
data_list_datasets(filter?: {asset_class?: Enum, freq?: Enum, tag?: string[]},
                   response_format?: "concise"|"detailed") -> DatasetSummary[]   // paginated ★

data_describe(dataset_id: str) -> {schema: ColumnSpec[], date_range: [Date,Date],
                                   n_rows: int, known_issues: str[], survivorship_note: str}

data_load_slice(dataset_id: str, start: Date, end: Date, columns: str[],
                universe: str, max_rows?: int = 100_000)
    -> {handle: DataHandle, rows_returned: int, truncated: bool, hint?: str}
    // HARD FAIL if [start,end] intersects the embargoed holdout. Error names the legal max date.

data_point_in_time_check(dataset_id: str, columns: str[])
    -> {lookahead_risk: Enum<"none"|"suspected"|"confirmed">, offending_columns: str[], rationale: str}
```

### Feature / signal construction
```ts
feat_register(name: str, expression: str /*symbolic DSL or python*/, rationale: str,
              hypothesis_id: str, dry_run: bool = true)
    -> {feature_id: str, ast_hash: str, complexity: {depth:int, n_ops:int, n_params:int},
        nearest_existing: {feature_id: str, similarity: float}[],   // AST + embedding
        rejected_as_duplicate: bool, estimated_compute: CostEstimate}

feat_ablate(pipeline_id: str, blocks: str[], cv: CVSpec)
    -> {block_contributions: {block: str, delta_metric: float, ci95: [float,float]}[]}
    // MLE-STAR-style: tells the agent WHICH block to edit next
```

### Experiment lifecycle  ★ (the core five)
```ts
exp_propose(hypothesis: str, parent_node_id?: str, operator: Enum<"draft"|"improve"|"debug"|"crossover"|"targeted_edit">,
            spec: PipelineSpec, complexity_hint?: Enum<"minimal"|"moderate"|"advanced">)
    -> {node_id: str, diff: UnifiedDiff, semantic_diff: {changed_blocks: str[], added: int, removed: int},
        novelty: {is_duplicate: bool, nearest: str, similarity: float},
        cost_estimate: CostEstimate, budget_remaining: BudgetView}          ★

exp_run(node_id: str, seeds: int[] = [0,1,2], cv: CVSpec, idempotency_key: str,
        max_usd: float, max_wall_clock_s: int, dry_run: bool = false)
    -> {run_id: str, status: Enum<"queued"|"running"|"done"|"failed"|"preempted">,
        cached: bool, actual_cost?: CostEstimate}                            ★

exp_result(run_id: str, response_format?: "concise"|"detailed")
    -> {metrics: {ic: float, icir: float, sharpe: float, dsr: float, max_dd: float,
                  turnover: float, capacity_usd: float},
        per_regime: {regime: str, sharpe: float, n_days: int}[],
        per_fold: FoldMetric[], leakage_report: LeakageReport,
        failure_class?: Enum<"syntax"|"runtime"|"oom"|"timeout"|"no_signal"|"leakage"|"invalid_output">,
        logs_ref: ArtifactRef, log_tail: str /*bounded, self-describing truncation*/}   ★

exp_compare(node_ids: str[], metric: str, correction: Enum<"none"|"deflated_sharpe"|"bh_fdr">)
    -> {ranking: {node_id: str, point: float, ci95: [float,float], adjusted: float}[],
        n_trials_in_family: int, note: str}                                  ★

exp_tree(root_id: str, depth?: int = 3, response_format?: "concise"|"detailed")
    -> {nodes: {node_id, parent, operator, status, score, cost, is_incumbent}[],
        frontier: str[], total_spend: CostEstimate}                          ★
```

### Backtest & risk (separate from exp_* because different risk class)
```ts
bt_run(strategy_spec: StrategySpec, period: [Date,Date], costs: CostModel,
       idempotency_key: str, dry_run: bool = true)
    -> {backtest_id: str, estimated_cost: CostEstimate} | BacktestResult

bt_stress(backtest_id: str, scenarios: Enum<"2008"|"2020-03"|"2022-rates"|"custom">[])
    -> {scenario: str, dd: float, recovery_days: int}[]

bt_capacity(backtest_id: str, adv_participation: float) -> {capacity_usd: float, decay_curve: Point[]}
```

### Memory ★
```ts
mem_write(tier: Enum<"L1"|"L2"|"L3">, kind: Enum<"observation"|"tactic"|"heuristic"|"negative_result">,
          content: str, evidence_run_ids: str[], regime_tags: str[],
          valid_as_of: Date, ttl_days?: int, source: Enum<"trusted_eval"|"agent_generated"|"external_doc">)
    -> {memory_id: str, accepted: bool, reason?: str, corroboration_count: int}   ★
    // L3 writes REQUIRE >= N independent evidence_run_ids; schema-validated; write failures surface loudly

mem_query(query: str, tier?: Tier[], scope?: Enum<"siblings"|"ancestors"|"global">,
          node_id?: str, regime_filter?: str[], k: int = 8, max_tokens: int = 4096)
    -> {items: MemoryItem[], token_count: int, stale_filtered: int}                ★

mem_query_outcomes(filter: {metric: str, op: Enum<">"|"<">, value: float,
                            date_range?: [Date,Date], feature_family?: str}, limit: int = 50)
    -> ExperimentRecord[]    // relational, NOT vector — this is the ledger query ★

mem_invalidate(memory_id: str, reason: str, superseded_by?: str) -> {ok: bool}
```

### Critique & verification
```ts
critic_review(artifact: {kind: Enum<"diff"|"result"|"hypothesis">, ref: str},
              checklist: Enum<"leakage"|"overfit"|"novelty"|"implementation"|"all">)
    -> {findings: {severity: Enum<"blocker"|"major"|"minor">, claim: str,
                   evidence: str, suggested_fix: str}[], confidence: float, abstained: bool}
    // runs on a DIFFERENT backbone; artifact is presented as external tool content

verify_prm_score(node_id: str, step_range?: [int,int])
    -> {step_scores: {step: int, reward: 0.0|0.5|1.0, label: str}[],
        trajectory_score: float, recommend: Enum<"continue"|"debug"|"abandon_subtree">}
```

### Governance ★
```ts
gov_budget(scope: Enum<"run"|"node"|"tree"|"day"|"hypothesis">, id?: str)
    -> {spent_usd: float, limit_usd: float, gpu_minutes_left: int,
        projected_exhaustion: Timestamp, marginal_value_per_usd: float}            ★

gov_estimate(action: ToolCallSpec) -> CostEstimate                                 ★

gov_request_approval(action: Enum<"exceed_budget"|"promote_to_paper"|"promote_to_live"|
                                  "access_embargoed_data"|"write_L3_memory">,
                     justification: str, evidence_refs: str[], requested_scope: Scope)
    -> {approval_id: str, status: Enum<"pending"|"granted"|"denied">, expires_at?: Timestamp}

gov_audit_append(action_type: str, action_detail: json, phase: Enum<"pre"|"post">,
                 parent_record_id?: str) -> {record_id: str, prev_hash: str}
    // normally written by the harness, not the agent; exposed for agent-authored rationale records

gov_checkpoint(label: str) -> {checkpoint_id: str}
gov_rollback(checkpoint_id: str, dry_run: bool = true) -> {will_revert: ChangeSummary[]}
```

### Meta
```ts
tool_search(query: str, k: int = 5) -> ToolDefinition[]      ★ // progressive disclosure
run_code(code: str, timeout_s: int, allow_net: false)        ★ // programmatic tool calling sandbox
    -> {stdout: str /*truncated w/ hint*/, artifacts: ArtifactRef[], error?: StructuredError}
```

**Cross-cutting conventions:**
`response_format: "concise"|"detailed"` on every read tool · `dry_run: bool` on every tool that spends · `idempotency_key` on every tool that mutates or spends · every error is `{code, message, expected, received, nearest_valid, suggested_fix}` · every truncation returns `{truncated, total, next_cursor, hint}` · IDs are human-readable slugs, never UUIDs · namespaces `data_ feat_ exp_ bt_ mem_ critic_ verify_ gov_`.

---

## 10. Open problems worth tracking

- **Selective procedural abstraction** — nobody can reliably turn trajectories into transferable skills (SkillEvolBench).
- **Robust final-node selection** — AIRA explicitly names this as the highest-value open problem; for trading it's the whole problem.
- **Budget calibration** — <47% interval coverage even after training (BAGEN).
- **Memory write-path security** — input-boundary defenses catch only 42.5% of weak-signal poisoning.
- **Verifier quality ceilings on Best-of-N** — more samples past a point selects verifier noise.
- **Long-horizon human baselines** — METR has human times for only 5 of 31 8h+ tasks; the long end of the capability curve is weakly measured.

---

## Sources

**Benchmarks & ML-engineering agents**
- MLE-bench (OpenAI) — https://arxiv.org/abs/2410.07095 · https://arxiv.org/html/2410.07095v5 · https://openai.com/index/mle-bench/
- AI Research Agents for ML: Search, Exploration, Generalization in MLE-bench (AIRA-dojo, Meta) — https://arxiv.org/abs/2507.02554 · https://arxiv.org/html/2507.02554v1 · https://neurips.cc/virtual/2025/poster/117980 · https://github.com/facebookresearch/aira-dojo
- ML-Master v1 — https://arxiv.org/html/2506.16499v1 · https://github.com/sjtu-sai-agents/ML-Master
- ML-Master 2.0 — https://sjtu-sai-agents.github.io/ML-Master/ · https://www.eigenai.com/blog/2025-12-28-ml-master-2-0
- MLE-STAR (Google) — https://research.google/blog/mle-star-a-state-of-the-art-machine-learning-engineering-agents/ · https://arxiv.org/abs/2506.15692
- AIRS-Bench — https://arxiv.org/pdf/2602.06855
- MLGym — https://arxiv.org/pdf/2502.14499 · MLE-Dojo — https://openreview.net/forum?id=5W5mFU4oMO
- RE-Bench (METR) — https://metr.org/AI_R_D_Evaluation_Report.pdf · https://metr.org/blog/2024-11-22-evaluating-r-d-capabilities-of-llms/
- METR Time Horizons 1.1 — https://metr.org/blog/2026-1-29-time-horizon-1-1/ · https://metr.org/time-horizons/
- PaperBench — https://arxiv.org/abs/2504.01848 · https://openai.com/index/paperbench/ · leaderboard (unverified aggregator) https://benchmarklist.com/benchmarks/openai_paperbench/
- AI Scientist-v2 (Sakana) — https://arxiv.org/abs/2504.08066
- Google AI co-scientist — https://research.google/blog/accelerating-scientific-breakthroughs-with-an-ai-co-scientist/ · https://storage.googleapis.com/coscientist_paper/ai_coscientist.pdf

**Search & program evolution**
- ShinkaEvolve — https://sakana.ai/shinka-evolve/ · https://arxiv.org/abs/2509.19349 · https://iclr.cc/virtual/2026/poster/10007692
- CodeEvolve — https://arxiv.org/html/2510.14150v1
- Self-Correction Illusion — https://awesomepapers.io/ai-agents/papers/2606.05976
- When/Why Multi-Agent Debate Fails — https://arxiv.org/abs/2510.20963 · Stop Overvaluing Multi-Agent Debate — https://arxiv.org/abs/2502.08788

**Tool / API design**
- Anthropic, Advanced tool use (Tool Search, Programmatic Tool Calling, Tool Use Examples) — https://www.anthropic.com/engineering/advanced-tool-use
- Anthropic, Writing effective tools for agents — https://www.anthropic.com/engineering/writing-tools-for-agents · mirror https://modelcontextprotocol.info/docs/tutorials/writing-effective-tools/
- Anthropic, Demystifying evals for AI agents — https://anthropic.com/engineering/demystifying-evals-for-ai-agents
- OpenAI, Function calling guide — https://developers.openai.com/api/docs/guides/function-calling
- Anthropic, Define tools — https://platform.claude.com/docs/en/agents-and-tools/tool-use/define-tools

**Memory**
- Anatomy of Agentic Memory (taxonomy + empirical limits) — https://arxiv.org/html/2602.19320v1
- Memory for Autonomous LLM Agents: Mechanisms, Evaluation, Frontiers — https://arxiv.org/html/2603.07670v1
- Episodic-Semantic Memory for Long-Horizon Scientific Agents — https://arxiv.org/html/2605.17625v1
- Multi-Layered Memory Architectures: Experimental Evaluation — https://arxiv.org/html/2603.29194v1
- SkillEvolBench — https://arxiv.org/html/2605.24117v1
- Agent Skills for LLMs: Architecture, Acquisition, Security — https://arxiv.org/html/2602.12430v3
- Experience Memory for Sequential Decision-Making — https://arxiv.org/html/2608.03420
- Voyager (skill library origin) — https://arxiv.org/html/2305.16291
- Memory benchmarks 2026 (LoCoMo / LongMemEval / BEAM, with caveats) — https://mem0.ai/blog/ai-memory-benchmarks-in-2026 · https://mem0.ai/blog/state-of-ai-agent-memory-2026
- Memory poisoning, systematic study — https://www.alphaxiv.org/abs/2606.04329 · https://arxiv.org/html/2601.05504v2 · https://pith.science/paper/2608.21230

**Multi-agent orchestration**
- MAST: Why Do Multi-Agent LLM Systems Fail? — https://arxiv.org/abs/2503.13657 · https://arxiv.org/pdf/2503.13657
- Anthropic multi-agent research system (engineering lessons) — https://www.zenml.io/llmops-database/building-production-multi-agent-research-systems-with-claude
- Single-Agent vs MAS under equal thinking-token budgets — https://arxiv.org/html/2604.02460v1
- Debate diversity collapse — https://tianpan.co/blog/2026/04/26/debate-diversity-collapse-multi-agent-ensemble

**Governance, budget, safety**
- BAGEN: Are LLM Agents Budget-Aware? — https://awesomepapers.io/ai-agents/papers/2606.00198 · https://pith.science/paper/2606.00198
- Dive into Claude Code: Design Space of AI Agent Systems (permissions, sandboxing, context compaction, approval fatigue) — https://arxiv.org/html/2604.14228v2
- IETF draft-sharif-agent-audit-trail — https://datatracker.ietf.org/doc/draft-sharif-agent-audit-trail/
- AI agent security 2026 (guardrails, permissions, sandboxes, MCP threats) — https://slavadubrov.github.io/blog/2026/04/20/ai-agent-security/
- Agent rollback & checkpoint patterns — https://www.digitalapplied.com/blog/agent-rollback-checkpoint-patterns-2026-engineering-reference

**Verifiers & reward models**
- DataPRM: Process-Level Reward Modeling for Agentic Data Analysis — https://arxiv.org/html/2604.24198v1
- SWE-TRACE: Rubric PRMs + heuristic test-time scaling — https://arxiv.org/html/2604.14820v1
- AgentPRM (WWW'26) — https://dl.acm.org/doi/10.1145/3774904.3792551
- Verifiable Process Rewards for Agentic Reasoning — https://arxiv.org/html/2605.10325v1
- Reliability without Validity: large-scale LLM-as-judge evaluation — https://arxiv.org/html/2606.19544v1
- Bias in the Loop: Auditing LLM-as-a-Judge for SE — https://arxiv.org/html/2604.16790v1
- A Survey on LLM-as-a-Judge — https://arxiv.org/html/2411.15594v6
- Multi-Agent Verification: scaling test-time compute with multiple verifiers — https://arxiv.org/pdf/2502.20379

**Quant-specific agentic research**
- QuantaAlpha: Evolutionary LLM-driven alpha mining — https://arxiv.org/html/2602.07085v2
- AlphaAgent: regularized exploration against alpha decay — https://arxiv.org/html/2502.16789v2
- Automate Strategy Finding with LLM in Quant Investment — https://arxiv.org/html/2409.06289v4
- Alpha-R1: alpha screening with LLM reasoning via RL — https://arxiv.org/html/2512.23515
