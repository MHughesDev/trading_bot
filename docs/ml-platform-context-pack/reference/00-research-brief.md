# Model Development Platform — Research Brief & Design Decisions
**For:** Mason · **Date:** 2026-09-12 · **Status:** Research complete (4 deep tracks), decisions proposed, awaiting your calls before I write the unified spec.

---

## 0. What this document is

Your prompt had six overlapping asks tangled together. This document collapses them into **one charter**, reports what the research actually found (some of it contradicts the original prompt), and ends with the decisions I need from you.

Underlying research (available on request, ~39,000 words total):
- `01-mlops-infra.md` — orchestration, tracking, versioning, point-in-time data, GPU/resource management
- `02-hpo-search.md` — hyperparameter search, multi-fidelity, meta-learning, statistically valid comparison
- `03-financial-ml.md` — backtest validity, leakage, promotion gates, multiple-testing control
- `04-agentic-ml.md` — agent architecture, memory, tool design, verifiers, budget control

Sections 3.5–3.8 (data substrate, vector spaces, internal models, retraining policy) are written from my own knowledge rather than a dedicated deep-research pass — flagged inline where confidence is lower.

---

## 1. The Unified Charter
*This replaces your jumbled prompt. One thing.*

> **Build a model-development environment — not a training button.**
>
> The system's job is to let a human or an authorized AI agent pose an objective ("beat this benchmark on this universe under these constraints"), and then run a **long-horizon, multi-session campaign** of controlled experiments that converges on a model or strategy which is *demonstrably* better than its baseline — where "demonstrably" means statistically defensible under the true number of trials the system has run, not a single flattering backtest.
>
> Everything the campaign produces — every configuration, every run including the failures, every metric, checkpoint, artifact, decision and rejection — is written to an **immutable, append-only Trial Ledger with logged decision propensities**. That ledger is not an audit byproduct. It is the platform's primary asset: the training corpus for a family of internal models that progressively take over the platform's own judgment — which configs to try, which candidates will fail the gates, which strategy families suit which assets, when to stop spending.
>
> Five design commitments, in priority order:
>
> 1. **The data model is the product.** Bitemporal market data, content-addressed datasets, a fixated trial ledger, and derived vector spaces over assets, regimes, configs and outcomes. Every capability above is a query against these. Get this wrong and nothing else can be retrofitted.
> 2. **Honest trial accounting is the core discipline.** An agent is a multiple-testing machine. The platform counts trials, deflates performance accordingly, and enforces it in infrastructure — not in researcher discretion.
> 3. **Humans and agents use the same granular surface.** One typed API; the UI is a client of it, the agent is a client of it. No capability exists for one and not the other. Agents get ~15 always-on tools + a searchable tail, not one `train_model()`.
> 4. **Optimization is defined relative to explicit multi-objective constraints.** Never "lowest loss." Selection is a vector: net-of-cost risk-adjusted return, calibration, stability across seeds and regimes, capacity, inference cost, explainability, drawdown.
> 5. **The platform learns about itself.** Internal models trained on the ledger, each shipping first as a heuristic and only replaced when the learned version beats that heuristic on a held-out slice of the ledger.

---

## 2. The single most important finding

**Your bottleneck is not search. It is selection.**

From the financial-ML research: with 10,000 trials over 5 years of data, the Sharpe ratio *expected from pure noise* is **1.92**. A human quant runs 10²–10³ configs a year. Your agent will run that before lunch. Every heuristic the industry uses ("Sharpe > 1", "it held up out-of-sample", "it survived 2008") was calibrated for human trial rates and is meaningless at agent trial rates.

Corroborating this from the agentic-ML side: Meta's AIRA study found that on a top ML-engineering agent, **selecting the final submission by test score instead of validation score was worth 9–13 percentage points** — larger than any search-algorithm improvement they measured. And submitting top-3 instead of top-1 recovered ~10%.

So the 11-step orchestration loop in your original prompt is right in shape but has a hole in it: **it has no step that counts trials.** Current published agentic-quant systems (e.g. QuantEvolve) run evolutionary search over thousands of candidates against a fixed split with *no* trial accounting and *no* multiple-testing correction. That's the gap to build into.

**Second-most-important finding, also counterintuitive:** search *policy* barely matters until your *operators* are good. AIRA's ablation showed MCTS and evolutionary search gave **zero gain** over greedy iteration when paired with a generic operator set; tuning the exploration constant moved nothing. Only after the edit operators were improved did the search policy start paying. Budget your engineering into "what changes can the agent make, and how surgically" — not into a clever tree-search algorithm.

---

## 3. Findings and decisions, by area

### 3.1 Orchestration & infrastructure

**Finding: static-DAG engines are structurally wrong here.** Airflow, Argo Workflows and Kubeflow Pipelines compile a DAG then walk it. An AI agent operator makes the DAG *dynamically unknown* — it decides step N+1 based on step N's result. You need durable execution (event-sourced replay), not DAG compilation.

**Decision: Temporal (control plane) + Ray (compute plane).** Split them. Temporal owns the campaign's durable state machine — resumable, cancellable, retryable across process death and deploys. Ray owns the actual training/backtest execution. Temporal's 2026 releases close the ML gaps specifically:
- *Worker Versioning GA* — an agent editing pipeline code no longer breaks in-flight workflows
- *Task Queue Priority & Fairness GA* — research sweeps cannot starve a production retrain
- *External Payload Storage* — you will hit payload limits; this handles it
- *Principal Attribution* — non-spoofable "who initiated this." Under audit, this is the difference between "a run happened" and "agent X, acting for user Y, ran it."

**Credible alternative: Flyte 2** (GA 2026-08-04). The `@workflow` DSL is gone — tasks call tasks in plain Python, `asyncio.gather()` distributes real compute — plus content-addressed caching, retries that *modify the resource request* rather than just replaying, and first-class human-in-the-loop. It's the more elegant single-system answer. Risk: GA is one month old, 2 core contributors, single commercial sponsor. I'd pick Temporal+Ray for a system meant to run for years.

**Market changes that affect vendor risk:** Prefect acquired Dagster Labs (2026-07). Anaconda acquired Outerbounds/Metaflow (2026-04). Determined AI is absorbed into HPE MLDE — don't build on it. Kubeflow *Pipelines* is losing ground but Kubeflow *Trainer v2* is good standalone (note: v2.2 **removed `ElasticPolicy`** — use torchrun elastic).

