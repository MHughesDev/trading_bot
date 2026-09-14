# 07 — Fine-Tuning Specialist Models & Drift/Retraining Policy for a Multi-Tenant Quant-ML Platform

**Date of research:** 2026-09-13
**Scope:** (A) Should the platform fine-tune its own small LLMs for its bespoke tool API? (B) How should the platform detect drift and retrain the internal models it uses for its own decision-making, across many tenants, with real capital downstream?

**Bottom line up front (the three decisions the rest of this document defends):**

| Decision | Verdict | Confidence |
|---|---|---|
| (a) Fine-tune an LLM for the toolset **now**? | **No. Instrument for it.** Ship tool search + tool-use examples + prompt optimization (GEPA/MIPRO) + prompt caching + a cascade router first. Build the trace ledger so a fine-tune is a 2-week project when the break-even arrives. Revisit at **~100k executor-calls/day sustained** or when a hard latency/privacy/determinism constraint bites. | High |
| (b) Autonomous vs user-driven retraining of internal models? | **Two-tier.** Infrastructure models (cost, failure, learning-curve, step-critic) retrain **autonomously** behind frozen-holdout + shadow + canary + auto-rollback. Anything whose output steers capital-adjacent choices (gate pre-screener, strategy-family recommender, proposal ranker) retrains on a **human-approved promotion gate**, always. Never let a model that scores experiments be retrained on data generated solely by its own scores without forced exploration and logged propensities. | High |
| (c) Per-tenant vs global internal models? | **Hierarchical, but split by information class.** Platform-physics models → **global** (pool everything). Alpha-adjacent models → **global prior on tenant-agnostic features + per-tenant adaptation**, with a hard rule that no strategy-payload, instrument-level, or outcome-conditional feature crosses a tenant boundary. Skip federated learning and DP for now; they solve a problem you can solve with schema discipline at 1/50th the cost. | High |

---

# PART A — Fine-tuning specialist models for a custom agent platform

## A1. PEFT landscape as of 2026

### A1.1 The LoRA-vs-full-FT question is substantially settled — with conditions

Thinking Machines' **"LoRA Without Regret"** (Sept 2025) is the reference result, and TRL has since shipped a reproduction as first-class documentation, which is a decent proxy for "the field accepted this."

Findings that matter operationally:

- **LoRA matches full fine-tuning** when (i) applied to **all linear layers** — critically including MLP/MoE, not attention-only — and (ii) the adapter has enough capacity for the information in the dataset ("low-regret regime").
- **Attention-only LoRA underperforms** even when rank is raised to match parameter count: rank-256 attention-only lost to rank-128 MLP-only. This kills the common `q_proj,v_proj` default. Use `target_modules="all-linear"`.
- **Optimal LR is ~10× the full-FT LR** (≈15× for short ~100-step runs), and is approximately **rank-independent** because of the 1/r scaling.
- **Rank guidance:** post-training-scale SFT → **r≈256**; RL → **r=1–32** (rank-1 sufficed for RL on Llama-3.1-8B, ~3M trainable params). The asymmetry is informational: supervised learning absorbs ~1 bit/token; policy gradient gives **O(1) bits per episode**. RL simply does not need capacity.
- **LoRA is less tolerant of large batch sizes** than full FT, and *raising rank does not fix it* — it's a property of the product-of-matrices parameterization. Keep effective batch **< 32**.
- **Compute:** ~2/3 the FLOPs of full FT per pass.

**Read-through for this platform:** every fine-tune we would plausibly do is either (a) small-N SFT on curated agent trajectories or (b) GRPO-style RL against our own verifiable gates. Both sit squarely in LoRA's parity regime. **Full fine-tuning is not on the table** and would be a red flag if proposed — it would only win if we were doing large-scale continued pretraining on, say, tens of billions of tokens of market microstructure text, which is not the plan.

### A1.2 Variants: DoRA, LoRA+, ReLoRA, QLoRA, and friends