**Tracking: MLflow 3.x.** Its `LoggedModel` entity is the right data model: models are first-class, and metrics attach to `(model, dataset_digest)` pairs — which is exactly the question a validator asks ("every model with Sharpe ≥ 1.5 *on this dataset digest*").

**The universal failure mode, and it will bite you:** the tracking database becomes a time-series database it was never designed to be. Latency decays ms → seconds → minutes, and worse, it becomes a *deployment dependency*. W&B publishes the only real calibration numbers: 10k runs/project, 500k steps/run, **100k distinct metric keys**. Cardinality kills, not volume.

> **Quant-specific trap:** logging per-instrument metrics (3,000 tickers × N runs) blows that ceiling instantly. **Rule: per-entity results are written as Parquet artifacts, never as metric series.** Metric series are for scalars you'd actually plot as a curve.

**The highest-damage, lowest-visibility failure: snapshot expiry.** Reproducibility will live on one integer — the Iceberg snapshot ID stored in run metadata — and `expire_snapshots` deletes the files behind it on a maintenance schedule. **Policy: every registered model version gets an Iceberg tag created transactionally at registration** (kilobytes, survives GC), audited monthly.

**Resource management:** Kueue + JobSet over Volcano (admission-gate model, no second scheduler). Note they're concurrent layers — direct Volcano submission silently bypasses quota. MIG slicing (`1g.10gb`) is under-used in quant and is the biggest available utilization win for small factor models. **Budget enforcement must happen at Kueue admission time**, and an agent operator needs a mandatory `max_gpu_hours` with **no default value**.

**Live UI streaming — never one connection per run.** Batched ingest → Parquet (system of record) → NATS JetStream → stateless SSE gateways, with snapshot-from-OLAP + SSE deltas. Connections then scale with *viewers*, not runs. Telemetry uses a bounded drop-oldest queue: a monitoring system that can stall a 3-day GPU job is worse than no monitoring.

**Governance moved under you, twice.** SR 11-7 was *replaced* 2026-04-17 by a risk-based framework, effective immediately — demanding column-level lineage, versioned feature definitions, and validator sign-off bound to specific model versions. Its load-bearing requirement: evidence must be a **byproduct of how models are built**, not reconstructed afterward. Meaning: if a side door exists that can train and register outside the workflow, you will find out during an exam that it was used. (Separately, EU AI Act Annex III high-risk was delayed from 2026-08-02 to 2027-12-02 — schedule relief, not exemption.)

---

### 3.2 Search & optimization

**Algorithm choice is mostly determined by three numbers: budget B, dimensionality D, parallelism P.**

| Regime | Use |
|---|---|
| B < ~10 full-training-equivalents | Don't search. Run a **meta-learned portfolio** of known-good configs. |
| B 10–30, expensive runs | PriorBand or ifBO (learning-curve BO) |
| B 30–200, mixed/conditional space | TPE (Optuna) or SMAC3 |
| B 30–200, continuous | GP-BO |
| B > 200, high D | CMA-ES (continuous) / DEHB (discrete + multi-fidelity) |
| P ≥ 8 with learning curves | ASHA underneath whatever searcher you pick |

**A 2024 correction worth knowing:** high-dimensional GP-BO was never broken — the *lengthscale prior* was. Scaling the LogNormal prior by √D makes vanilla GP-BO match or beat SAASBO/TuRBO/ALEBO up to 6,392 dimensions. Fix the prior before reaching for trust regions or embeddings.

**Random search is a live competitor, not a strawman.** In LLMSYS-HPOBench (2026; 364k configs, ~95k GPU-hours), random search beat Hyperband, BOHB, SMAC *and* HEBO on two of seven systems. **Recommendation: always run a random arm at 10–20% of budget as a surrogate-misspecification canary.** This doubles as your exploration quota for off-policy learning (see §3.5).

**Sharpest production hazard: ASHA × noisy validation metrics.** Promoting on a last-step metric whose noise is comparable to inter-config spread is near-random, and creates a compounding winner's-curse bias. Fixes by ROI:
1. Smooth the rung metric (EMA or last-3 mean)
2. Raise `grace_period` past the learning-curve crossing region
3. PASHA-style noise-estimated soft ranking (ε = 90th pct of rank-swap gaps) — 2.3–3.4× speedups on NAS-Bench-201, 15.5× on WMT
4. Optuna `WilcoxonPruner` when replicates exist
5. **Re-evaluate the top-3 with fresh seeds before crowning an incumbent** — cheap, catches most of the damage

**Learning-curve stopping should be posterior-based.** LC-PFN does Bayesian curve extrapolation in a single forward pass (>10,000× faster than MCMC parametric ensembles); FT-PFN (inside ifBO) conditions on the config too. Stopping rule: **stop when P(final > incumbent + δ_practical | partial curve) < 0.05.**

**Reusing your own experiment DB: portfolios beat clever transfer.** Greedy submodular portfolio construction is provably within 1−1/e of optimal. Auto-sklearn 2.0's 32-pipeline portfolio cut normalized error 16.21 → 3.58 vs v1.0. TabRepo's 15-config portfolio beat full AutoGluon at lower latency.
> **The critical enabler is a storage decision: store raw per-fold validation predictions.** Then ensembles can be *simulated by lookup* with zero retraining. This single schema choice is worth more than any meta-learning algorithm. It's in §3.5's schema.

**On task representation** (directly relevant to your asset-vector idea): Auto-sklearn 2.0 deliberately uses only `n_rows` and `n_features`, and a 2026 study found **essentially no statistical meta-feature survives FDR-controlled screening** for routing between model families. Use **performance-based landmarkers** (how do 5 cheap reference configs score on this task?) plus a text/LLM embedding of the task description — not hand-engineered meta-features. This is an important caution for §3.6.

**Tabular foundation models have displaced classic AutoML as single models.** TabArena 2026: TabPFN-3 Elo ≈1673, statistically tied with a 4-hour AutoGluon ensemble, vs ≈1433 for tuned LightGBM. Handles 1M rows / 200 features, extends to time-series, distills into MLPs/trees for cheap serving. **Licensing is a hard gate:** TabPFN ≥2.5 requires a commercial license; Mitra / TabICL / TabDPT / Nori are permissive. Production answer is a foundation model *inside* an AutoGluon-style portfolio+stacking pipeline.

**Separate the search loop from the decision loop.** Search-loop validation scores are inadmissible as performance estimates. Protocol for declaring A better than B:
- Randomize *all* nuisance sources (Bouthillier et al.: their biased estimator matched the ideal one at **51× less compute**)
- ~29 paired replicates
- Declare A > B iff `P(A>B) ≥ 0.75` **and** `P(A>B) − CI_lower > 0.5`
- Holm correction for promotion decisions; Benjamini-Hochberg / e-BH for screening
- Because the agent peeks continuously: **betting-based anytime-valid confidence sequences** plus a ROPE of ±δ_practical
- **Register `δ_practical` before every campaign.** Most stopping and promotion questions are literally unanswerable without it. This becomes a required field in the campaign object.

**Cheapest remaining gain is post-hoc:** greedy model soup over the sweep's own checkpoints (free at inference, ingredients already exist); greedy ensemble selection (Caruana) beats unconstrained weight optimization on threshold-dependent metrics because its implicit sparsity prevents validation overfitting. Per CalArena (~2000 experiments): smooth logistic-family calibrators win for binary; binning methods lose despite flattering ECE. **Order: ensemble → calibrate on a dedicated split → set threshold in closed form from the cost matrix.** Not tuned.

---

### 3.3 Financial validity — the gate stack

This is where your original prompt's list of safeguards becomes executable rules.

**Embargo rule to hard-code:**
```
E = label_horizon + max_feature_lookback + settlement_lag     (minimum: h + 1 bar)
```
López de Prado's percentage embargo is a guess; this version is derived from your pipeline. **Purge on `t1` (label end), not `t0`** — purging on `t0` silently under-purges by the full horizon. This is a common, invisible bug.

**Three leakage tests that do most of the work** (all implementable, all belong in CI):
1. **Causal access guard** — wrap the dataframe in a proxy that raises on any read of rows past the decision timestamp; run the whole pipeline under it.
2. **Random-label test** — permuted labels must produce Sharpe ≈ 0. This validates the *harness*, not just the features. Run it on the platform itself, continuously.
3. **Snapshot reproducibility test** — re-running on a 6-month-old data snapshot must reproduce identical historical positions. Catches retroactive adjusted-price mutation, the classic corporate-action trap.
Plus a soft flag: `CV_Sharpe − WalkForward_Sharpe > 1.0` → suspect overlapping-label leakage.

**Promotion gate stack, cheapest-first** (each is a config value, not a hardcode):

| # | Gate | Threshold |
|---|---|---|
| 1 | Pre-registration hash-locked before first backtest | required |
| 2 | Leakage suite | 100% pass |
| 3 | Cost sensitivity | break-even cost multiple ≥ 3× |
| 4 | Capacity | deploy ≤ 20% of capacity-at-half-Sharpe; ≤5% ADV soft / 10% hard |
| 5 | CPCV | **5th-percentile** path Sharpe > 0 (not the mean) |
| 6 | PBO (CSCV) | < 0.20 |
| 7 | Deflated Sharpe | ≥ 0.95 on **platform-counted** N_eff |
| 8 | Minimum backtest length | SR ≥ 1.5·√(2 ln N_eff / y); ≥5yr; ≥300 independent events |
| 9 | Factor attribution | net alpha t-stat ≥ 3.0 vs FF5+MOM+STR+BAB+QMJ (Newey-West), factor R² < 0.7 |
| 10 | Regime coverage | no single regime > 50% of PnL |
| 11 | Perturbation robustness | no parameter cliff; no single name > 20% of PnL |
| 12 | Stationary bootstrap | 5th-pct Sharpe > 0 |
| 13 | **Romano-Wolf stepdown** vs the full candidate family | p < 0.05 |
| 14 | Paper/shadow → capital ramp | 10/25/50/100% |

**Where I'd push back on the canonical framework** (worth your attention — these are not standard takes):
- **CPCV is not a backtest.** Its paths train on data *after* some test blocks. It measures model-class generalization, not what a trader could have earned, and it assumes the stationarity it doesn't test. The 2024 result showing CPCV beating walk-forward is on *synthetic* data with mild parametric non-stationarity. **Require both CPCV and a strictly causal walk-forward. Promote on neither alone.**
- **DSR is exquisitely sensitive to N, which nobody knows.** Under-count → rubber stamp. Over-count → nothing ever passes. This is why N must be platform-computed via correlation clustering of stored return series, never self-reported.
- **PBO is a property of a selection procedure over a family**, meaningless for a single candidate and weak over a correlated parameter sweep.
- **Romano-Wolf stepdown is the most defensible single gate**, because it uses the *actual correlation structure* of your trials rather than a guessed N_eff. A 200-point sweep over one idea is correctly not penalized as 200 independent tests.

**Expectation deflation for sizing:** `SR_expected = min(DSR-implied SR, 0.5 × backtest SR)`. Harvey-Liu put the multiple-testing haircut alone at ~60% for SR 0.75 with 200 trials — and the haircut is **non-linear**: SR < 0.4 typically loses >50%, SR > 1.0 loses ≤25%. The "just halve it" rule of thumb is wrong in both directions. Use BHY (FDR), not Bonferroni.

**Paper trading tests the pipeline, not the alpha.** Over 60 days the standard error of Sharpe is ~2.0 — it cannot distinguish 0 from 2. **Gate on process:** signal reproduction ≥99% match vs backtest (failures here are almost always point-in-time data bugs and are the most valuable output of the whole exercise), slippage ≤1.5× modelled, turnover within ±20%. Reject on Sharpe only if catastrophically negative (sign error).

**Pre-register kill criteria at deployment time.** Post-hoc kill decisions always arrive too late, because the drawdown that should trigger them always arrives with a plausible excuse. Use SPRT/CUSUM on live Sharpe against `H₀: SR = SR_expected`, plus MDD > 1.5× backtest MDD.

**The gate stack's own failure mode:** agents will gate-hack. Mitigations: the trial counter must include gate-*failing* runs; some gates should be stochastic with a platform-held seed; the sealed holdout must never inform gate tuning. Expect single-digit-percent pass rates — that's intended. The real risk is people routing around the registry, so **make the registry the only path to capital.**

---

### 3.4 Agent architecture & tooling

**The architecture has converged, and it isn't about the search algorithm.** Every 2025–26 SOTA ML-engineering agent is the same shape: *tree search over solution nodes, typed edit operators on the edges, scoped memory injected into the reasoning trace, deterministic sandboxed evaluation.* MLE-bench went 16.9% (AIDE+o1-preview, 2024) → 29.3% (ML-Master v1) → **56.44%** (ML-Master 2.0, 24h budget). MLE-STAR hits 63% on MLE-bench-Lite.