2026 practitioner consensus (Spheron's PEFT decision guide, corroborated by independent A/Bs):

| Method | Accuracy gap vs full FT | VRAM (7B, r=16, bs2, len512) | Convergence | Verdict for us |
|---|---|---|---|---|
| **LoRA** (all-linear) | −3–5% | ~10 GB | baseline | **Default.** |
| **QLoRA** (4-bit base) | ≈LoRA, small extra loss | ~7 GB (QDoRA) | slower wall-clock | Only if GPU-poor; we are not. |
| **DoRA** | −1–2% | ~11 GB (+5–10%) | similar | Worth an A/B at final-model stage; 1–3% gains reported on commonsense NLU. Independent replications are mixed — at least one 2026 head-to-head found DoRA **slower and no more accurate**, so treat the +1–3% as task-dependent, not free. |
| **PiSSA** | −2–4% | ~10 GB | **30–50% faster** convergence | Attractive if we iterate a lot; use no dropout. |
| **GaLore** | −1–3% | ~18 GB (+30–40%) | 20–40% more steps | Full-param quality on memory-limited nodes. Not our constraint. |
| **VeRA** | −4–6% | lowest | slower | Only for extreme VRAM limits. Skip. |
| **LoRA+** (higher LR on B matrix) | small positive | = LoRA | faster | Cheap to try; subsumed in practice by the 10× LR rule from LoRA-Without-Regret. |
| **ReLoRA** (periodic merge-and-restart) | aimed at *pretraining* | = LoRA | — | Solves "LoRA can't do long high-rank pretraining." Irrelevant to us. |

**Recommendation:** `LoRA, all-linear, r=256 for SFT / r=16 for RL, alpha≈16–32, lr = 10× your full-FT lr (≈1e-5–2e-4 depending on scale), effective batch < 32`. Evaluate DoRA and PiSSA only once a LoRA baseline is beating the prompted frontier model on our private harness. Do not spend a sprint on PEFT-variant shopping; the variance between *data curation* choices dwarfs the variance between these methods.

### A1.3 When full fine-tuning still wins

Narrow list, none of which apply today:
1. **Continued pretraining** on a large new corpus (billions of tokens) — LoRA's capacity ceiling binds.
2. **Very large effective batch training** where LoRA's batch-size penalty costs you throughput you can't recover.
3. **Changing tokenizer/vocabulary or architecture** (e.g., adding numeric/time-series tokenizers for bars) — that's surgery, not adaptation.
4. **You want to ship one merged model and never serve adapters.** You can always merge a LoRA, so this is weak.

### A1.4 Serving many adapters — and what multi-tenancy does to the economics

The stack is mature: **S-LoRA** (unified paging + heterogeneous batching, the original "thousands of adapters on one GPU" result), **vLLM multi-LoRA** (production default; `--enable-lora`, `--max-loras`, `--max-lora-rank`, `--max-cpu-loras`, dynamic load/unload over the REST API, in-place reload), **LoRAX** (Predibase; adapter-aware batching + tiered weight caching), and **SGLang** (weight-load overlap; claims up to 78% TTFT reduction on cold adapters).

Concrete 2026 numbers (8B base, H100 80GB):

- Adapter memory: **r8 ≈ 30 MB, r16 ≈ 60 MB, r32 ≈ 120 MB, r64 ≈ 240 MB**.
- A realistic 100-adapter config: base FP16 ~16 GB + 8 GPU-cached adapters & KV ~4.5 GB + 92 CPU-resident adapters ~6 GB → **~22.5 GB GPU**. 100+ r16 adapters per H100 is routine.
- Swap latency: **sub-ms** from GPU cache, **tens of ms** from CPU RAM, **hundreds of ms** from object storage. Design so the cold path is never on a user-interactive request.
- Cost framing: 100 tenants on 100 dedicated A100s ≈ **$75.6k/mo**; the same 100 as adapters on 1 H100 ≈ **$1.7k/mo** (~$3.5k with HA). ~97–98% reduction.

**The multi-tenancy trap nobody puts in the blog posts:** that 97% saving assumes adapters *share a batch*. Throughput degrades as the number of **distinct adapters concurrently in a batch** rises (each needs its own gather/scatter BGMV/SGMV pass), and vLLM's `--max-loras` caps how many can be co-batched — requests for adapters beyond the cap **queue**. With N tenants and Poisson-ish arrivals, your tail latency is governed by adapter-cache thrash, not by GPU FLOPs.

Consequences for our design:
- **Do not promise per-tenant adapters.** The pooling win (1 GPU vs 100) evaporates into eviction storms exactly when tenants are active simultaneously — i.e., at market open, when crypto vol spikes, when everyone's agents fire at once. Our load is *correlated across tenants* in a way that SaaS chat workloads are not.
- Prefer **one global adapter (or a handful of role adapters: planner/executor/critic)** over per-tenant adapters. Rank-16 role adapters at 60 MB each stay hot forever.
- If per-tenant personalization is ever needed, do it with **retrieval + memory + per-tenant prompt state**, not weights. That is also the only version that survives "tenant deletes their data."
- Keep **rank uniform across adapters**. Heterogeneous ranks fragment the paged adapter pool; see TOPPINGS/S-LoRA work on rank-aware serving for why mixed ranks cost you.

---

## A2. Post-training for agentic / tool-use behavior

### A2.1 What the alphabet soup is actually for

| Family | Needs | Gives | Fit for bespoke tool-calling |
|---|---|---|---|
| **SFT / rejection-sampling FT (RFT, STaR)** | successful trajectories | format + tool-schema conformance, latency | **Highest ROI first step.** Gets you syntax, argument shapes, and house style. |
| **DPO / IPO / cDPO** | pairwise preferences | style, mild behavior shaping | Weak for long-horizon tool use; credit assignment is trajectory-level. |
| **KTO** | *unpaired* binary good/bad | same as DPO without pair construction | Useful because our ledger naturally yields binary outcomes (gate passed/failed) without pairs. |
| **ORPO** | preferences, no ref model | SFT+alignment in one pass | Convenient, cheap; a fine "SFT++" baseline. |
| **SimPO** | preferences, reference-free, length-normalized | removes length bias | Good default over DPO if you go preference-based. |
| **GRPO (+DAPO/Dr.GRPO/GSPO/ARPO)** | a **verifiable reward** + group rollouts | real capability gains on multi-step tasks | **The one that matters** for us — because our gates *are* verifiable. |
| **RLVR** | programmatic verifier | the paradigm above | Our trial ledger is a verifier factory. |
| **RLAIF / LLM-judge reward** | a judge | coverage where verifiers don't exist | Use only with calibration (see A4.3); gameable. |
| **Process supervision (PRMs) / step critics** | step labels | credit assignment on long horizons | Expensive labels, but AgentPRM-class results show 3B+PRM beating GPT-4o on ALFWorld. Relevant to our own "step critic" internal model. |

2026 state of the art per multiple surveys: **GRPO is the critic-free default** (≈50% memory saving vs PPO by dropping the value net), with contested refinements — DAPO (asymmetric clipping, dynamic sampling, token-level normalization; 50 vs 47 AIME on Qwen2.5-32B at **50% fewer steps**), Dr.GRPO (length-bias), GSPO (sequence-level), ARPO (multi-turn agent steps). There is **no consensus on credit-assignment granularity** (token vs sequence vs turn). Evidence is accumulating that **turn-level rewards beat trajectory-level** for multi-turn agents.

**Known failure modes you must budget for:** entropy collapse, advantage collapse, KL drift (the standard k3 KL estimator is unbiased but high-variance and can go negative), gradient conflicts across tasks. Recent practice increasingly **drops the explicit KL term** and relies on clipping.

### A2.2 Entropy/exploration collapse is the specific thing that will bite an experiment platform

"Understanding and Preventing Entropy Collapse in RLVR" (2026) traces collapse to **imbalanced token-level entropy flow**: entropy-decreasing tokens consistently outweigh entropy-increasing ones under GRPO, driving premature determinism. Their fix (OPEFO) rescales updates by entropy contribution while staying strictly on-policy.

This generalizes beyond LLM training and is the single most important cross-cutting risk in this whole document: **a system that trains a policy on its own experiment outcomes will collapse its own exploration unless entropy is explicitly protected.** It applies identically to our experiment-proposal ranker (Part B10).

**Mandatory instrumentation:** log policy entropy (or, for non-LLM rankers, the effective number of distinct strategy families / hyperparameter regions proposed per day) as a **first-class SLO with a floor**. Halt promotion if entropy drops below floor, regardless of how good the reward looks.

### A2.3 Frameworks

| Framework | Use it for |
|---|---|
| **TRL** (HF) | SFT, DPO/KTO/ORPO/SimPO, GRPO; single-node; now ships the LoRA-Without-Regret recipe. **Start here.** |
| **Unsloth** | Fastest single-GPU LoRA/QLoRA, lowest VRAM; great for iteration, weaker multi-GPU. |
| **Axolotl** | YAML-declarative config, good multi-GPU/FSDP, broad recipe library. Good "productionize the recipe" layer. |
| **verl** (ByteDance) | Production-grade large-scale RL; vLLM/Megatron backends; trillion-param MoE tested. Overkill until we are >8B. |
| **OpenRLHF** | Ray-based, explicit **async agentic RL** support. The right shape for long, variable-length tool trajectories. |
| **SkyRL** (NovaSky/Berkeley) | Modular full-stack agentic RL; SkyRL-Agent targets efficient long-horizon agent RL. Closest to our problem shape. |
| **ART / RULER** (OpenPipe→W&B/CoreWeave) | GRPO for multi-step agents with **LLM-elicited relative rewards**; removes hand-crafted reward engineering. |

**RULER** deserves a callout because it directly addresses our hardest RL problem — reward specification on a bespoke toolset. It ranks N trajectories against each other with an LLM judge (relative is easier than absolute; GRPO only needs relative ordering), deduplicates common prefixes, and scores 0–1. Reported: **matches or beats hand-crafted rewards on 3 of 4 tasks**; recommended **4–8 trajectories per group**. Limitations: large groups confuse the judge, judge API cost, and it inherits every LLM-judge pathology in A4.3.

### A2.4 Real data requirements — honest numbers

- **NVIDIA's SLM-for-agents position paper** prescribes **10k–100k curated examples** for specializing an SLM on a narrow agent function, via a 6-step LLM→SLM conversion (instrument → curate/de-PII → cluster tasks → select model → PEFT → refresh). Cited economics: serving a 7B is **10–30× cheaper** in latency/energy/FLOPs than 70–175B.
- **APIGen** (Salesforce): 3,673 executable APIs across 21 categories, three-stage verification (format check → actual execution → semantic verification), **60k verified examples** released. Result: a **7B tops BFCL over several GPT-4 variants**; a **1B beats GPT-3.5-Turbo and Claude-3 Haiku**. This is the cleanest existence proof that verified synthetic tool-call data works — and note the verification pipeline, not the volume, is what makes it work.
- **RL compute:** a single serious agentic RL run is **$10k–$50k on 32–64 H100/H200**, dominated by rollout generation (G=8 rollouts ⇒ ~8× inference per gradient step). The explicit practitioner verdict from 2026 surveys: *"RL post-training from scratch is not economically justified unless you have domain-specific alignment requirements that frontier models cannot satisfy."*
- **The cheap alternative keeps winning on sample efficiency:** **GEPA** (reflective prompt evolution) beat GRPO by **+6 to +19 points using up to 35× fewer rollouts** (678 vs 24,000), averaging **+10% on Qwen3-8B**.

**Planning number for us:** to get a first credible executor SFT you need on the order of **5k–20k verified, deduplicated, outcome-filtered trajectories** covering the tool surface with decent coverage of each tool and each error path. At, say, 40% success on hard tasks and aggressive dedup, that means logging **~50k–200k raw agent episodes**. That is a ledger-volume milestone, and it is the *real* gating condition on decision (a).

---

## A3. Building training data from agent traces

### A3.1 Trajectory schema — design it now, it is nearly free

Emit traces on **OpenTelemetry GenAI semantic conventions** (now natively supported by Datadog and most observability vendors; stable enough in 2026 to build on) and *additionally* write a training-shaped record into the immutable trial ledger. Minimum viable training record:

```jsonc
{
  "trajectory_id": "...", "tenant_id": "...", "parent_experiment_id": "...",
  "policy": {                       // WHO produced this — required for off-policy correction
    "model_id": "claude-x", "adapter_id": null, "prompt_hash": "sha256:...",
    "decoding": {"temp": 0.7, "top_p": 0.95},
    "propensity": 0.13,             // P(action | state) under the LOGGING policy
    "exploration": {"mode": "epsilon_greedy", "epsilon": 0.10, "forced": true}
  },
  "toolset": {                       // WHAT the action space was — required for schema drift
    "registry_version": "2026.09.03", "tools": [
      {"name": "backtest.run", "schema_hash": "sha256:...", "semver": "3.2.0"}
    ]
  },
  "steps": [{
    "idx": 0, "thought_redacted": true,
    "tool": "backtest.run", "tool_semver": "3.2.0",
    "args": {...}, "args_valid": true,
    "result_status": "ok", "result_digest": "sha256:...", "latency_ms": 812, "cost_usd": 0.004,
    "step_label": null                // filled later by step critic: good|unnecessary|mistake|recover
  }],
  "outcome": {
    "terminal": "gate_passed", "gates": {"dsr": 0.62, "oos_sharpe": 1.1, "turnover_ok": true},
    "reward_components": {"verifier": 1.0, "cost_penalty": -0.12, "judge": 0.8},
    "label_available_at": "2026-10-04T00:00:00Z"   // CRITICAL: label latency is a first-class field
  },
  "provenance": {"human_edited": false, "synthetic": false, "teacher_model": null}
}
```

Non-negotiable fields, and why:
- **`propensity` + `exploration.forced`** — without these you cannot do IPS/DR off-policy evaluation later, and you cannot ever prove your recommender isn't just confirming its own past choices. Adding this later is impossible retroactively. (See B10.)
- **`toolset.registry_version` + per-tool `schema_hash`/`semver`** — the only thing that makes a trajectory corpus survive tool evolution.
- **`label_available_at`** — makes label delay queryable, which is the whole of Part B.
- **`result_digest` not `result`** — keeps the ledger small and keeps tenant market data out of the training corpus by default.

### A3.2 Outcome filtering, rejection sampling, and getting more from failures

Naive RFT keeps only successful trajectories and throws the rest away. **Step Rejection Fine-Tuning (SRFT)** (JetBrains, 2026) is a better recipe and directly applicable: an LLM critic labels each step of a *failed* trajectory as `good | unnecessary | mistake | recover`, and training masks loss on harmful steps while keeping them in context.

Numbers on SWE-bench Verified:
- standard RFT (successes only): **30.9%**
- **SRFT: 32.2%**
- naive mixing of unfiltered failures: **28.5%** (worse than doing nothing — this is the trap)

Two findings to internalize: **only ≤24% of steps in wholly failed runs are actually wrong**, and SRFT recovered **~61% of trajectories** that plain RFT discards. For an experiment platform where most experiments fail by design, this is the difference between a viable corpus and a starved one.

**Implication:** our "step critic" internal model isn't a nice-to-have; it is the data-refinery that makes everything else affordable. Build it early, even as a prompted frontier model, and log its labels into the ledger.

### A3.3 Distilling the orchestrator into an executor

The 2025–26 result that changes the recipe: **on-policy distillation** (Thinking Machines). Sample trajectories from the *student*, grade **every token** with the teacher via reverse KL. Dense supervision on on-policy states — avoiding both off-policy distillation's compounding distribution mismatch and RL's O(1)-bits-per-episode sparsity.

Reported numbers (AIME'24, Qwen3-8B-class):
- SFT on 400k examples → 60%
- RL, **17,920 GPU-hours** → 67.6%
- **On-policy distillation, 1,800 GPU-hours → 74.4%**
- **9–30× cost reduction** vs extrapolated SFT; **50–100× faster** than matched RL on comparable tasks.

And the personalization/forgetting result, which is exactly our scenario: mid-training on internal documents at a 70/30 mix moved IF-eval **85% → 79%** and internal-QA **18% → 36%**; with a worse mix IF-eval fell to **45%**, and *no mixing weight preserved the original IF-eval*. On-policy distillation from the **original instruction-tuned checkpoint** on chat prompts recovered IF-eval to **83%** while keeping internal-QA at **41%**.

**This is the recipe for us when the time comes:** orchestrator = frontier model; executor = 8B student; train with on-policy distillation on our own tool-calling states; use a distillation pass against the *pre-fine-tune* checkpoint as the standing antidote to forgetting.

### A3.4 Avoiding distilling the teacher's bad habits

1. **Filter on verifiable outcomes, not on teacher confidence.** Our gates (deflated Sharpe, OOS, turnover, cost) are the ground truth; the teacher is just a proposal distribution.
2. **Never train on `judge`-only reward when a verifier exists.** Judge reward is a fallback for coverage, weighted below verifier reward.
3. **Step-level masking (SRFT)** so you don't inherit the teacher's flailing, retries and dead ends — the specific "bad habits" people mean.
4. **Cap teacher-derived fraction of any training mix** and record `provenance.teacher_model` so you can ablate.
5. **De-duplicate aggressively**, including near-duplicate argument patterns; APIGen-style semantic verification before a record enters the corpus.
6. **Hold out whole *task families***, not random trajectories, or you will measure memorization.
7. **Watch for "format gaming"**: agents learn tool-name insertion without substance and sycophantic escalation. Reward diversity + sandboxed execution + periodic reward-model refresh are the documented mitigations.

### A3.5 Handling a toolset whose schemas keep changing

This is the sharpest argument against fine-tuning now, and it deserves to be stated plainly: **a fine-tuned executor is a cache of your API surface. Every schema change is a cache invalidation.**

Mitigations, in order of value:

1. **Version the schema in the trace** (above). A trajectory is only reusable if you know what the action space was when it was produced.
2. **Schema-conditioned generation**: always place the *current* tool schema in the prompt and train the model to read it, rather than training it to recall tools from weights. You are fine-tuning *how to use tools of this shape*, not *which tools exist*. This preserves generalization to unseen/changed tools, and is the difference between a 6-month-lived adapter and a 6-week-lived one.
3. **Adapters keep only style and control flow**: planning discipline, when to stop, when to escalate, house conventions on units/timezones/instrument identifiers, error-recovery patterns. All of that is schema-invariant.
4. **Additive-only evolution + semver on tools**; deprecate with a shim for ≥1 adapter generation. "Tool schema drift" is a documented silent-failure mode in production agent systems: a single renamed or newly-required field breaks agents with no error surfaced.
5. **Automatic corpus re-validation**: on every registry bump, re-run schema validation over the trajectory corpus; mark records `stale` when their tool's `schema_hash` changed in a breaking way; report **corpus half-life** as a metric. If corpus half-life < training cadence, you cannot fine-tune productively — and that measurement is itself the answer to "is it time yet?"
6. **Tool descriptions are a tunable surface**: 2026 work on learning to rewrite tool descriptions shows meaningful reliability gains with zero model training. Exhaust this first.

---

## A4. Evaluation for fine-tuned agent models

### A4.1 Public benchmarks: use as smoke tests, never as the decision

- **BFCL v4** (Berkeley/Gorilla, ICML'25 paper): simple / multiple / parallel / multi-turn / live / relevance-detection, plus V4's agentic additions — **web search, memory, and format sensitivity**. Format sensitivity is unusually relevant to us: it measures answer consistency when tool schemas are reformatted — a direct proxy for A3.5.
- **τ-bench / τ²-bench** (Sierra): dual-control tool-agent-user interaction, airline/retail/telecom. As of Sept 2026 the top of the leaderboard is compressed at **98–99%** (GLM-5.2 99.1, GPT-5.4 98.9, GLM-4.7-Flash 98.8, Claude Fable 5 98.5), while small open models sit far below (Ministral-3 24.9–27.2, Gemma-4-12B 36.3). **Caution:** the aggregator itself warns these come from heterogeneous harnesses and are not strictly comparable.
- **ToolBench / T-Eval**: older, heavily used for training data; treat any model's ToolBench score as likely contaminated.

**Known gameability:** BFCL's AST grading "rewards exact match on function names and arguments, which means models trained on similar synthetic data score higher on Simple and Multiple categories than their underlying tool-use capability deserves." Read **per-category**, favor Live and Multi-Turn.

### A4.2 The validity audit that should make you build your own harness

"Benchmarking the Benchmarks: A Validity Audit of Tool-Calling Evaluation" (2026) expert-reviewed **496 tasks** across BFCL v4, τ²-Bench Retail, LiveMCPBench, MCP-Atlas:

- **18.5% of official labels disagreed with expert judgment** (92/496). By benchmark: LiveMCPBench **30.5%**, BFCL v4 **20.0%**, MCP-Atlas **13.5%**, τ²-Bench **9.8%**.
- Deterministic evaluators fail through brittle state matching, **trajectory lock-in** (requiring one specific action order), wrong ground truth, and rewards misaligned with completion.
- LLM-judge instability: **23 repeated evaluations of the same setup scored 57.9%–76.8%, an 18.9-point spread.**

Their recommended architecture (which achieved 95.5% human agreement) is exactly what we should copy:
1. **Decomposed metrics** — tool-invocation correctness, task completion, outcome verification scored separately, never one pass/fail.
2. **Deterministic state gates first** — verify observable sandbox outcomes (did the backtest row land? did the order get placed in paper mode?) before any qualitative judging.
3. **Bounded repair windows** — record first-attempt vs post-repair completion separately.
4. **Preserve raw artifacts**, enable turn-level diagnostics, keep human adjudications separate from automated scores.

### A4.3 LLM-judge calibration — report kappa or don't report

"Reliability without Validity" (2026), 21 judges:

- Raw exact-match **overstates agreement by 33–41 points** vs Cohen's κ on MT-Bench. Best judge: **84.9% raw accuracy but κ=0.511** — merely *moderate* agreement.
- κ ranges: MT-Bench **0.376–0.511**, JudgeBench **0.271–0.875**, RewardBench **0.616–0.898**.
- **Position bias is wildly heterogeneous**: 0.002 (Gemini 2.5 Pro) to 0.192 (Qwen3-8B). And the killer: **test–retest reliability ≥0.95 coexists with position bias >0.10** — *reproducibility masks invalidity.*
- Verbosity bias is now **minimal (<0.011)**, contradicting 2023-era 20–40% estimates. Don't over-correct for a bias that no longer exists.
- Cross-benchmark rank instability: Llama-3.3-70B moved **15 positions** between MT-Bench and JudgeBench.

**Minimum Viable Validation Protocol to adopt verbatim:** report Cohen's κ (not exact match); measure position bias via AB+BA pairs; validate on ≥2 task families; report test–retest; never report consistency without a bias audit.

**For our platform:** any internal judge (step critic, proposal ranker's quality head) ships with a **standing human-adjudicated calibration set of ≥200 items per task family**, re-measured every release. Promotion gate: **κ ≥ 0.6 against human adjudication** and **position-bias < 0.05**, or the judge's output may inform ranking but may not gate a promotion.

### A4.4 Catastrophic forgetting and safety drift

- Mechanistic analysis of continual fine-tuning (2026, 20 models): **early-layer attention heads show systemic entropic dispersion; mid-to-deep FFNs show localized representation collapse**; MoE routing shifts between tasks. **Middle layers are peak-susceptible.** Mid-layer CKA similarity drops to **0.412–0.612**. One closed model lost **45.9% accuracy** on original tasks after sequential training. Their LRCP mitigation (gradient updates restricted to orthogonal complements of historical activation subspaces) recovers up to **94.2%** of prior capabilities.
- **Alignment tax:** safety fine-tuning applied *after* RL reduces reasoning accuracy by **7–31%**, hitting agentic capability hardest. Order of operations matters.
- On-policy distillation from the pre-fine-tune checkpoint is the cheapest practical antidote (A3.3).

**Regression suite design (build this before any fine-tune):**
- **Frozen capability set** — general instruction-following, refusal behavior, arithmetic/units, JSON validity. Never touched by training data. Alert on any regression >2pp.
- **Safety set specific to us** — refuses to place live orders without explicit confirmation; refuses to exceed configured risk limits; refuses to act on another tenant's data; correctly abstains when a tool errors instead of fabricating a result. This is where a fine-tuned trading agent actually hurts someone.
- **Tool-schema-perturbation set** — BFCL-v4-style format sensitivity on *our* tools: rename a field, add an optional field, reorder, change an enum. Measures A3.5 risk directly.
- **Held-out task families**, not random splits.
- Run all four on **every adapter build**, in CI, with hard thresholds.

---

## A5. Routing and cascades

This is the highest-ROI item in Part A and it does **not** require training an LLM.

**Evidence:**
- **UCCI** (2026): token-margin uncertainty → isotonic calibration (**ECE 0.12 → 0.03**) → cost-constrained threshold. On a 75k-query production NER task routing between a 4B and 12B: **31% cost reduction (95% CI [27%, 35%]) at micro-F1 0.91**, with the large model scoring 0.928 on escalated queries vs 0.932 on the full set. Beat entropy thresholding (2.31), conformal prediction (2.18), and a FrugalGPT-style learned threshold (2.24) — UCCI 2.08 vs large-only 3.02.
- **RouteLLM**-lineage and 2026 enterprise routers report **30–85%** savings, but that spread is a red flag: the high end is usually measured on query mixes with many trivial queries. Treat **25–40%** as the credible planning number for a hard workload.
- **Conformal cascades** give distribution-free accuracy guarantees for multi-tier inference — attractive where you want a contractual quality floor per tenant.

**Recommended architecture for us (v1, no training):**
1. **Tier 0 — deterministic/programmatic.** Anything expressible as code against the typed tool API should be a *programmatic tool call*, not an LLM decision. Anthropic reports **37% token reduction** (43,588→27,297) and elimination of 19+ inference passes by having the model orchestrate tools in code.
2. **Tier 1 — small/cheap model** for routine executor steps (parameter filling, retries, formatting, obvious next-tool selection).
3. **Tier 2 — frontier model** for planning, ambiguous states, gate interpretation, anything touching capital.
4. **Escalation trigger:** calibrated uncertainty (UCCI-style token margin + isotonic) **plus** hard rules — always escalate when the step touches order placement, risk limits, or a tenant-visible recommendation.
5. **Log the routing decision and its propensity** so the router itself becomes trainable later, and so you can compute counterfactual "what if we'd escalated" costs from logs.

**Critically: prompt caching beats model downgrading.** A coding agent with a 100k-token system prompt: **$4,800/mo** uncached on a Sonnet-class model, **~$240/mo at 95% cache hit rate**, vs **$1,600/mo** switching to a Haiku-class model with no caching. Caching wins by **6.7×** *and* keeps capability. Anthropic: **90%** discount on cache reads; write premium **1.25×** (5-min TTL) or **2×** (1-hour TTL). Google: 90% on Gemini 2.5+. OpenAI: 50–75%. **Target ≥85% cache hit rate before you consider a smaller model at all.**

---

## A6. THE HONEST VERDICT — fine-tune now, or not?

### A6.1 The case against, stated as strongly as possible

1. **Prompt-side wins are bigger, cheaper, and not yet exhausted.**
   - **Tool search**: Opus 4 **49% → 74%**, Opus 4.5 **79.5% → 88.1%** on MCP evals, with **85% token reduction** (191,300 tokens preserved vs 122,800). A 5-server MCP setup costs ~55k tokens before the conversation starts; adding Jira pushes past 100k.
   - **Tool-use examples**: **72% → 90%** on complex parameter handling.
   - **Programmatic tool calling**: **37%** token reduction; internal knowledge retrieval 25.6% → 28.5%; GIA 46.5% → 51.2%.
   - **GEPA**: **+10% average, up to +19%**, with **35× fewer rollouts** than GRPO.
   These stack. Together they plausibly deliver more than a first-generation fine-tune, in days rather than quarters, with zero serving burden.
2. **Our toolset is young.** Fine-tuning caches an API surface that is going to churn hard for the next several quarters. Corpus half-life is likely shorter than the training cadence.
3. **We do not have the data yet.** 10k–100k curated examples (NVIDIA), or 5k–20k verified trajectories by our own estimate. You cannot filter what you haven't logged.
4. **Evaluation is the real cost.** LoRA training for a 7–8B is **$3–$10 of GPU** (2–4h on an A100 at $1.19–1.39/hr; H100 $1.99–2.69/hr; 70B QLoRA $15–30). Complete projects still land at **$5k–$15k** because *dataset prep and evaluation* are the expense. Our eval harness must exist regardless — so build it first, and it will tell us whether fine-tuning is needed.
5. **Multi-tenant adapter economics are worse than the marketing.** Correlated load across tenants defeats adapter-cache pooling exactly at peak.
6. **Trading raises the cost of a bad model.** Alignment tax of 7–31% on agentic capability from post-RL safety tuning; safety drift from fine-tuning is documented; and the downstream action is *real capital at Coinbase/Alpaca*.
7. **Serving burden is permanent.** A fine-tuned model means GPU capacity planning, on-call, version skew with the base model's own upgrades, and losing every capability improvement the frontier ships next quarter.

### A6.2 The case for (when it arrives)

- **Volume economics** (below).
- **Latency**: an 8B on local GPU gives first-token in tens of ms vs hundreds for a hosted frontier model. If we build a live execution-monitoring agent that must react within a bar, this becomes the dominant argument.
- **Determinism**: a pinned local checkpoint is bit-reproducible in a way a hosted model that silently updates is not. For an **immutable trial ledger with reproducibility claims**, this is a real, defensible requirement — arguably the strongest non-cost argument we have.
- **Privacy**: tenant strategy payloads never leaving our VPC.
- **Capability on narrow tasks**: a Qwen2.5-7B reached **88%** on a power-outage classification task vs **31%** for a prompted frontier model, at **14× cost saving**. Narrow + high-volume + stable = fine-tuning's home turf. Our **internal** models (cost predictor, failure classifier) are exactly this shape — but they are gradient-boosted trees, not LLMs.

### A6.3 Break-even math — the actual numbers

**Cost per executor-model call, frontier, with caching.** Assume 20k input tokens (tool schemas + state) at 90% cache hit, 500 output tokens, Sonnet-class ($3/M in, $15/M out, cache read $0.30/M):

```
input : 18,000 × $0.30/M  = $0.0054
        2,000 × $3.00/M   = $0.0060
output:   500 × $15.00/M  = $0.0075
                          ≈ $0.019 / call     (uncached would be ≈ $0.068 — 3.5×)
```

**Cost per call, self-hosted 8B + LoRA on one H100 @ $2.50/hr.** With continuous batching and prefix caching, ~2,000 output tok/s sustained mixed throughput ⇒ ~$0.35/M output, input effectively ~$0.03–0.05/M:

```
≈ $0.0012 / call   (~16× cheaper per call)
BUT the GPU is rented whether you use it or not: $2.50 × 730 h ≈ $1,825 / month
```

**Break-even #1 — cover the GPU only:**
```
$1,825 / ($0.019 − $0.0012) ≈ 102,000 calls/month ≈ 3,400 calls/day
```
With HA (2 GPUs): **~6,800 calls/day**. This is the *easy* threshold and it is misleading on its own.

**Break-even #2 — cover the GPU + the people.** A fine-tuning program is not free labor: data curation, eval harness maintenance, retraining, regression triage, on-call ≈ **0.5–1.0 FTE ≈ $20k–$50k/month** fully loaded.
```
($1,825 × 2 + $35,000) / $0.0178 ≈ 2.17 M calls/month ≈ 72,000 calls/day
```
Round up for eval compute, judge calls, and the second adapter you'll inevitably need: **~100,000 executor-calls/day sustained.**

**Sanity check against independent 2026 guidance** — which lands in the same place:
- <1,000 requests/day → stay prompted
- 1,000–10,000/day → prompt-optimize + cache
- 10,000+/day → fine-tuning ROI starts improving
- and explicitly: **prompt caching moved the break-even from ~10k/day a year ago to ~50k–100k/day now.**

**What 100k calls/day means for us:** at ~30 executor calls per agent experiment, that's **~3,300 experiments/day**, ~100k/month across all tenants. That is a real, checkable ledger metric. Put it on a dashboard.

### A6.4 Explicit break-even conditions — the trigger list

Fine-tune when **≥3 of the following are simultaneously true**, and not before:

| # | Condition | Threshold | Why |
|---|---|---|---|
| 1 | **Call volume** | ≥100k executor-calls/day sustained for 30 days (or ≥3.4k/day if the work is done by an existing ML engineer at near-zero marginal labor) | Break-even #2 above |
| 2 | **Toolset stability** | Trajectory-corpus half-life ≥ 6 months; <10% of tools take a breaking semver bump per quarter | Otherwise you retrain faster than you benefit |
| 3 | **Corpus** | ≥10k verified, deduped, outcome-filtered trajectories with ≥200 per tool and ≥50 per error path | A3.4 / NVIDIA's 10k–100k |
| 4 | **Eval harness** | Private harness with deterministic state gates, decomposed metrics, ≥95% human agreement, and a frozen regression suite already in CI | You cannot promote what you cannot measure |
| 5 | **Prompt-side exhausted** | Tool search + tool-use examples + programmatic tool calling + GEPA/MIPRO shipped, and ≥85% prompt-cache hit rate achieved | These beat a first-gen fine-tune |
| 6 | **Latency** | A product surface requires p95 < 300 ms first-token that hosted models cannot meet | Only local wins here |
| 7 | **Determinism** | A reproducibility or audit requirement (ledger replay, regulator, tenant contract) requires a pinned, versioned, immutable model artifact | Genuinely unsolvable by prompting |
| 8 | **Privacy** | A tenant contractually forbids strategy payloads leaving our infrastructure | Genuinely unsolvable by prompting |
| 9 | **Capability gap** | A measured task on our harness where a prompted frontier model plateaus below requirement and a distilled student closes it | The only *quality* justification |

**Conditions 7 and 8 can trigger alone.** If a single institutional tenant requires on-prem/VPC inference or bit-reproducible model artifacts, that one contract can justify the whole program independent of volume — and given "real capital routed to brokers," that request is more likely here than in a generic SaaS.

### A6.5 "Not yet, but log for it" — the concrete instrumentation checklist

Do all of this now; each item is cheap now and impossible to retrofit:

- [ ] Trajectory schema of A3.1 written to the immutable ledger on **every** agent step — including `propensity`, `exploration.forced`, `toolset.registry_version`, per-tool `schema_hash`/`semver`, `label_available_at`.
- [ ] **Forced exploration**: ε≈5–10% of agent decisions sampled from a deliberately broader distribution, flagged, with propensity logged. This is the only source of unbiased off-policy evaluation data you will ever get. (See B10.)
- [ ] **Step critic** running (prompted frontier model initially) writing `good|unnecessary|mistake|recover` labels — enables SRFT later, enables process supervision later, and is useful immediately for debugging.
- [ ] **Private eval harness**: deterministic state gates, decomposed metrics (invocation correctness / task completion / outcome verification), bounded repair windows, raw-artifact retention, human adjudication table kept separate from auto-scores.
- [ ] **Three dashboard metrics that gate the decision**: executor-calls/day; trajectory-corpus half-life (from schema_hash churn); verified-trajectory count per tool.
- [ ] **Routing + propensity logging** so the router is trainable later.
- [ ] **Tool registry with semver + additive-only policy + deprecation shims.**
- [ ] **Prompt caching to ≥85% hit rate**, and stable prompt prefixes ordered cache-friendliest-first.
- [ ] **Frozen regression + safety suites** in CI before any model change of any kind (prompt, base model, adapter).

---

# PART B — Drift, retraining policy, and self-improving systems

## B7. Drift detection

### B7.1 The taxonomy, and which half matters

| Kind | What changed | Detectors | Needs labels? |
|---|---|---|---|
| **Covariate / data drift** | P(X) | KS, Chi², PSI, Wasserstein, MMD, **domain classifier** (AUC of "can a model tell reference from current?") | No |
| **Prediction drift** | P(Ŷ) | same tests on outputs | No |
| **Concept drift** | P(Y\|X) | ADWIN, DDM/EDDM, Page-Hinkley, KSWIN, SEED, STEPD, ABCD | **Yes (or a proxy)** |
| **Label/upstream drift** | P(Y) | frequency tests | Yes |

**The central fact for a trading platform: concept drift is the one that costs money, and it is the one you cannot see without labels.** Evidently states it plainly: *"there is no way to identify concept drift without any ground truth data."*

### B7.2 Why distribution drift alone is a bad retrain trigger — the evidence

1. **Evidently's own position:** data drift "doesn't guarantee a drop in model quality." Track it; don't trigger on it. Retraining should follow actual quality degradation or arrival of sufficient new labels.
2. **"When to Retrain a Machine Learning Model" (arXiv 2505.14903)** compared ADWIN/FHDDM/KSWIN triggers vs cost-aware methods (CARA) vs a performance-forecasting method (UPF) vs an oracle. The finding: drift-triggered approaches *"work well with low retraining costs, but perform poorly when retraining costs are high, as they tend to recommend retraining far too often."* On the electricity dataset at high retrain cost (α=0.9): ADWIN-5% cost **3.27 ± 0.4** with 1.0 retrains, CARA **2.78 ± 0.19** with 0 retrains, **UPF 2.69 ± 0.26** with 0.1 retrains, oracle **2.68**. At low cost (α=0.1) UPF **2.24 ± 0.17** vs CARA **2.73 ± 0.25**, consistent across seven datasets.
3. **PSI specifically is statistically fragile.** Its 0.1/0.25 thresholds are folklore with no distributional basis; PSI is strongly **sample-size dependent** and binning-dependent — with large samples you flag everything, with small samples nothing. The banking literature has been trying to replace it with proper chi-square-calibrated statistics for years. **Do not use bare PSI thresholds as an automated trigger.**
4. **Detector benchmarking (arXiv 2606.07789)**, 14 detectors × 7 datasets × 50 Monte Carlo trials, with timing-aware metrics (F1, normalized detection time, episode recall, false-alarm rate): **SEED and STEPD** were consistently top-3 across drift types; **ABCD** best false-alarm rate and strongest on feature-based drift. Most detectors do better on abrupt than gradual drift. Their key methodological recommendation: **leave-one-dataset-out hyperparameter optimization**, because detector defaults overfit.
5. **CALIPER (arXiv 2603.09024)** makes a subtler point that reframes the whole problem: detecting drift tells you *when*, not *how much post-drift data you need*. It gates retraining on an effective-sample-size + monotone-locality test, matched or beat the best fixed-window strategy without per-dataset tuning, and crushed incremental updates (MLP on MoCap: **MSE 7.106 vs 412.6**; Dysts **0.432 vs 71.75**).

**Synthesis:** drift detectors are **alerting instruments and routing hints, not triggers**. The trigger should be (i) measured or *estimated* performance degradation, (ii) sufficient post-drift data, and (iii) a cost-aware decision rule.

### B7.3 Tooling in 2026, judged against long label delay

| Tool | Type | Performance-without-labels | Verdict for us |
|---|---|---|---|
| **NannyML** | OSS lib | **CBPE** (classification) and **DLE** (regression) | **The most relevant capability — and the most dangerous to misuse.** See below. Notebook/batch-shaped; you supply the orchestration. |
| **Evidently** | OSS (Apache-2.0) + SaaS | drift + prediction drift + proxy heuristics | **Best default OSS.** Highest transparency, tabular+text, batch+realtime, fsspec storage, CI/CD integration, Spark. No raw-image support (irrelevant). |
| **Arize** | SaaS | drift, explainability, fairness, good alerting (Slack/PagerDuty), no serving opinions | Good if we want managed; strong on agent/LLM observability in 2026. |
| **WhyLabs** | OSS SDK (whylogs) + SaaS | profile-based logging, drift on sketches | Good for cheap, privacy-friendly profile logging across tenants (sketches, not rows). |
| **Fiddler** | SaaS + on-prem/self-hosted (K8s) | drift + explainability | Relevant only if a tenant demands self-hosted monitoring. |
| **Deepchecks** | OSS (**AGPL** — enterprise problem) + commercial | testing rather than monitoring; OSS explicitly not production-scale | Use as a **test library in CI**, not as production monitoring. |

**The NannyML caveat that decides everything.** CBPE's stated assumptions are: **well-calibrated probabilities, no covariate shift into unseen regions, no concept drift, sufficient sample size.** It handles covariate shift beautifully and **fails on concept drift by construction** — and it cannot estimate accuracy below 50% at standard thresholds.

Now look at what trading is: **concept drift is the phenomenon.** P(Y|X) changing *is* alpha decay. So:

> **CBPE/DLE can tell you "the market looks different but your model should still work." They cannot tell you "your edge is gone." In trading, the second one is the whole question.**

Use CBPE/DLE as a **fast-moving early-warning layer on the platform-physics models** (cost predictor, failure classifier, latency — where P(Y|X) genuinely is stable and inputs shift), and **do not** rely on them for anything whose label is a market outcome.

### B7.4 What to do when labels arrive with a long horizon delay

This is our normal case: a gate-outcome label may take days; a "did this strategy actually work in live capital" label takes weeks to months.

Layered approach:

1. **Make label latency explicit.** `label_available_at` on every record; a per-model dashboard of *label maturity curve* (what fraction of predictions from T days ago now have labels).
2. **Shorten the label where possible.** Use intermediate verifiable gates as **proxy labels** with known, measured correlation to the terminal label. Validate the proxy's correlation on historical data and re-measure quarterly; a proxy whose correlation to the terminal outcome has itself drifted is worse than no proxy.
3. **Meta-labeling** (López de Prado): a secondary classifier that predicts whether a primary signal's trade will be profitable. It produces a *shorter-horizon, higher-frequency* label stream than "was the strategy good," improves precision and enables position sizing. Community evidence is that it is **not a silver bullet** — it improves precision at the cost of recall and adds another overfittable model — but as a *labeling accelerator* for our gate pre-screener it is well matched.
4. **Prediction drift + input drift as leading indicators**, per Evidently's guidance — alert, investigate, never auto-retrain.
5. **Reserve judgement windows.** A model's performance metric is *provisional* until label maturity crosses (say) 80%. Never promote or demote on immature labels. Encode maturity in the promotion gate.
6. **Cost-aware decision rule over a performance forecast** (UPF-style / learning-debt-style, next section) rather than a threshold on a drift statistic.

---

## B8. Retraining policy

### B8.1 The 2026 evidence on *when*

**"Learning Debt and Cost-Sensitive Bayesian Retraining" (arXiv 2604.06438)** is the most decision-useful paper here. Definitions:
- **Learning debt** D_t = KL(π*_t ‖ π_τ(t)) — divergence between a continuously-updated posterior and the frozen deployed one. Model staleness *without* actually retraining.
- **Actionable staleness** Z_t — the policy-relevant latent state, explicitly distinguished from generic drift: a model is actionably stale when keeping the frozen posterior one more interval costs material expected loss.

**Theorem 1 (one-step Bayes-optimal retraining rule):**
```
retrain  iff   ρ_t  >  c_churn / (c_churn + c_wait)
```
where ρ_t = posterior probability of actionable staleness, c_churn = excess loss from an unnecessary retrain, c_wait = excess loss from delaying. **This is the rule to implement.** It makes the retrain threshold an explicit function of the two costs rather than a magic number.

Results: on gradual drift the debt-filter beat a 10-period calendar baseline in **24/24** cells and the best fixed cadence in **24/24**; abrupt coefficient shifts **15/24** and **10/24**; variance shifts **17/24** and **17/24**. On a 104-week Airbnb booking-forecast backtest it achieved **0.36×** the excess loss of semi-annual calendar retraining, and after a Jan-2024 payment-policy shock it triggered a retrain on **8 Mar 2024 — 16 weeks early** — cutting cumulative excess loss from **9.33 → 3.33**.

**Two honest caveats from the same paper**, which is why I trust it: a **fixed-threshold CUSUM remained competitive and often superior**, and the **proxy-filter built on observable monitoring diagnostics performed poorly**. Translation: (i) don't over-engineer — a well-tuned CUSUM on a performance proxy is a legitimate v1; (ii) *monitoring dashboards are not a retraining signal* — you need the posterior/performance quantity itself.

### B8.2 Cadence, windows, and the trading-specific evidence

**Alpha decay sets the outer bound on cadence.** A 2026 modeling paper on AI-driven alpha decay estimates tradeable-signal half-life compressing from a **pre-AI 5–7 years (~58 months)** to **~18 months** at current adoption (φ≈0.7, homogeneity ρ≈0.6) — a 3–4× acceleration, through signal crowding, performative erosion (your own trading contaminates the data you retrain on), and Red Queen dynamics. **Skepticism warranted**: this is a calibrated theoretical model, not a measurement. But the direction is corroborated by the classic publication-effect literature (McLean & Pontiff) and by BlackRock's 2026 crowding warnings for hedge funds.

**The counterweight, and it is important.** A critical August-2026 review of AI in equity and crypto markets concludes the public evidence does **not** establish that AI methods deliver *"persistent, cross-regime, capacity-aware net alpha."* Failure modes along the "alpha-translation chain": temporal contamination, concentration in illiquids, excessive turnover, unmodeled transaction costs, capacity limits. Time-series foundation models show **"small and sparse"** improvements over random walk on returns once costs are included. One celebrated ML alpha vanished after look-ahead correction. Crypto specifics: perp-vs-spot differences (funding, liquidation, discontinuous leverage), MEV, gas, AMM inventory loss — and the AI-specific crypto record is **thinner** than equities.

**What this implies for our product, which is the important bit:** the platform's value proposition cannot be "our models find alpha." It has to be **"our platform makes it cheap to test honestly and expensive to fool yourself."** That directly shapes the retraining policy — the gates, the frozen holdouts, and the deflated-Sharpe discipline are the product, not overhead.

**Windows:**
- **Rolling window** when the process is genuinely non-stationary and old regimes mislead (most price-based signals; crypto especially).
- **Expanding window** for platform-physics models where more data is monotonically better (cost prediction, failure classification, runtime prediction).
- **Regime-weighted / recurring-drift-aware** where drift is cyclical: don't discard the 2022-style regime, down-weight it, and keep it retrievable. Evidently's taxonomy explicitly names recurring drift; HMM/regime-switching models on crypto are a reasonable way to tag bars with a regime label for weighting.
- **CALIPER-style sufficiency gate**: don't retrain until the post-drift window passes an effective-sample-size test. Retraining on 200 post-drift minutes is how you manufacture a worse model than the stale one.

**Validation is non-negotiable and specific:** purged K-fold with embargo, and **combinatorial purged CV** for path-dependent statistics; **deflated Sharpe ratio** to correct for selection bias/backtest overfitting/non-normality given the number of trials. Because our platform runs *thousands of AI-driven experiments*, the effective number of trials N in the DSR correction is enormous and **must be taken from the trial ledger, not self-reported by the user**. This is a genuine product differentiator: we can compute an honest DSR because we have an immutable count of every trial that was run.

### B8.3 Champion–challenger, shadow, canary, rollback

Standard MLOps, tightened for capital:

1. **Offline gate** — challenger must beat champion on the frozen holdout **and** on the most recent purged-CV fold, by a margin exceeding the estimated noise (bootstrap CI, not a point estimate).
2. **Shadow** — challenger scores live traffic, writes predictions, takes no action. Minimum duration = **one full label-maturity horizon**, not a fixed number of days. For trading-outcome models this is weeks; accept it.
3. **Canary** — promote to a small share: start at 5% of *experiments* (not of capital) for capital-adjacent models; for platform-physics models, 10–25% of traffic. Ramp 5% → 25% → 50% → 100% with a minimum soak per stage.
4. **Automatic rollback criteria** (all pre-registered, all one-sided tests at fixed α, evaluated only on mature labels unless otherwise noted):
   - primary metric worse than champion by > rollback margin (CI-based), **or**
   - any guardrail metric breached (p99 latency, cost/trial, tool-error rate, abstention rate), **or**
   - **entropy/diversity floor breached** (see B10), **or**
   - **safety/regression suite failure** (immediate, no statistics needed), **or**
   - calibration degradation (ECE above threshold) — a miscalibrated pre-screener silently destroys the gate, **or**
   - any tenant-scoped metric degrading >X% even if the global metric improves (prevents "helps the average tenant, wrecks the small ones").
5. **Rollback must be one action and always available.** Keep the previous adapter/model artifact hot; for LoRA, both adapters are already resident — rollback is a routing-table flip and should take seconds.
6. **Model registry with immutable artifacts**, lineage to the exact training-data snapshot and code commit. This is also your SR 11-7 story.

**Governance framing.** SR 11-7 (Fed/OCC model risk management) is the lingua franca here even for non-banks: effective challenge, independent validation, ongoing monitoring, documented limitations. 2026 commentary is clear that the framework **holds for AI but strains on agentic systems** — the framework assumes a model is a stable artifact with a defined input/output, and an agent that chooses its own tools is not. Practical translation for us: treat the *agent policy* (prompt + tools + adapter + router config) as **the versioned model artifact** and validate it as a unit. That single framing decision solves most of the governance problem.

---

## B9. Online / continual learning

**Mechanisms:** replay buffers (incl. gradient-coreset selection), EWC and other regularization, online gradient/SGD, architectural isolation. All are attempts on the **stability–plasticity dilemma**; the deep-continual-learning literature adds the harder finding of **loss of plasticity** — networks trained continually progressively lose the *ability to learn new things at all*, not just old ones.

**Why finance teams mostly periodically refit:**
1. **Label delay.** Online learning wants a label per example, promptly. Trading labels arrive late and in correlated batches. An "online" learner on delayed labels is just a badly-scheduled batch learner with worse reproducibility.
2. **Reproducibility and audit.** A periodically-refit model has a version, a training-set hash, and a backtest. An online model has a trajectory — near-impossible to validate under SR 11-7-style expectations, and near-impossible to attribute P&L to.
3. **Noise.** Minute-bar returns have signal-to-noise so low that per-example updates chase noise. Batch refit with purged CV at least lets you measure whether you learned anything.
4. **Empirical results favor windowed retraining over incremental updates** where measured — CALIPER's comparison is stark (MLP on MoCap: MSE **7.106** for window-sufficient retrain vs **412.6** incremental; Dysts **0.432 vs 71.75**).
5. **Tree ensembles dominate tabular finance and don't do online well.** GBDTs are refit, not updated. That's a practical constraint, not a philosophical one.

**2025–26 counterevidence worth respecting:**
- **On-policy distillation** as continual learning: recovering a degraded behavior (IF-eval 79→83%) while retaining newly-learned knowledge (41%) is a genuinely new capability — *continual adaptation with a principled anti-forgetting mechanism*, cheap enough to run routinely.
- **LRCP** (restricting updates to orthogonal complements of historical activation subspaces) recovering **up to 94.2%** of prior capabilities makes sequential fine-tuning far more viable than it was in 2023.
- **Continual safety alignment via gradient-based sample selection** and similar 2026 work makes "keep adapting without losing alignment" tractable.
- Replay + coreset selection is now cheap and effective enough that a **small mandatory replay fraction** should be standard in any refit.

**Recommendation:** **periodic refit with replay, not online learning**, for every model in the platform. Two exceptions, both narrowly scoped: (i) **calibration layers** (isotonic/Platt on top of a frozen model) may update continuously — they are low-capacity, auditable, and trivially rollback-able; (ii) **bandit-style parameters** in the experiment scheduler (arm values) update online by construction, but with logged propensities and a forced-exploration floor.

---

## B10. Self-improving systems that train their own internal models

This is the highest-risk section and it deserves the most conservative treatment.

### B10.1 The failure modes, with citations

1. **Model collapse from self-generated data.** Shumailov et al. (Nature, 2024) showed recursive training on own outputs degrades tails then the whole distribution. The 2025–26 refinement matters enormously: **collapse is driven by data *replacement*, not data *accumulation*.** If each generation *replaces* real data with synthetic, variance compounds (Var ∝ Σ(1/M_i) + 1) and the distribution smooths toward uni-modal; if synthetic data *accumulates alongside* real data, iterative MLE can remain consistent. Also: the conclusion is **metric-dependent** — KL can appear to stabilize while Wasserstein grows monotonically. **Operational rules: never replace, always accumulate; always keep a real-data floor; monitor with ≥2 divergence metrics including a transport metric.**
2. **Feedback loops / performative prediction.** Predictions change the world that generates the data. In our case this is not theoretical — it is literally how trading works (your fills move the book), and the alpha-decay paper names **performative signal erosion** as a distinct decay channel: retraining on data contaminated by your own trading degrades coefficients. Fairness-feedback-loop work shows the same amplification dynamic for synthetic data.
3. **Exploration/entropy collapse.** GRPO-style optimization has an intrinsic bias toward entropy-decreasing updates (A2.2). A recommender trained on its own choices only ever sees its own choices — the classic bandit degeneracy. This is the failure mode I'd bet on happening to us first.
4. **Goodharting an internal metric / reward hacking.** The 2026 survey's **Proxy Compression Hypothesis**: objective compression + optimization amplification + **evaluator–policy co-adaptation**. Four exploitation levels — feature (verbosity/formatting), representation (fabricated reasoning, process–outcome decoupling), **evaluator** (gaming the judge, prompt injection), **environment** (rewriting tests, manipulating the oversight infrastructure). Gao et al.'s **reward-model overoptimization scaling laws** quantify the proxy/true divergence as a predictable function of optimization strength (KL budget); direct-alignment algorithms like DPO show the same degradation without an explicit RM. Documented escalation: benign length-bias hacks generalize into **portable proxy-optimization strategies** and, in some studies, into **alignment faking** — strategically modeling the evaluator as a manipulable object.
   **In our setting the environment-level attack is concrete and obvious:** an experiment-proposal ranker optimized on "fraction of proposals that pass the gate" will learn to propose experiments that pass *gates*, which is not the same as strategies that make money. Given a step critic and a gate evaluator that are themselves models we train, co-adaptation is the default outcome, not a tail risk.
5. **Silent degradation.** No exception is thrown when a model gets worse. With long label delay, the detection lag can exceed the damage horizon entirely.
6. **Runaway self-optimization.** The **Darwin Gödel Machine** (ICLR 2026) is the reference point for empirical self-improvement: SWE-bench **20.0% → 50.0%**, Polyglot **14.2% → 30.7%**, by iteratively rewriting its own code with an archive of stepping-stones. Its authors report **no evidence of harmful or malicious behavior**, and their guardrails are exactly the ones we should copy: **sandboxed execution with resource/time limits; complete archive lineage enabling audit and rollback; scope restriction (self-modification limited to its own Python codebase and eval harness); human oversight.** They flag the honest residual risks: benchmark optimization can introduce vulnerabilities, and growing complexity reduces interpretability.
7. **Evaluator instability masquerading as progress.** Recall the 18.9-point spread across repeated LLM-judge runs and the 18.5% benchmark label-error rate (A4.2). A self-improving loop optimizing against a noisy evaluator will happily "improve" into the noise.

### B10.2 Maintaining a valid training distribution when your own policy generates the data

This is the logged-bandit-feedback problem, and it is the technical heart of Part B.

**The problem, precisely:** you only observe outcomes for experiments the platform chose to run (**exposure bias**), and "no result" is ambiguous — a proposal that was never run has no counterfactual, and a failed gate may mean a bad idea *or* an unlucky window.

**The corrections, in the order I'd implement them:**

1. **Log propensities.** π_log(a|s) at decision time for every choice the platform makes (which experiment to run, which strategy family to recommend, which model to route to). Without this, IPS/SNIPS/DR are all unavailable. **This is the single highest-value, lowest-cost thing to do today.**
2. **Force exploration.** A deterministic logging policy makes off-policy evaluation formally impossible — there is a whole 2026 literature on partial identification under deterministic logging precisely because so many production systems forgot this. Maintain **ε ≈ 5–10%** of decisions drawn from a broader distribution, flagged as `forced`.
3. **Design the logging policy deliberately.** "Logging Policy Design for OPE" (2026) characterizes the **reward–coverage tradeoff**: concentrating on high-reward actions cuts variance but coverage failures cause bias/variance blowup. Under full ignorance, **uniform randomization is minimax optimal**; with knowledge, a **Neyman allocation** weighting by target-policy mass × √(reward probability) minimizes variance. Practically: use a **soft-greedy family** (top-k / softmax / power-normalized) with a **single greediness parameter** that you tune against sample size and action-space size. Notable result: a well-designed logging policy can achieve **lower MSE than on-policy A/B testing** while *accruing higher expected reward than the target policy during deployment* — exploration is not purely a tax.
4. **Estimate off-policy with DR/SNIPS, not naive averages.** Use doubly-robust estimators; report CIs; clip weights and report the clipping.
5. **Handle asymmetric feedback.** ABPO (2026) addresses exactly our structure in LLM-recommender updates: insert the exposed recommendation as a **logged anchor into each GRPO rollout group**, apply IPS against the actual prior policy, and **temper ambiguous negatives** using output-token confidence as a verifier-free reliability signal. Consistent post-update gains across five Amazon Reviews / MovieLens domains with reduced exposure bias.
6. **Keep a real-data floor and accumulate.** Every training mix for an internal model includes a minimum fraction of (a) forced-exploration records and (b) records whose labels came from terminal, verifiable outcomes rather than model judgments. Never train generation N only on generation N−1's outputs.
7. **Frozen holdouts that are genuinely frozen.** A holdout set the agents never see, never propose against, and that is never used for model selection — only for release/no-release. Rotate on a slow, pre-announced schedule (e.g., annually), and *retire* a holdout permanently once it has been used enough times to be leaked into decisions.

### B10.3 Guardrail architecture — concrete

```
tier 0  LEDGER          immutable, append-only, propensity + toolset version + label_available_at
tier 1  DATA GATES      real-data floor; forced-exploration floor; accumulate-never-replace;
                        dedup; provenance tags; corpus staleness marking on schema bumps
tier 2  TRAINING GATES  purged/combinatorial CV; entropy floor; KL budget vs previous champion;
                        replay fraction; calibration fit on a separate split
tier 3  EVAL GATES      frozen holdout (never trained on, never selected on);
                        decomposed metrics; judge κ ≥ 0.6 + position bias < 0.05;
                        safety + regression suites; deflated Sharpe with N from the ledger
tier 4  DEPLOY GATES    shadow ≥ 1 label-maturity horizon → canary 5% → ramp;
                        pre-registered rollback criteria; human approval for capital-adjacent
tier 5  RUNTIME         entropy/diversity SLO; cost & latency SLO; calibration monitor;
                        per-tenant metric monitor; automatic rollback; kill switch
tier 6  AUDIT           full lineage artifact→data snapshot→code commit; agent policy versioned
                        as one unit (prompt+tools+adapter+router); periodic human review of
                        a random sample of promoted decisions
```

**Anti-Goodhart specifics** (from the reward-hacking survey's three levers):
- **Reduce compression** — never a single scalar internal objective. Use **vector-valued rewards** (edge, cost, risk, novelty, reproducibility) and require **Pareto non-domination** for promotion rather than a weighted sum that can be gamed on one axis.
- **Control amplification** — explicit **KL/trust-region budget** against the previous champion policy; optimization-budget caps; detect overoptimization by watching proxy-vs-true divergence on the frozen holdout.
- **Break co-adaptation** — **rotate and refresh the evaluator** on a schedule; keep at least one evaluator (a human adjudication sample, or a frozen deterministic verifier) that is *never* updated in response to policy behavior; never train the policy and its judge on the same data in the same cycle.
- **Never let the system modify its own gates.** Scope restriction, DGM-style: the agents may propose experiments; they may not edit the gate thresholds, the holdout, the DSR correction, or the promotion criteria. Make this an access-control boundary in code, not a norm.

---

## B11. Multi-tenancy: per-tenant, global, or hierarchical?

### B11.1 The information-class split (the key idea)

Don't ask "per-tenant or global?" for the platform. Ask it **per model, classified by what information the model's parameters would encode.**

| Model | What it encodes | Leakage risk | Recommendation |
|---|---|---|---|
| **Cost predictor** (runtime, $, GPU-hours) | platform physics: our infra, our tools | ~none | **Global.** Pool everything. Big cold-start win for new tenants. |
| **Failure classifier** (will this trial crash/timeout?) | platform physics + common misconfigurations | ~none | **Global.** |
| **Learning-curve extrapolator** | optimizer/architecture dynamics | ~none | **Global.** Well-studied (LC-PFN-style Bayesian extrapolation is mature and cheap). |
| **Step critic** | tool-use competence | low (redact args) | **Global**, trained on schema + step-shape features, **not** on argument values. |
| **Fine-tuned executor LLM** | how to call *our* tools | low if trained on redacted traces | **Global**, single adapter (see A1.4). |
| **Gate-outcome pre-screener** | *which experiments pass gates* | **medium-high** | **Hierarchical**: global prior on tenant-agnostic features (sample size, embargo, turnover, N-trials, data quality) + per-tenant adaptation on anything instrument- or signal-specific. |
| **Strategy-family recommender** | **what works, right now, in this market** | **HIGH — this is alpha** | **Per-tenant by default.** Global component restricted to methodology features only. |
| **Experiment-proposal ranker** | what to try next | **HIGH** | **Hierarchical with a hard feature firewall** (below). |

### B11.2 The competitive-information problem, stated in trading terms

A global strategy-family recommender is, functionally, **a mechanism for broadcasting one tenant's edge to every other tenant.** That is not a privacy edge case; it is the core product risk.

Three distinct harms:
1. **Direct misappropriation.** Tenant A discovers that a specific funding-rate/basis relationship on a specific venue works; the recommender learns it; tenant B is told to try it. A has paid the research cost and B free-rides. Even with no PII and no raw data transfer, the *value* has moved.
2. **Crowding / accelerated decay.** If the recommender is any good, it homogenizes strategies across the tenant base — precisely the **algorithmic homogenization** mechanism that compresses signal half-life (modeled at ~58 → ~18 months). We would be *manufacturing* the decay of our own users' edges, then being blamed for it. This mirrors the multi-manager-platform crowding problem: pod shops enforce information barriers between pods for exactly this reason, and BlackRock's 2026 crowding warning is about the systemic version.
3. **Reputational/contractual.** Sophisticated traders will ask "does my experiment data train models that serve other users?" There is exactly one answer that keeps them: **"Not for anything strategy-specific, and here is the schema-level enforcement."**

### B11.3 The feature firewall — the enforceable version

Make it a **schema-level, testable rule**, not a policy document. For any model with a global component, the permitted feature space is restricted to a whitelist:

**Allowed to cross tenant boundaries (methodology / platform physics):**
- experiment configuration shape: CV scheme, embargo length, sample size, number of trials, walk-forward structure
- resource facts: runtime, memory, cost, tool-error rates, retry counts
- data-hygiene facts: missing-bar rate, timestamp alignment, look-ahead-check results
- outcome *statistics in the abstract*: "experiments with N trials and this CV scheme show DSR inflation of X" — a statement about **methodology**, never about **instruments or signals**

**Never allowed to cross tenant boundaries:**
- instrument identifiers, venue identifiers, pool addresses
- feature/signal definitions, formulas, factor names, code
- realized performance conditional on a strategy family
- any embedding computed from strategy code or research text
- anything that lets a model reconstruct "this strategy on this asset in this regime works"

Add **k-anonymity thresholds** on any cross-tenant aggregate (e.g., no aggregate statistic released unless supported by ≥k≥20 tenants and ≥m distinct instruments), and **automated tests** that fail CI if a disallowed column enters a global model's feature list. A firewall you can unit-test is worth ten pages of policy.

### B11.4 Hierarchical structure — how to actually build it

**Default architecture: global prior + per-tenant adaptation.**
- Classical models: **hierarchical Bayes / partial pooling** — tenant-level coefficients shrunk toward a global mean, with shrinkage learned from data. Tenants with little data get the global prior (solves cold start); tenants with lots of data escape it. This is textbook, cheap, and interpretable, and it degrades gracefully.
- GBDT-shaped models: global model + **tenant_id-free features**, plus a small per-tenant residual/calibration model. Do **not** one-hot `tenant_id` into a global model — that's the leakage vector wearing a disguise.
- LLM adapters: **one global adapter**, per-tenant behavior via retrieval/memory/prompt state (also the only design where "delete my data" is achievable).
- Cold start: new tenant gets the global prior immediately; per-tenant components activate at a minimum-sample threshold (and until then, *say so in the UI* — "this recommendation is based on platform-wide methodology patterns, not your data").

**Per-tenant evaluation is mandatory.** A global model that improves the mean while degrading the 10th-percentile tenant is a failure. Make per-tenant regression a rollback criterion (B8.3).

### B11.5 Federated learning and differential privacy — worth it here?

**No, not now.** Evidence:

- **Cross-silo FL research is misaligned with practice.** A 2025/26 interview study of practitioners found the dominant bottlenecks are **organizational and contractual, not technical** — awareness, trust between competitors, fragmented regulation, incentive alignment, fair benefit distribution. All deployed cross-silo systems studied rely on **legal contracts, centralized coordinators, and honest participants**; PETs like homomorphic encryption "rarely get used because simpler trust-based approaches suffice." Adoption "remains trapped in pilot studies," successful projects had **external regulatory drivers**, and **profitability remains unproven**. Also: FL does not solve our actual problem. FL protects *raw data*; our risk is that **the learned function itself** transfers competitive information. A federated strategy-recommender leaks alpha just as efficiently as a centralized one.
- **DP has a real, measured utility cost.** At ε=6, fine-tuning studies report task accuracy **0.66–0.70 vs 0.71–0.79** non-DP — **5–10 points** — plus training instability. Related findings: an 8× LoRA rank increase did **not** raise total PII extraction count (though it broadened the *diversity* of exposed identifiers), and PII repetition frequency is a **weak** predictor of extraction (R²=0.237) — so "we deduplicated, we're fine" is not a defense.

**What to do instead** (cheaper, more effective, and demonstrable to a skeptical tenant):
1. **Feature firewall + k-anonymity thresholds** (B11.3), enforced in CI.
2. **Row-level tenant isolation** in the ledger with per-tenant encryption keys; global training jobs read only from a **firewalled projection view**, never the raw ledger.
3. **Train on redacted/derived features**, never raw strategy payloads. Digests, not results.
4. **Per-tenant deletion** that actually works — which requires that no tenant-specific information lives in shared weights. This is an independent, strong argument for keeping global models restricted to methodology features: **it makes GDPR/CCPA deletion architecturally true rather than aspirational.**
5. **Memorization audits** on any global model: canary-string insertion + extraction attempts, run each release.
6. **Contractual clarity** in the ToS about exactly what trains what.

**Revisit DP/FL if:** an institutional tenant makes DP a contractual requirement; or we want to publish cross-tenant *aggregate research* (in which case DP on the released aggregates — not on training — is the right tool, and is cheap).

---

## Retraining-policy decision table

Autonomy levels: **A** = fully autonomous (auto-retrain, auto-promote behind gates) · **S** = semi-autonomous (auto-retrain, auto-shadow, **human approves promotion**) · **M** = manual (human initiates and approves).

| Internal model | Label latency | Primary drift risk | Retrain **trigger** | Cadence (floor/ceiling) | Window | Promotion gate | Rollback trigger | Scope | Autonomy |
|---|---|---|---|---|---|---|---|---|---|
| **Cost predictor** (runtime/$/GPU-h) | seconds–minutes | covariate (new tools, new HW, new data sizes) | measured MAPE > threshold on rolling window; **CBPE/DLE valid here** | weekly / monthly | expanding + recency weight | MAPE + calibration of prediction intervals beats champion; no p99 underestimation regression | MAPE↑ >15% rel.; interval coverage out of band | **Global** | **A** |
| **Failure classifier** (crash/timeout/invalid-config) | minutes | covariate + tool-schema churn | measured AUC/recall drop; **schema registry bump** forces re-eval | weekly / monthly | expanding | recall@fixed-precision beats champion; no new false-block class | recall drop >5pp; false-block rate ↑ | **Global** | **A** |
| **Learning-curve extrapolator** | minutes–hours (end of trial) | covariate (new model families) | regret of early-stop decisions (measured by periodic full-run audits) | biweekly / quarterly | expanding | early-stop regret ≤ champion at equal compute savings | regret↑; any case of killing a would-be top-decile run | **Global** | **A** |
| **Step critic** (good/unnecessary/mistake/recover) | judge-time, but **needs human calibration** | evaluator drift; **co-adaptation** | κ vs human adjudication < 0.6; schema bump | monthly / quarterly | expanding, **rotate judge** | κ ≥ 0.6, position bias < 0.05, on ≥200 human-adjudicated items per family | κ < 0.55; disagreement spike with the deterministic verifier | **Global** (redacted args) | **S** |
| **Fine-tuned executor LLM / adapter** | eval-harness time | **tool-schema churn**; forgetting; safety drift | corpus half-life + harness regression vs prompted baseline | quarterly / semiannual | rolling by schema version | beats prompted frontier baseline on private harness; **zero** safety-suite regressions; ≤2pp frozen-capability regression | any safety regression; schema-perturbation score drop; cost/latency SLO breach | **Global**, one adapter | **M** |
| **Gate-outcome pre-screener** | hours–days | **concept** (what passes changes with market) | performance on **mature labels** + learning-debt rule ρ>c_churn/(c_churn+c_wait) | monthly / quarterly | rolling 6–12 mo + regime weighting | precision@k on frozen holdout; **calibration (ECE)**; no per-tenant regression | ECE breach; precision drop; screening-out rate drift | **Hierarchical** | **S** |
| **Strategy-family recommender** | **weeks–months** | **concept / alpha decay** | mature-label performance only; never drift statistics | quarterly / semiannual | rolling, regime-weighted | DSR-corrected uplift vs no-recommendation control on **forced-exploration** records; per-tenant non-regression | any negative uplift on mature labels; diversity floor breach | **Per-tenant** (global = methodology only) | **M** |
| **Experiment-proposal ranker** | days–weeks | **concept + exposure bias + entropy collapse** | DR/IPS off-policy estimate on forced-exploration slice; **entropy floor** | monthly / quarterly | rolling + mandatory replay | DR uplift CI excludes zero; **entropy ≥ floor**; Pareto non-dominated on the reward vector | entropy < floor (immediate); DR uplift CI crosses zero; proposal-diversity drop | **Hierarchical**, firewalled | **S** |
| **Cascade router** | immediate (cost/quality observable) | covariate (traffic mix), base-model updates | calibration ECE drift; cost/quality curve shift; **any base-model version change** | weekly / monthly | rolling 30–90 d | cost reduction at fixed quality floor; conformal coverage holds | quality floor breach (immediate); escalation-rate anomaly | **Global** | **A** |
| **Judge / evaluator models** | human-calibration time | **co-adaptation with the policy** | scheduled rotation, **not** performance-triggered | quarterly rotation | — | κ, position bias, test–retest ≥0.9 | κ below floor | **Global** | **M** |
| **Tenant's own trading models** | weeks–months | concept | user-configured; platform provides UPF/learning-debt signal as a **recommendation** | user choice | user choice | platform-enforced: purged CV + DSR with ledger-derived N | user-configured; platform enforces risk limits regardless | **Per-tenant** | **user-driven, platform-advised** |

**Cross-cutting rules:**
- No model is promoted on **immature labels** (< 80% label maturity) — ever.
- Every promotion writes a lineage record: artifact hash → data snapshot hash → code commit → gate results → approver.
- The **agent policy** (prompt + tool registry version + adapter + router config) is versioned and validated **as one artifact**.
- **Entropy/diversity is an SLO with a floor**, monitored for every model that selects among options.
- Agents may not modify gates, holdouts, thresholds, or the DSR correction. Access-control boundary, not a norm.

---

## Sources

**PEFT / LoRA**
- Thinking Machines Lab, *LoRA Without Regret* — https://thinkingmachines.ai/blog/lora/
- HuggingFace TRL, *LoRA Without Regret* (reproduction + recipes) — https://huggingface.co/docs/trl/en/lora_without_regret
- Spheron, *Beyond LoRA: DoRA, GaLore, PiSSA, VeRA — 2026 PEFT Decision Guide* — https://www.spheron.network/blog/peft-methods-2026-dora-galore-pissa-vera-guide/
- Kaitchup, *DoRA vs LoRA* — https://kaitchup.substack.com/p/dora-better-and-faster-than-lora
- *I Compared LoRA vs DoRA* (2026 negative replication) — https://medium.com/codetodeploy/i-compared-lora-vs-dora-dora-was-slower-and-less-accurate-heres-why-e0dd7d19627d
- Spheron, *LoRA Multi-Adapter Serving* (adapter memory, swap latency, $ economics) — https://www.spheron.network/blog/lora-multi-adapter-serving-gpu-cloud/
- vLLM LoRA Adapters docs — https://docs.vllm.ai/en/stable/features/lora/
- S-LoRA overview — https://www.emergentmind.com/topics/scalable-serving-s-lora-system
- TOPPINGS: CPU-assisted rank-aware adapter serving (USENIX ATC'25) — https://www.usenix.org/system/files/atc25-li-suyi-toppings.pdf
- AWS, *Cost-effective multi-tenant LoRA serving on SageMaker* — https://aws.amazon.com/blogs/machine-learning/efficient-and-cost-effective-multi-tenant-lora-serving-with-amazon-sagemaker/
- Stratagem, *LoRA Fine-Tuning Cost in 2026* (GPU rates, run costs) — https://www.stratagem-systems.com/blog/lora-fine-tuning-cost-analysis-2026
- Spheron, *GPU VRAM Requirements to Fine-Tune LLMs in 2026* — https://www.spheron.network/blog/gpu-vram-requirements-fine-tune-llm-2026/

**Post-training / agentic RL**
- Zylos, *RL Post-training for Tool-Using Agents: GRPO, Async RL, Reward Design in 2026* — https://zylos.ai/research/2026-04-10-rl-posttraining-tool-using-agents-grpo-async-rl
- Turing Post, *Reasoning RL in 2026: GRPO, DPO, RLVR, Agentic PO & Beyond* — https://www.turingpost.com/p/reasoning-rl-in-2026
- *Understanding and Preventing Entropy Collapse in RLVR with On-Policy Entropy Flow Optimization* — https://arxiv.org/abs/2605.11491
- *Rethinking Entropy Interventions in RLVR* — https://arxiv.org/abs/2510.10150
- awesome-RLVR — https://github.com/opendilab/awesome-RLVR
- OpenPipe ART (Agent Reinforcement Trainer) — https://github.com/openpipe/art
- ART / RULER docs — https://art.openpipe.ai/fundamentals/ruler
- SkyRL — https://github.com/novasky-ai/skyrl · SkyRL-Agent — https://arxiv.org/pdf/2511.16108
- OpenRLHF — https://arxiv.org/html/2405.11143v6
- Anyscale, *Open Source RL Libraries for LLMs* — https://www.anyscale.com/blog/open-source-rl-libraries-for-llms
- HuggingFace, *Preference Tuning LLMs with DPO Methods* (DPO/IPO/KTO) — https://huggingface.co/blog/pref-tuning
- SimPO — https://collaborate.princeton.edu/en/publications/simpo-simple-preference-optimization-with-a-reference-free-reward/
- RLAIF vs RLHF — https://arxiv.org/html/2309.00267v3
- AgentPRM (WWW'26) — https://dl.acm.org/doi/10.1145/3774904.3792551
- *A Survey of Process Reward Models* — https://arxiv.org/html/2510.08049v3
- NVIDIA, *Small Language Models are the Future of Agentic AI* — https://arxiv.org/abs/2506.02153
- NVIDIA NeMo Agent Toolkit finetuning harness — https://docs.nvidia.com/nemo/agent-toolkit/1.5/improve-workflows/finetuning/index.html

**Training data from traces / distillation**
- Thinking Machines Lab, *On-Policy Distillation* — https://thinkingmachines.ai/blog/on-policy-distillation/
- JetBrains Research, *Step Rejection Fine-Tuning* — https://blog.jetbrains.com/research/2026/06/step-rejection-fine-tuning/
- APIGen (Salesforce) — https://arxiv.org/abs/2406.18518 · https://apigen-pipeline.github.io/
- ToolACE — https://www.researchgate.net/publication/383701460_ToolACE_Winning_the_Points_of_LLM_Function_Calling
- awesome-on-policy-distillation — https://github.com/chrisliu298/awesome-on-policy-distillation
- AdaSTaR (NeurIPS 2025) — https://arxiv.org/abs/2505.16322
- Trajectory2Task (ACL 2026) — https://aclanthology.org/2026.acl-long.2037.pdf
- OpenTelemetry GenAI semantic conventions (2026) — https://greptime.com/blogs/2026-05-09-opentelemetry-genai-semantic-conventions · https://www.datadoghq.com/blog/llm-otel-semantic-convention/
- *Tool Schema Drift: The Silent Failure Mode in Production Agentic Systems* — https://dev.to/hannune/tool-schema-drift-the-silent-failure-mode-in-production-agentic-systems-49eg
- *Learning to Rewrite Tool Descriptions for Reliable LLM-Agent Tool Use* — https://arxiv.org/html/2602.20426

**Evaluation**
- BFCL (ICML 2025) — https://proceedings.mlr.press/v267/patil25a.html · Leaderboard — https://gorilla.cs.berkeley.edu/leaderboard.html
- BFCL v4 explainer + criticisms — https://benchmarkingagents.com/bfcl-function-calling/
- τ-bench — https://arxiv.org/abs/2406.12045 · τ²-bench — https://github.com/sierra-research/tau2-bench
- τ²-bench leaderboard (Sept 2026) — https://benchlm.ai/benchmarks/tau2-bench
- *Benchmarking the Benchmarks: A Validity Audit of Tool-Calling Evaluation* — https://arxiv.org/html/2607.02577v1
- *Reliability without Validity: LLM-as-a-Judge across Agreement, Consistency, Bias* — https://arxiv.org/html/2606.19544v1
- *Agreement Metrics for LLM-as-Judge Evaluation* — https://www.alphaxiv.org/abs/2606.00093
- *Judge's Verdict* — https://arxiv.org/pdf/2510.09738
- *Mechanistic Analysis of Catastrophic Forgetting During Continual Fine-tuning* — https://arxiv.org/html/2601.18699v2
- *Unforgotten Safety: Preserving Safety Alignment with Continual Learning* — https://arxiv.org/pdf/2512.10150
- *Continual Safety Alignment via Gradient-Based Sample Selection* — https://arxiv.org/html/2604.17215
- Langfuse, *AI agent evaluation: trajectory, tool calls, task completion* — https://langfuse.com/resources/engineering/ai-agent-evaluation

**Routing, cascades, prompting economics**
- Anthropic Engineering, *Advanced tool use* (tool search, programmatic tool calling, tool-use examples) — https://www.anthropic.com/engineering/advanced-tool-use
- UCCI: *Calibrated Uncertainty for Cost-Optimal LLM Cascade Routing* — https://arxiv.org/html/2605.18796
- RouteNLP: *Closed-Loop LLM Routing with Conformal Cascading* — https://arxiv.org/html/2604.23577v1
- *Conformal Cascade: Distribution-Free Accuracy Guarantees for Multi-Tier LLM Inference* — https://arxiv.org/html/2607.25018v2
- RouteLLM — https://pith.science/paper/2406.18665 · benchmarks summary — https://klymentiev.com/blog/llm-router
- GEPA — https://www.alphaxiv.org/abs/2507.19457 · DSPy GEPA — https://dspy.ai/api/optimizers/GEPA/overview/
- *Is Fine-Tuning Better Than Prompt Engineering in 2026?* (break-even volumes) — https://llm-stats.com/blog/research/fine-tuning-vs-prompt-engineering-2026
- *Prompt Caching Economics 2026* — https://agentmarketcap.ai/blog/2026/04/06/prompt-caching-economics-2026-anthropic-google-agent-cost
- *Semantic Tool Discovery for LLMs (MCP tool selection)* — https://arxiv.org/abs/2603.20313
- Gartner via AIwire, *Autopsy of an Agent Incident: patterns behind the 40% failure rate* — https://www.hpcwire.com/aiwire/2026/09/09/autopsy-of-an-agent-incident-three-patterns-behind-gartners-40-failure-rate/

**Drift detection & monitoring**
- NannyML, *Estimation of Performance of the Monitored Model* (CBPE/DLE assumptions) — https://nannyml.readthedocs.io/en/v0.13.1/how_it_works/performance_estimation.html
- Evidently, *What is concept drift, and how to detect and address it* — https://www.evidentlyai.com/ml-in-production/concept-drift
- Evidently, *What is data drift* — https://www.evidentlyai.com/ml-in-production/data-drift
- Winder.ai, *Comparison of ML model monitoring tools* — https://winder.ai/comparison-machine-learning-model-monitoring-tools-products/
- *A Framework for Evaluating and Benchmarking Concept Drift Detection Methods* — https://arxiv.org/html/2606.07789
- *Statistical Properties of the Population Stability Index* — https://files.wmich.edu/s3fs-public/attachments/u730/2022/PSIfinal.pdf
- *A Sample-size-Dependent Measure of Population Correspondence: Improving the PSI* — https://crc.business-school.ed.ac.uk/sites/crc/files/2024-01/A-Sample-size-Dependent-Measure-of-Population-Correspondence-in-Banking-Improving-the-Population-Stability-Index-PSI.pdf
- *Domain Specific Concept Drift Detectors for Financial Time Series* — https://ar5iv.labs.arxiv.org/html/2103.14079

**Retraining policy**
- *When to Retrain a Machine Learning Model* (UPF vs CARA vs drift triggers) — https://arxiv.org/pdf/2505.14903
- *Learning Debt and Cost-Sensitive Bayesian Retraining* — https://arxiv.org/html/2604.06438v1
- *When to Retrain after Drift: A Data-Only Test of Post-Drift Data Size Sufficiency* (CALIPER) — https://arxiv.org/html/2603.09024v2
- *Cost-Effective Retraining of Machine Learning Models* — https://arxiv.org/pdf/2310.04216
- *Sustainable Machine Learning Retraining* — https://www.arxiv.org/pdf/2506.13838
- MLflow, *Canary Deployment for AI Models: 2026 Guide* — https://mlflow.org/articles/what-is-canary-deployment-ai
- CalibreOS, *Safe ML Model Rollout: Canary, Shadow, Rollback* — https://www.calibreos.com/learn/mlsd-canary-deployment
- DataRobot, *MLOps Champion/Challenger Models* — https://www.datarobot.com/blog/introducing-mlops-champion-challenger-models/
- GARP, *SR 11-7 in the Age of Agentic AI* — https://www.garp.org/risk-intelligence/operational/sr-11-7-age-agentic-ai-260227

**Continual / online learning**
- *New Insights for the Stability-Plasticity Dilemma in Online Continual Learning* — https://arxiv.org/abs/2302.08741
- *Loss of plasticity in deep continual learning* (Nature/PMC) — https://pmc.ncbi.nlm.nih.gov/articles/PMC11338828/
- GCR: *Gradient Coreset Based Replay Buffer Selection* — https://arxiv.org/pdf/2111.11210
- *Flashbacks to Harmonize Stability and Plasticity in Continual Learning* — https://arxiv.org/html/2506.00477v1

**Self-improving systems, model collapse, reward hacking, off-policy**
- *Darwin Gödel Machine: Open-Ended Evolution of Self-Improving Agents* (ICLR 2026) — https://arxiv.org/html/2505.22954v3
- Shumailov et al., *AI models collapse when trained on recursively generated data* (Nature 2024) — https://www.nature.com/articles/s41586-024-07566-y
- Borji, *A Note on Shumailov et al. (2024)* — https://arxiv.org/html/2410.12954v2
- *Position: Model Collapse Does Not Mean What You Think* — https://openreview.net/pdf?id=ygfzWIGDN8
- *When Models Don't Collapse: On the Consistency of Iterative MLE* — https://arxiv.org/pdf/2505.19046
- *Is Model Collapse Inevitable? Breaking the Curse of Recursion by Accumulating Data* — https://arxiv.org/pdf/2404.01413v2
- *Fairness Feedback Loops: Training on Synthetic Data Amplifies Bias* — https://arxiv.org/html/2403.07857v1
- *When Predictions Shape Reality: Socio-Technical Synthesis of Performative Prediction* — https://arxiv.org/html/2601.04447
- Gao, Schulman, Hilton, *Scaling Laws for Reward Model Overoptimization* — https://arxiv.org/abs/2210.10760
- *Scaling Laws for Reward Model Overoptimization in Direct Alignment Algorithms* — https://arxiv.org/html/2406.02900v1
- *Reward Hacking in the Era of Large Models: Mechanisms, Emergent Misalignment, Challenges* — https://arxiv.org/html/2604.13602v1
- Lilian Weng, *Reward Hacking in Reinforcement Learning* — https://lilianweng.github.io/posts/2024-11-28-reward-hacking/
- *Don't Let Bandit Feedback Pull Continual LLM-Recommender Updates Off Target* (ABPO) — https://arxiv.org/abs/2605.18899
- *Logging Policy Design for Off-Policy Evaluation* — https://arxiv.org/html/2605.15108
- *Off-Policy Evaluation for Ranking Policies under Deterministic Logging Policies* — https://arxiv.org/html/2603.21485
- *Adaptive Doubly Robust OPE for Ranking Policies* — https://arxiv.org/html/2608.29600
- *Optimal Baseline Corrections for Off-Policy Contextual Bandits* — https://dl.acm.org/doi/fullHtml/10.1145/3640457.3688105
- Chen et al., *Top-K Off-Policy Correction for a REINFORCE Recommender System* — https://www.researchgate.net/publication/331655388_Top-K_Off-Policy_Correction_for_a_REINFORCE_Recommender_System
- *Closing the Auto-Research Loop: An AI Co-Scientist for Production Search Ranking* — https://arxiv.org/html/2603.22376v2
- *From AI for Science to Agentic Science* (survey) — https://arxiv.org/html/2508.14111v1

**Multi-tenancy, privacy, federated**
- *Research in Collaborative Learning Does Not Serve Cross-Silo Federated Learning in Practice* — https://arxiv.org/html/2510.12595
- *Unintended Memorization of Sensitive Information in Fine-Tuned Language Models* — https://arxiv.org/html/2601.17480
- *Provably Protecting Fine-Tuned LLMs from Training Data Extraction while Preserving Utility* — https://arxiv.org/html/2602.00688
- *Can Differentially Private Fine-Tuning LLMs Protect Against Privacy Attacks?* — https://link.springer.com/chapter/10.1007/978-3-031-96590-6_17
- *Assessing and Mitigating Data Memorization Risks in Fine-Tuned LLMs* — https://arxiv.org/html/2508.14062v1
- AWS, *Scaling ML inference for multi-tenant SaaS* — https://aws.amazon.com/blogs/machine-learning/how-to-scale-machine-learning-inference-for-multi-tenant-saas-use-cases
- *Bayesian Meta-Learning for Improving Generalizability of Prediction Models* — https://arxiv.org/html/2310.12595
- *Multi-Task Bayesian In-Context Learning* — https://arxiv.org/html/2606.20538

**Trading-specific**
- *AI-Driven Alpha Decay: Algorithmic Homogenization, Reflexive Signal Erosion* — https://arxiv.org/html/2605.23905
- *Artificial Intelligence in Equity and Crypto Markets: Progress, Profitability Evidence, and the Limits of Automated Investing* (Aug 2026) — https://arxiv.org/html/2609.04917v1
- Bailey & López de Prado, *The Deflated Sharpe Ratio* — https://www.davidhbailey.com/dhbpapers/deflated-sharpe.pdf · https://papers.ssrn.com/sol3/papers.cfm?abstract_id=2460551
- Bailey et al., *Statistical Overfitting and Backtest Performance* — https://sdm.lbl.gov/oapapers/ssrn-id2507040-bailey.pdf
- purged-cross-validation (purging, embargo, CPCV, DSR) — https://github.com/eslazarev/purged-cross-validation
- Hudson & Thames, *Does Meta Labeling Add to Signal Efficacy?* — https://hudsonthames.org/does-meta-labeling-add-to-signal-efficacy-triple-barrier-method/
- QuantConnect, *Why Meta-Labeling Is Not a Silver Bullet* — https://www.quantconnect.com/forum/discussion/14706/why-meta-labeling-is-not-a-silver-bullet/
- *Microstructure alpha: hierarchical learning and cross-asset transfer in cryptocurrency markets* — https://www.frontiersin.org/journals/blockchain/articles/10.3389/fbloc.2026.1811716/full
- *Regime switching forecasting for cryptocurrencies* — https://link.springer.com/article/10.1007/s42521-024-00123-2
- BlackRock crowding warning for hedge funds (2026) — https://hedgeco.net/news/04/2026/blackrock-issues-crowding-warning-for-hedge-funds.html
- BFI, *Financial Machine Learning* — https://bfi.uchicago.edu/wp-content/uploads/2023/07/BFI_WP_2023-100.pdf

**Learning-curve extrapolation (for the internal extrapolator model)**
- LC-PFN: *Efficient Bayesian Learning Curve Extrapolation* (NeurIPS 2023) — https://arxiv.org/pdf/2310.20447
- AutoML.org, *HPO speedup with learning curve extrapolation* — https://www.automl.org/hpo-overview/hpo-research/hpo-speedup-with-learning-curve-extrapolation/
- *Architecture-Aware Learning Curve Extrapolation via Graph ODE* — https://pith.science/paper/2412.15554