**Invest in operators, not search policy** (see §2). Specifically, what MLE-STAR does that works: run an **ablation to find the highest-leverage pipeline block, then edit only that block.** Plus prompt-adaptive complexity cues conditioned on sibling count, crossover between solution branches, and *operator-scoped memory* — siblings for draft/improve (diversity), ancestors for debug (anti-oscillation).

**Skill libraries don't work. Tiered memory does.** SkillEvolBench (180 tasks, 10 model configs): raw-trajectory reuse frequently *beats* distilled skills; static curated skills underperformed the no-skill baseline by −2.44 points; multi-skill composition hit 0% in some environments. **Skip Voyager-style skill libraries.** What replicates:
- ML-Master 2.0's 3-tier cache: L1 run feedback / L2 tactical / L3 cross-project heuristics, with a hard **~4K-token injection cap**
- Dual-process episodic buffer + consolidated semantic profile — 100% accuracy at 100k messages where full-context crashed at ~10k. Notably **RAG alone scored 0% on "current state" queries** — you need both structured state and retrieval.
- Upgrading the consolidator model from GPT-4o-mini to GPT-4o improved accuracy by 0.07%. **Spend on the extraction schema, not on a bigger consolidator.**
- Storage: Postgres as system of record for runs/metrics/lineage; vector index **only over natural-language insight records**. **Never embed numeric results** — numbers belong in SQL.

**Budget must be harness-enforced, not prompted.** BAGEN found task performance and budget-awareness are essentially uncorrelated (r=0.35); frontier agents are consistently over-optimistic and "continue spending on tasks unlikely to succeed instead of alerting the user early"; interval calibration stayed <47% even after training. But SFT+RL early-stopping saved 28–64% of tokens on failed trajectories. So:
- Hierarchical budgets checked **before dispatch**
- Marginal-value stopping rules on subtrees
- **Idempotency keys + config-hash dedup** — kills a surprising share of spend outright
- `dry_run` and `estimate_cost` on every expensive tool
- Errors that **refuse with alternatives** so the agent replans instead of being silently killed

**Approval fatigue is measured: users approve ~93% of permission prompts.** So per-action approval is security theater. **Use envelopes instead:** define a sandboxed envelope where the agent is free, and gate only four things — (1) spend above threshold, (2) promotion to paper/live, (3) embargoed/sealed-holdout access, (4) L3 memory writes. Separate reasoning from enforcement: *the model proposes, the harness enforces.* Add a hash-chained audit trail with `record_phase: pre` so you can prove denials prevented actions — which doubles as regulatory research provenance.

**Tool design has hard numbers now:**
- Tool Search: 77K → 8.7K tokens of definitions, MCP eval accuracy 79.5% → 88.1%
- Programmatic Tool Calling: −37% tokens, GAIA 46.5% → 51.2% — exactly right for "run 200 backtests, return top 5"
- Tool Use Examples: complex-parameter accuracy 72% → 90%
- OpenAI's guidance: **<20 functions visible per turn**
- Cross-cutting conventions worth standardizing: `response_format: concise|detailed` (~67% token savings), `dry_run`, `idempotency_key`, structured errors carrying `nearest_valid`/`suggested_fix`, self-describing truncation with continuation hints, **human-readable slugs instead of UUIDs**

**Verification: build a small domain PRM; don't trust LLM judges.** DataPRM (4B params, environment-grounded, ternary reward — 1.0 correct / 0.5 *correctable* / 0.0 irrecoverable) beats 72B generic PRMs and self-rewarding with a 235B model: +7–11% on Best-of-N. Meanwhile LLM-as-judge raw agreement overstates chance-corrected κ by 33.8–41.2 points (an "85% agreement" judge is κ≈0.48). **Use judges for triage only** (novelty, leakage plausibility, failure recoverability), with periodic human κ calibration. Never to decide whether a strategy is good — that's what the gate stack is for.

**Multi-agent: pay for it only when the work is genuinely parallel.** Anthropic's orchestrator/worker reported +90.2% — at ~15× token cost, with token usage alone explaining 80% of the variance. Under *equal* thinking-token budgets, single-agent matched or beat every multi-agent variant; multi-agent only won under 70% context noise. MAST's 1,600-trace study puts the dominant fixable failure category in **verification** — so make verification *code*, not conversation.
> One free reliability win: if you do use a critic, present artifacts as **external tool content**, not as the agent's own thoughts. The Self-Correction Illusion result shows relabeling an identical claim from `<thought>` to a tool/user/memory role lifts correction rates by **23–93 points**.

---

### 3.5 The data substrate — "data is king," made concrete

Your stated design philosophy is that data and the mathematical manipulation of data comes first, and the system is designed around it. Here is that, made specific.

#### Four planes, bottom-up. Each one is impossible to retrofit.

**L0 — Market data plane (bitemporal).**
Every off-the-shelf feature store models exactly one time dimension: event time. Finance needs **two**:
```
event_time      -- when the fact was true in the world
knowledge_time  -- when your system could first have known it
```
Without `knowledge_time` you train on restated fundamentals, revised economic releases, and survivorship-cleaned universes that did not exist at decision time. **This is the single most expensive thing to retrofit** — it isn't a column you add later, because the historical knowledge times are gone.

- Store on Apache **Iceberg** (v3 adds `timestamp_ns`, explicitly motivated by trading; microsecond truncation silently reorders microstructure events).
- Build the **as-of join** yourself. Use Feast only for the online-store abstraction — Feast's own docs note it doesn't run transformations.
- Steal **Chronon's online/offline consistency measurement**: log online feature fetches, backfill those exact keys/timestamps, diff them. This converts "we believe there's no training/serving skew" into a monitored number.
- Universe membership, corporate actions, borrow availability, halts, tick size and ADV are all bitemporal facts, not static metadata.

**L1 — Dataset & feature plane (content-addressed).**
A dataset is never a file path. It is a **spec** that hashes to an ID:
```
dataset_id = H(universe_spec, date_range, frequency, feature_set_id,
               label_spec, split_spec, iceberg_snapshot_id, transform_code_hash)
```
Two runs with the same `dataset_id` used byte-identical data — that's the guarantee. A feature definition is versioned code plus a declared lookback plus a declared `knowledge_time` requirement; the lookback feeds the embargo formula automatically (§3.3). Splits are objects, not slices: a walk-forward split with 12 folds is one `split_spec` with a deterministic expansion.

**L2 — The Trial Ledger (immutable, append-only, propensity-logged).**
This is the asset. Requirements, each of which exists for a reason:

1. **The ledger row is written BEFORE results are returned.** Not after. An agent (or a human) cannot run a backtest that doesn't get counted. There is no opt-out flag.
2. **Every run is logged** — crashed, cancelled, abandoned, "just exploring," gate-failed. Trial accounting is worthless if only interesting runs are recorded, and the failures are the training data for your failure models.
3. **Full out-of-sample return series are persisted**, not just summary metrics. PBO, SPA and Romano-Wolf all need the full trial matrix. Summary stats are a lossy projection you can always recompute; the series you cannot.
4. **Raw per-fold validation predictions are persisted.** (TabRepo's finding, §3.2.) This lets you simulate ensembles by lookup, with zero retraining, forever.
5. **`N_eff` is computed by the platform** via correlation clustering of the stored return series. Never self-reported.
6. **Pre-registration is hash-locked** before the first backtest: hypothesis, feature set, universe, horizon, cost model, δ_practical, promotion thresholds.
7. **The sealed holdout is rate-limited by ledger**, one evaluation per strategy, ever. Enforced at the data tool, not in a prompt.
8. **Decision propensities are logged.** ← see below. This is the non-obvious one.

**L3 — Derived knowledge plane.** Vector spaces, learned surrogates, agent insight records, internal model artifacts. Everything here is *recomputable* from L0–L2, which is what makes it safe to iterate on.

#### "Fixating" the data — your word, made into a mechanism

Fixation = **a written fact can never change; corrections are new facts that supersede old ones.** Four mechanisms stacked:
1. **Content addressing** — artifacts are stored under the hash of their bytes. Identical content deduplicates; different content cannot share an ID.
2. **Hash-chained ledger** — each trial row includes the hash of the previous row. Tampering with history is detectable, not just discouraged.
3. **Iceberg tags at registration** — as in §3.1, a transactional tag per registered model version so GC can never orphan a reproducibility reference.
4. **Bitemporal supersession** — a corrected trial row is a NEW row with `supersedes: <prev_id>` and its own `knowledge_time`. The original stays. This matters because it means a meta-model trained six months ago can still be explained: you can reconstruct exactly what the ledger looked like at the moment it was trained.

#### The decision that makes internal models possible at all: **log propensities**

This is the single most important data-engineering choice in this document, and it costs almost nothing to implement if you do it on day one and is impossible to add later.

Every time the platform (or an agent) *chooses* to run configuration `c` out of a candidate set `C`, log:
```
candidate_set_hash, chosen_config_id, propensity p(c | context, policy_version),
policy_id, policy_version, exploration_flag
```
Why: without it, your trial ledger is a **biased, self-selected sample**. Configs only got run because something thought they were promising. A model trained naively on that log learns "everything works," because the bad ideas were never tried. With logged propensities, the ledger becomes a valid **logged-bandit-feedback dataset**, and you can:
- Train unbiased models via inverse-propensity weighting / doubly-robust estimators
- **Evaluate a proposed new experiment-selection policy against the historical log without running it** (off-policy evaluation) — which means you can improve the orchestrator's judgment without spending GPU-hours to test each idea
- Detect when your policy has collapsed into exploitation

Paired with this: **a mandatory exploration quota.** The random-search arm from §3.2 (10–20% of budget) isn't only a misspecification canary — it is your source of unbiased, high-propensity-diversity samples. If the selector only ever proposes what it already believes is good, the ledger degenerates and the internal models stop improving. **Exploration is a data-collection obligation, not wasted compute.** Build it as a hard floor the agent cannot lower.

#### Storage architecture

| Layer | Store | Why |
|---|---|---|
| System of record: trials, lineage, registry, permissions, approvals | **Postgres** | transactions, FK integrity, audit |
| Per-step metrics, per-bar PnL, per-fold predictions, per-instrument results | **Parquet on object storage, Iceberg-catalogued** | cardinality escape hatch (§3.1) |
| Analytical queries ("compare 500 runs × 12 metrics", "all trials on assets like X") | **DuckDB** (single-node) → **ClickHouse** (when concurrent analysts > ~10) | columnar scan speed |
| Live metric stream to UI | **NATS JetStream → SSE** | scales with viewers, not runs |
| Vector similarity | **pgvector in the same Postgres** until >1M vectors | see §3.6 — do not add a vector DB early |
| Artifacts, checkpoints | content-addressed object store | dedup, immutability |

**Explicit anti-recommendation:** do not put per-step metrics in Postgres, do not put trial metadata only in Parquet, and do not adopt a separate vector database before you have a million vectors. Each of those is a common, expensive mistake.

---

### 3.6 Vector spaces — your asset-embedding idea, evaluated honestly

You asked to build vector representations of assets from pricing (OHLCV), spot "pricing groupings," and use vector/matrix mathematics to make better decisions about new algorithms, informed by accumulated backtest data. **This is a good instinct and it has a precise, defensible formulation.** It also has a well-known way of going wrong, which I want to flag before you build it.

#### Five spaces

| Space | Contents | Built from |
|---|---|---|
| **A** — Asset | one vector per (asset, time-window) | price/volume statistics + metadata |
| **R** — Regime | one vector per (market, time-window) | cross-sectional market state |
| **S** — Strategy/config | one vector per configuration | structured encoding of the config tree |
| **O** — Outcome | one vector per trial | the multi-objective result vector, NOT a scalar |
| **F** — Failure | one vector per failed/rejected trial | diagnostic signature |

**Critical: `O` is a vector, not a number.** `[net_Sharpe, DSR, PBO, max_DD, turnover, capacity, cost_breakeven_multiple, regime_PnL_dispersion, seed_stability, inference_latency, factor_R²]`. Your original prompt says "never call a model optimal from one number" — this is how you enforce that structurally: the outcome *type* in the database makes it impossible to store a single score.

#### The central object: a sparse Outcome Tensor

```
T[asset_cluster, regime, strategy_family, config, metric] → value
```
Almost every meta-question you asked is a query against `T`:
- *"What worked on assets like this?"* → kNN in **A** → slice `T` → aggregate over **O**
- *"Is this config likely to fail the gates?"* → supervised model on (**S**,**A**,**R**) → gate-pass label
- *"Which family should I try next on this asset?"* → recommendation over `T`

That last one has a precise analogy: **assets are users, configs are items, outcomes are ratings.** It is a matrix/tensor completion problem — the same mathematics as collaborative filtering. Non-negative tensor factorization or a low-rank factorization with side information (asset features, config features) gives you cold-start recommendations for an asset you've barely tested.

**But the missingness is NOT at random**, and this is where naive matrix factorization produces confident nonsense. Two distortions:
1. **Selection bias** — configs were run because something liked them. Fixed by the logged propensities in §3.5 (IPW-weighted factorization).
2. **Censoring** — runs killed early by ASHA have *truncated* outcomes, not bad ones. Treat with survival analysis / censored regression, not by imputing zero or dropping them.

Get those two wrong and the tensor will cheerfully tell you that everything works.

#### How to actually build **A** (and the caution)

**Start with a hand-crafted, interpretable fingerprint, not a learned embedding.** From §3.2: a 2026 study found essentially no statistical meta-feature survives FDR-controlled screening for model-family routing, and Auto-sklearn 2.0 deliberately uses almost none. Learned embeddings of price series are also notoriously unstable across regimes. So:

**Tier 1 — ship this first (~40–60 dims, all computable from OHLCV + reference data):**
realized vol (multi-horizon), vol-of-vol, return skew/kurtosis, tail index, autocorrelation at lags 1/5/21, variance ratio, Hurst exponent, jump intensity, downside/upside vol ratio, max drawdown frequency, Amihud illiquidity, turnover/ADV, spread proxy (Corwin-Schultz), price level and tick-size regime, volume profile shape, overnight-vs-intraday variance split, factor betas (market/size/value/momentum/quality), sector/industry one-hot, options-implied vol if available.

**Tier 2 — performance landmarkers (this is the part most people skip and it's the strongest signal):**
run 5–8 cheap, fixed reference strategies on the asset (e.g. simple momentum, mean-reversion, vol-carry, a small GBDT on standard features). *Their scores are the asset's coordinates.* Per §3.2, performance-based landmarkers consistently beat statistical meta-features for exactly this routing task. This costs a few CPU-minutes per asset and is the highest-value component.

**Tier 3 — learned encoder, later and only if it beats Tier 1+2 on a probing task:**
self-supervised contrastive encoder (TS2Vec-style) or embeddings extracted from a time-series foundation model. Gate its adoption on measurable downstream lift, not on it being more modern.

**Every asset vector carries a validity interval.** An asset's fingerprint in 2019 is not its fingerprint in 2026. Schema:
```sql
asset_embedding(
  asset_id, embedding_version, valid_from, valid_to,
  knowledge_time,              -- when it could have been computed
  window_spec,                 -- e.g. trailing 252d
  tier1 vector(60), tier2 vector(8), tier3 vector(128) NULL,
  quality_flags jsonb,         -- insufficient history, stale, corp-action affected
  PRIMARY KEY (asset_id, embedding_version, valid_from)
)
```
`knowledge_time` here is not decoration: **a retrieval that uses an asset vector computed with future data is meta-level leakage**, and it's easy to do by accident.

#### Mathematical cautions worth respecting

- **Metric choice matters more than dimensionality.** For correlation-structured data, use correlation distance `√(2(1−ρ))` or Mahalanobis with a *shrunk* covariance — raw Euclidean on unnormalized features is meaningless.
- **Denoise correlation matrices** before clustering: Marchenko-Pastur eigenvalue filtering, detoning (remove the market mode), Ledoit-Wolf shrinkage. An unfiltered sample correlation matrix on N assets with T observations where T/N is small is mostly noise, and clusters found in it are mostly noise.
- **Distance concentration.** Past ~20–30 effective dimensions, nearest and farthest neighbors converge in distance. Whiten and reduce (PCA to retained variance) before kNN. Don't retrieve in a 200-dim raw space.
- **Validate the space before trusting it.** Three probing tests: (a) retrieval precision — do kNN assets share known sector/factor structure? (b) downstream lift — does config transfer between kNN assets beat transfer between random assets? (c) temporal stability — how far does an asset drift per year, and does that correlate with regime change? If (b) fails, the embedding is decorative and you should not build on it.
- **At your likely scale (a few thousand assets), exact kNN in Postgres/NumPy is faster and simpler than any vector database.** Vector DBs earn their keep past ~1M vectors with high QPS. Use `pgvector` in the existing Postgres and move only if measurement forces it.
- **Never embed numeric results into the vector space** (§3.4). Numbers go in SQL; vectors are for fuzzy similarity over assets, regimes, configs, and natural-language insight records.

#### Regime space **R**

Define a regime *operationally*, or it becomes storytelling. Practical stack: HMM or HSMM over a small market-state feature vector (realized vol, term structure slope, credit spread, breadth, dispersion, correlation level) + Bayesian online change-point detection for transitions, with regimes labeled post-hoc by their statistical properties rather than by narrative ("high-vol / high-dispersion / negative-momentum" not "the 2022 regime"). Validate by out-of-sample regime-conditional performance separation. Store regime assignments bitemporally — the regime you *thought* you were in at time t matters more than the one you now know you were in.

---

### 3.7 Internal model roster — "go all-in," staged by data availability

You said go all-in. Here is the full roster, with the honest answer to "when does this actually beat a heuristic?"

**The universal rule, applied to every one of these:**
> Every internal model ships first as a **documented heuristic**. The learned version runs in **shadow** against that heuristic on a held-out slice of the ledger. It replaces the heuristic only when it wins by more than δ_practical on that slice, with the comparison done under §3.2's protocol. Promotion is logged, versioned, and reversible with one command.

This is what stops "we added ML to our ML platform" from silently making things worse.

| ID | Model | Input → Label | Trainable at | Model class | Cold-start heuristic |
|---|---|---|---|---|---|
| **M1** | Runtime/cost predictor | config + dataset spec → wall-clock, GPU-h, $ | ~200 runs | GBDT (quantile) | analytical estimate from data size × epochs |
| **M2** | Failure classifier | config + env → {OOM, NaN/divergence, data error, timeout, OK} | ~300 failures | GBDT / small MLP | static rules (batch×params vs VRAM) |
| **M3** | Learning-curve extrapolator | partial curve + config → final metric posterior | ~1k partial curves | LC-PFN / FT-PFN style | median stopping rule |
| **M4** | Config→performance surrogate (per asset-cluster) | (S, A, R) → O | ~2k trials/cluster | GBDT ensemble or TabPFN-class | TPE's internal model |
| **M5** | **Gate pre-screener** | (S, A, R, cheap-eval metrics) → P(passes gates 1–13) | ~1k gated candidates | calibrated GBDT | run cheap gates first (already the order) |
| **M6** | Asset encoder | OHLCV → A-vector | day 1 (unsupervised) | Tier1 stats → Tier3 SSL | Tier 1 + Tier 2 landmarkers |
| **M7** | Regime classifier | market features → regime posterior | day 1 | HMM/HSMM + BOCPD | vol-tercile buckets |
| **M8** | Strategy-family recommender | (A, R) → ranked families | ~5k trials over ≥100 assets | IPW tensor factorization | global family win-rates |
| **M9** | Experiment-proposal ranker | proposed hypothesis + context → expected value-per-dollar | ~2k proposals w/ outcomes | learning-to-rank (LambdaMART) | acquisition function (EI/UCB) |
| **M10** | Leakage detector | pipeline AST + feature stats + CV/WF gap → P(leak) | **day 1 via synthetic injection** | GBDT + static analysis | the 3 tests in §3.3 |
| **M11** | Metric-stream anomaly detector | live curves → anomaly score | ~500 runs | simple: robust z / matrix profile | threshold rules |
| **M12** | Fine-tuned tool-executor LLM | accepted agent trajectories → tool calls | ~5–10k accepted trajectories | LoRA on a small open model | frontier model + good tool design |
| **M13** | Step critic / PRM | trajectory step → {correct, correctable, irrecoverable} | ~3k labeled steps | 4B-class PRM (per §3.4) | rule-based + judge triage |

**Highest ROI, in order: M5, M9, M3, M1.**
- **M5 (gate pre-screener) is the money model.** Your gate stack will have a single-digit-percent pass rate. Every candidate that gets a full CPCV + bootstrap + Romano-Wolf evaluation costs real compute. A calibrated model that says "this has a 2% chance of passing" before you spend that, with a conservative operating threshold tuned for high recall, is a direct multiplier on how many hypotheses you can afford to explore.
- **M9 turns the agent's own history into judgment.** It's also the one that most needs the propensity logging from §3.5, because it's explicitly a policy-learning problem.
- **M10 is special: it's trainable on day one**, because you can generate unlimited labeled data by *deliberately injecting known leaks* into known-clean pipelines. Synthetic-label bootstrapping is available here and nowhere else on this list. Build it early.

**M12 — is fine-tuning worth it?** Honest answer: **not at first.** In 2026 the break-even for fine-tuning a small model on a bespoke toolset is roughly: high call volume on a stable toolset, latency or cost pressure, or a privacy/determinism requirement. Early on your toolset will change weekly, which is exactly the condition where fine-tuning is wasted. **What to do instead from day one: log every trajectory in a fine-tuning-ready schema** (tool schema version, full call sequence, outcome, human/gate verdict) so that when the toolset stabilizes you have the corpus and can train in a week rather than starting collection then. Same pattern as propensity logging: cheap now, impossible later.

---

### 3.8 Your scheduled-vs-user-driven question — a direct answer

You asked: should the platform retrain its own internal models on a schedule, or should the user drive it? Should the server ever impose new training?

**Answer: it depends on what the model touches, and the split is three tiers.**

**Tier A — fully autonomous. Server retrains and auto-promotes.**
M1 (cost), M2 (failure), M6 (asset encoder), M7 (regime), M11 (anomaly).
*Why it's safe:* these models' errors are cheap, immediately visible, and reversible. A bad cost estimate wastes a queue slot. Retrain on a schedule **plus** a drift trigger; auto-promote only if the challenger beats the champion on a **frozen holdout slice of the ledger** that the challenger's training never touched. Auto-rollback on degradation.

**Tier B — server trains continuously, human approves promotion.**
M4 (surrogate), M5 (gate pre-screener), M8 (family recommender), M9 (proposal ranker), M12 (fine-tuned executor), M13 (critic).
*Why the gate:* these models **shape what gets explored**. That's a feedback loop — if M8 stops recommending mean-reversion on high-vol assets, you stop generating evidence about mean-reversion on high-vol assets, and M8's belief becomes self-confirming and unfalsifiable. Training is continuous and free; *promotion* is a decision with an owner. Mitigations that go with it: the mandatory exploration floor (§3.5), off-policy evaluation against the log before promotion, and a periodic check that the policy's entropy hasn't collapsed.

**Tier C — never autonomous.**
The promotion gates themselves, the thresholds, δ_practical, position sizing, capital allocation, and anything that touches money.
*Why:* a system that learns to adjust its own passing criteria based on its own outcomes will learn to pass. **The gates must never be trained on the platform's own results.** They are set by a human, versioned, and changing one invalidates prior comparisons — which the system should say out loud when someone tries.

**Three guardrails that apply across all tiers:**
1. **Model collapse protection.** Never train an internal model purely on platform-generated data without a real-outcome anchor. Every retraining set must include ground truth from actual market outcomes (paper/live performance), not just backtest results. Recursive training on synthetic/self-generated distributions degrades measurably and quietly.
2. **An immutable seed holdout.** A slice of the ledger frozen at inception, never used for training any internal model, kept for detecting long-run drift in the platform's own judgment.
3. **A "platform judgment" dashboard.** The system should report on *itself*: is M5's calibration still good? has M8's recommendation entropy collapsed? is the gate pass rate trending in a way that suggests threshold drift or gate-hacking? This is the observability layer nobody builds and everybody needs.

**On "should the server ever impose training?"** — Yes, for Tier A, and it should be invisible. For Tier B it should *propose*: "M5 has a challenger that improves recall 8% on held-out data; review and promote?" For Tier C, never. And there should always be a global "freeze internal models" switch, because during a market dislocation the last thing you want is your meta-models retraining on three weeks of unprecedented data.

---

## 4. Architecture at a glance

```
┌─────────────────────────────────────────────────────────────────────┐
│  CONTROL SURFACES  (one typed API; UI and agent are both clients)   │
│  Human workspace: presets → guided → expert; pipeline builder;      │
│    comparison dashboards; lineage; approvals; agent action review   │
│  Agent workspace: ~15 always-on typed tools + searchable tail;      │
│    dry_run / estimate_cost / idempotency_key on everything          │
└───────────────────────────────┬─────────────────────────────────────┘
                                │
┌───────────────────────────────▼─────────────────────────────────────┐
│  CAMPAIGN ENGINE  (Temporal workflows — durable, resumable)         │
│  objective+constraints+δ_practical → baseline → error analysis →    │
│  hypothesis generation → controlled trials → statistical compare →  │
│  gate stack → compute reallocation → branch pruning → promote/stop  │
│  ── with trial accounting and propensity logging at every choice ── │
└───────────────────────────────┬─────────────────────────────────────┘
                                │
┌──────────────┬────────────────▼──────────────┬──────────────────────┐
│ SEARCH       │  EXECUTION (Ray + Kueue/MIG)  │  EVALUATION          │
│ portfolio →  │  train / backtest / CPCV /    │  gate stack 1–14     │
│ TPE/GP/DEHB  │  walk-forward / bootstrap     │  Romano-Wolf, DSR,   │
│ + ASHA       │  checkpoint · resume · cancel │  PBO, factor attrib  │
│ + ≥10% random│  budget enforced at admission │  sealed holdout (RL) │
└──────────────┴───────────────┬───────────────┴──────────────────────┘
                               │
┌──────────────────────────────▼──────────────────────────────────────┐
│  DATA SUBSTRATE                                                     │
│  L3 knowledge:  A / R / S / O / F vector spaces · Outcome Tensor    │
│                 internal models M1–M13 · agent insight records      │
│  L2 ledger:     immutable · hash-chained · propensity-logged ·      │
│                 full OOS series · per-fold predictions              │
│  L1 datasets:   content-addressed specs · versioned features        │
│  L0 market:     BITEMPORAL (event_time + knowledge_time) · Iceberg  │
└─────────────────────────────────────────────────────────────────────┘
```

**Store split:** Postgres (system of record + pgvector) · Parquet/Iceberg (metrics, series, predictions) · DuckDB→ClickHouse (analytics) · NATS→SSE (live UI) · content-addressed object store (artifacts).

---

## 5. What this changes versus your original prompt

Things the original prompt got right and I'd keep unchanged: the 11-step orchestration loop's shape; multi-session campaigns with preserved lineage; the human/agent dual surface; granular agent tools; "never call it optimal from one number"; the financial safeguards list; preset/guided/expert UI modes.

Things I'd change:

1. **Add trial accounting as a first-class step.** The 11-step loop has no step that counts how many hypotheses have been tested. Without it, steps 6 and 10 ("statistically defensible comparison," "quality gates") cannot actually be done. This is the largest hole.
2. **Add `δ_practical` as a required field** on every campaign. "Better" is undefined without a smallest-meaningful-difference, and most stopping rules are unanswerable without it.
3. **Add propensity logging and a mandatory exploration floor.** Without these, the trial ledger can never train the internal models you want. Cheap on day one, impossible later. (§3.5)
4. **Make the market data plane bitemporal.** The prompt says "strict chronological separation," which is necessary but not sufficient — restated data and survivorship need `knowledge_time`. Also unretrofittable. (§3.5)
5. **Replace per-action human approval with budget envelopes.** Users approve 93% of prompts; per-action approval produces the *feeling* of control and none of it. Gate four specific things instead. (§3.4)
6. **Rebalance effort from search algorithms to edit operators.** The prompt lists grid/random/Bayesian/population-based search prominently. Research says operator quality dominates search policy, and that the *selection* of the final candidate dominates both. (§2, §3.4)
7. **De-emphasize distributed training and mixed precision.** These are in the prompt and they're real capabilities, but for the model sizes typical in quant (GBDTs, small nets, factor models) they are close to irrelevant and would absorb engineering that belongs in the data substrate. Keep single-node multi-GPU + MIG slicing; skip multi-node until something needs it.
8. **Reframe "ensembling/calibration/threshold optimization" as a fixed post-hoc pipeline**, not as options. Order matters: ensemble → calibrate on a dedicated split → closed-form threshold from the cost matrix. Tuning the threshold is a mistake the prompt's phrasing invites. (§3.2)
9. **Split "evaluation" into model metrics and portfolio metrics.** They're different objects with different owners and different gates; the prompt runs them together.
10. **Add self-observability.** The platform must monitor its own judgment (gate pass-rate drift, recommender entropy collapse, internal-model calibration), not just model drift. (§3.8)
11. **The UI's hard problem is comparison and attribution, not configuration entry.** The prompt spends most of its UI budget on config surfaces. The screen that actually determines whether this system is useful is *"why did candidate A win, and do I believe it?"* — lineage diff, variable-change tracking, per-regime PnL decomposition, gate-by-gate results, and the trial count that contextualizes the whole thing.

---

## 6. Decisions I need from you

| # | Decision | Options | My recommendation |
|---|---|---|---|
| 1 | **Scale target** | single-user/small-team vs multi-tenant SaaS | Changes RBAC, quota, and store choices substantially. Tell me which. |
| 2 | **Asset universe & frequency** | e.g. US equities daily? intraday? crypto? futures? | Drives whether L0 needs `timestamp_ns`/microstructure, and whether capacity gates matter |
| 3 | **Orchestration** | Temporal + Ray vs Flyte 2 | Temporal + Ray (maturity), unless you want one system and accept young-GA risk |
| 4 | **Build vs buy tracking** | MLflow 3 self-hosted vs W&B vs build on Postgres | MLflow 3 for the registry/lineage, **your own** trial ledger — MLflow can't express the ledger's guarantees |
| 5 | **Gate strictness at launch** | strict (single-digit pass rate) vs permissive w/ tightening | Strict. Permissive gates that tighten later invalidate every earlier comparison |
| 6 | **Internal-model retraining** | see §3.8 tiers | Tier A auto / Tier B propose-and-approve / Tier C never |
| 7 | **Tabular foundation model licensing** | TabPFN commercial license vs permissive (Mitra/TabICL/TabDPT) | Start permissive; the Elo gap doesn't justify a license negotiation pre-revenue |
| 8 | **Does the platform ever trade real capital, or is it research-only?** | | Changes gate 14, governance load, and whether the 2026 model-risk framework applies to you |

---

## 7. Proposed next steps

1. You react to this brief — especially the eight decisions above, and anything in §5 you disagree with.
2. I write **the unified specification**: frontend, backend services, database entities and DDL, the training-job state machine, experiment/checkpoint lifecycle, the full agent tool catalog with typed signatures, permissions model, APIs, optimization algorithms, evaluation rules, promotion gates, observability, failure handling, infrastructure, and representative user flows for both a human and an agent.
3. I write **the implementation plan** — build order, with the explicit note that the unretrofittable pieces (bitemporal L0, propensity-logged ledger, per-fold prediction storage) come first even though the live UI and cost dashboards are what everyone wants to build first.
4. Then the long-horizon verification you described: agent designs a strategy with a model in the loop → backtests → analyzes → modifies → re-runs, with the whole thing observable in the ledger, and we see where the design breaks.

---

*Sources: four research reports totaling ~39,000 words with ~250 cited URLs, available on request. Sections 3.5–3.8 are my own synthesis and have not had a dedicated deep-research pass — say the word if you want one before the spec.*
