# HPO, Multi-Fidelity Search, Automated Experiment Selection, and Meta-Learning

**Research note for a production ML training & experimentation platform with an AI-agent operator.**
Status as of **September 2026**. Verified against current library versions and 2025–26 literature where possible.

---

## 0. TL;DR decision table (read this first)

Let `B` = number of *full-fidelity-equivalent* trials the budget affords, `D` = number of tunable hyperparameters,
`P` = number of parallel workers, `t_full` = wall-clock of one full training run.

| Regime | Use |
|---|---|
| `B < 10`, any `D` | **Do not search.** Use a *portfolio / zero-shot* set of known-good configs from the experiment DB (§3). Rank by meta-learned surrogate, run the top `B`. |
| `B` 10–30, `D ≤ 8`, cheap runs, no learning curve | **GP-BO** (Ax/BoTorch `HyperparameterOptimizationFacade`-equivalent, LogEI) or Optuna `GPSampler`. TPE is under-powered at this budget. |
| `B` 10–30, expensive DL runs **with** learning curves | **PriorBand / πBO-style prior-guided multi-fidelity**, or **ifBO** (FT-PFN freeze-thaw). This is the "10× t_full total budget" regime PriorBand was built for. |
| `B` 30–200, `D` 5–25, mixed/categorical/conditional space | **TPE (Optuna)** or **SMAC3** (RF surrogate handles conditionals + categoricals natively). |
| `B` 30–200, `D` 5–25, continuous only | **GP-BO with dimensionality-scaled lengthscale prior** (Hvarfner et al. 2024). Vanilla GP-BO is *not* broken in high-D — the default prior was. |
| `B` > 200, `D` > 25, mostly continuous | **CMA-ES** (or Nevergrad `NGOpt` wizard). GP cost is O(n³) and starts to dominate. |
| `B` > 200, discrete/mixed, multi-fidelity available | **DEHB** — DE inside Hyperband brackets; flat runtime cost, no cubic surrogate blowup. |
| `P ≥ 8` workers, learning curves exist, `t_full` large | **ASHA** (asynchronous successive halving) — near-linear worker utilization (~95.7% measured at 1,280 workers). Add a model-based searcher on top → **MOBSTER / ASHA-BORE / BOHB**. |
| `P ≥ 8`, but `max_t` unknown / you want auto-stop | **PASHA** — grows the max rung only while top-rung rankings are unstable. 2.3–3.4× over ASHA on NAS-Bench-201, up to 15.5× on large PD1 tasks. |
| Single long training run, HPs are *schedules* (LR, augmentation strength, entropy coef in RL) | **PBT / PB2**. Only method that finds a *schedule* rather than a static config. Needs `P ≥ 16` and mutable checkpoints. |
| 2 objectives (acc/latency, acc/cost), `B` ≤ 50 | **qNEHVI** (BoTorch) or **PriMO** if you have priors. Avoid NSGA-II at this budget — EAs are "compute-inefficient" below a few hundred evals. |
| 2 objectives, `B` > 500 | **NSGA-II**; ≥4 objectives → **NSGA-III**. |
| Hard constraint (latency ≤ X ms, memory ≤ Y GB) | **Constrained BO**: qNEHVI/qLogNEI with outcome constraints, or feasibility-weighted EI. Do **not** penalty-scalarize into a single objective. |
| Tabular, ≤ 500k rows | **Skip HPO entirely first.** Run a tabular foundation model (TabPFN-3 / TabICLv2 / Mitra) + AutoGluon `extreme` portfolio. Only tune if that plateaus (§4). |

**Meta-rule for the agent operator:** the highest-expected-value action at the start of any new task is *not* to launch a search. It is to (a) query the experiment DB for the nearest prior tasks, (b) materialize a 8–32 config portfolio, (c) run those under ASHA, and (d) only then hand the residual budget to a model-based searcher warm-started on those results.

---

## 1. Modern HPO algorithms: what each is and when it wins

### 1.1 The baselines that refuse to die

**Random search** is not a strawman. In the 2026 `LLMSYS-HPOBench` study (7 real LLM systems: LightRAG, NaiveRAG, HtmlRAG, vLLM, SGLang, AutoGPT, OpenHands; 364,450 configurations, 12–23 dims, ~2.3M CPU-core-hours + ~95k GPU-hours), **random search was the best optimizer on LightRAG and vLLM**, beating Hyperband, BOHB, SMAC, and HEBO. The authors attribute this to interactions between "AI" hyperparameters (temperature, top-k) and "non-AI" ones (chunk size, batching policy, KV-cache config) producing landscapes that break the smoothness assumptions of surrogate models.

Operational consequence: **always run a random-search arm in parallel with the smart searcher**, at 10–20% of budget. It is your canary for "the surrogate is mis-specified on this task." If random matches or beats the model-based arm after ~40 trials, switch to random/CMA-ES and log the task signature as "surrogate-hostile" in the meta-DB.

**Grid search**: never, except for ≤2 dims with genuinely ordinal semantics you must report on.

### 1.2 TPE (Optuna)

Tree-structured Parzen Estimator models `p(x | y < y*)` and `p(x | y ≥ y*)` and maximizes their ratio, rather than modeling `p(y|x)`. Density-ratio estimation, so it is O(n) not O(n³), handles categorical + conditional (tree-structured) spaces natively, and parallelizes acceptably with the constant-liar trick.

- **Wins**: 30–500 trials, mixed/conditional spaces, pipeline search (which model × which preprocessor × its HPs), when you need a library that just works and stores to a DB.
- **Loses**: very low budgets (< 20 trials — the KDE has nothing to fit); strongly-correlated continuous dims (TPE's univariate factorization ignores interactions; `multivariate=True` partially fixes this); expensive per-trial where sample efficiency dominates.
- **Current state (Sept 2026)**: Optuna **v4.9.0** is the latest line. `GPSampler` got Kriging-Believer batch handling plus full constrained/multi-objective support in 4.9; 4.8 added constant-liar to reduce duplicate parallel proposals. Several TPE knobs (`prior_weight`, `gamma`, `categorical_distance_func`), `CmaEsSampler(x0, sigma0)`, `optuna.terminator`, and the MLflow/TensorBoard/Comet integration callbacks are **deprecated for removal in v6.0.0** — they migrate to OptunaHub. *Pin your Optuna version and budget a migration.*
- **`AutoSampler`** (OptunaHub) dispatches among TPE / GP / NSGA-II / NSGA-III based on budget, space type, objective count and constraints. It is a reasonable default for a platform that must serve non-experts — but the published material does **not** disclose the numeric thresholds, so if you need auditable behavior, implement your own dispatcher (§0 table) rather than depending on an opaque wizard.

### 1.3 GP-based BO (Ax / BoTorch)

Gaussian-process surrogate + acquisition optimization. Best sample efficiency per trial when the space is continuous, low-to-moderate dimensional, and the objective is reasonably smooth.

The big 2024–26 correction: **"Vanilla Bayesian Optimization Performs Great in High Dimensions"** (Hvarfner, Hellsten, Nardi, ICML 2024). The claimed failure of standard GP-BO above ~20 dims was largely an artifact of lengthscale priors that concentrate on short lengthscales; in high D, distances between random points in `[0,1]^D` scale as √D, so a fixed prior makes the kernel matrix approach the identity and the model learns nothing. The fix is one line:

```
ℓ_i ~ LogNormal(μ₀ + log(D)/2, σ₀)      # μ₀ = √2, σ₀ = √3
```

This shifts mode and mean by √D. With it, plain GP-BO ("DSP") is best or near-best on MOPTA08 (124D), SVM (388D), Lasso-DNA (180D), Ant (888D), Humanoid (**6392D**) — matching or beating SAASBO, TuRBO and ALEBO. **Implication for the platform: do not reach for TuRBO/SAASBO/embedding tricks by default.** Use a well-priored GP first; reserve SAASBO for genuinely sparse problems where you expect ≤10 of 100+ dims to matter and you can afford fully-Bayesian (NUTS) inference, and TuRBO for large-budget (≥1000 eval) local-trust-region problems.

- GP-BO cost is cubic in observations. Past ~300–500 observations, surrogate fitting time becomes a real fraction of the loop; switch to a scalable surrogate (RF/SMAC, BORE/density-ratio, or CMA-ES).

### 1.4 SMAC3

Random-forest surrogate + EI, built for *algorithm configuration* (categorical, conditional, hierarchical spaces; multiple instances; runtime objectives). SMAC3 v2.x exposes facades (`HyperparameterOptimizationFacade`, `MultiFidelityFacade`, `BlackBoxFacade`, `AlgorithmConfigurationFacade`, `RandomFacade`) and, since v2.0, **all facades/intensifiers support multi-objective, multi-fidelity and multi-threading natively**.

- **Wins**: heavily conditional search spaces (pipeline/NAS-like), noisy objectives evaluated over instance sets, when you need runtime as a first-class objective, when `D > 20` with many categoricals.
- **Loses**: smooth low-D continuous problems (GP is better calibrated); RF gives poor extrapolative uncertainty.

### 1.5 Successive halving → Hyperband → ASHA → async ASHA

**Successive halving (SH)** is a non-stochastic best-arm-identification bandit (Jamieson & Talwalkar 2016): evaluate `n` arms at budget `r`, keep the top `1/η`, multiply budget by `η`, repeat. **Hyperband** wraps SH in an outer loop over brackets with different `(n, r)` trade-offs, hedging against the unknown correlation between low- and high-fidelity ranks.

**ASHA** (Li et al., *A System for Massively Parallel Hyperparameter Tuning*) removes the synchronization barrier: a free worker promotes any config that is in the top `1/η` of its rung, else samples a new config at the bottom rung. This is the workhorse for distributed HPO.

Measured at scale (mlr3 async benchmark, XGBoost/8 HPs, bank-marketing, 3-fold CV AUC, **1,280 workers / 10 nodes / 30 min**):

| method | worker utilization |
|---|---|
| ASHA | 95.7% |
| ADBO (async distributed BO) | 93.5% |
| ASHA hotstart | 92.8% |
| parallel random search | 92.7% |

ASHA variants evaluated **12–15× more configurations** than random search in the same wall-clock, though on that particular (easy) task final AUC converged similarly across all four. Communication + resampling overhead was ~7% for all.

**Practical ASHA settings.** `η = 3` (default, good) or `4`; `η = 2` promotes too conservatively and wastes compute, `η ≥ 5` is aggressive and dangerous with noisy metrics. `grace_period` (`r_min`) should be **at least the point where your learning curves stop crossing** — empirically, for DL, crossings are rare after the first ~10–20% of training. Set `max_t` to the real training length; if unknown, use PASHA. Use `brackets = 1` (pure ASHA) when you trust low fidelity, `brackets = 3–4` (async Hyperband) when you do not.

**BOHB** = Hyperband brackets + a TPE/KDE model fit on the *highest fidelity with enough observations*, used to sample new configs instead of sampling randomly. Strictly dominates Hyperband once ≥ ~`2·(D+2)` observations exist at some rung. The synchronous formulation hurts at high parallelism; use **MOBSTER** (async ASHA + GP) or **ASHA-BORE** in Syne Tune, or **Hyper-Tune** (VLDB 2022), which adds asynchronous scheduling + a bandit-based fidelity-level selector on top of ASHA.

**DEHB** replaces BOHB's Bayesian model with differential evolution, keeping a subpopulation per fidelity and drawing mutation parents from the *lower* fidelity subpopulation so information flows upward. Claims up to **32× faster than BOHB** on HPO problems and 1000× over random search; crucially, **runtime does not grow with the number of evaluations** (no cubic surrogate). Best choice for long-running, high-dimensional, discrete-heavy searches.

### 1.6 Freeze-Thaw BO and its 2024–26 successors

Classic Freeze-Thaw BO (Swersky et al. 2014) models *partial* learning curves with an exponential-decay kernel, and at every step decides: resume some paused run, or start a new one. Historically fragile (GP on curves is slow and mis-specified).

**ifBO** (Rakotoarison et al., ICML 2024) is the modern instantiation: **FT-PFN**, a prior-data-fitted transformer trained on synthetic learning-curve priors, does Bayesian learning-curve extrapolation in **a single forward pass**, **10–100× faster** than deep-GP or deep-ensemble surrogates and more accurate; paired with an **MFPI-random** acquisition (multi-fidelity PI with randomized horizon/threshold) it sets SOTA across three DL HPO benchmark families. There is a maintained implementation (`automl/ifBO`, on PyPI).

A NeurIPS 2025 follow-up adds **cost-sensitive freeze-thaw BO**, which prices the *resume cost* (checkpoint load, data-loader warmup, cold GPU) into the acquisition — this matters a lot in practice: if resuming a run costs 90 s and a rung is 120 s, naive freeze-thaw is a net loss.

**Platform requirement:** freeze-thaw and pause/resume schedulers demand **cheap, correct, resumable checkpoints** — model + optimizer + LR-scheduler + RNG + data-loader position + AMP scaler. If your trainer cannot resume bit-reproducibly, restrict yourself to *stopping*-type schedulers (ASHA-stop, median rule) and forfeit freeze-thaw entirely.

### 1.7 PriorBand

Targets the realistic DL regime: **total budget ≈ 10× the cost of one full training run**, and the practitioner already has a decent guess. PriorBand replaces Hyperband's uniform sampling with an *ensemble* sampler that draws from {expert prior, incumbent-centered local distribution, uniform random} with weights that shift over the bracket/rung structure — early on it leans on the prior, and it decays prior influence as evidence accumulates. It is explicitly engineered to **degrade gracefully under bad priors** (the ε-greedy / decaying-weight structure prevents the prior from trapping the search).

This is the single most agent-friendly algorithm in the list: the LLM operator's job — "propose a plausible region for lr, wd, warmup based on the model family and prior experiments" — maps exactly onto PriorBand's prior input, and the algorithm is robust when the agent is wrong. The multi-objective analogue is **PriMO** (2025), which reports **~10× speedups** at 20-evaluation budgets and recovers from misleading priors better than πBO+RW.

### 1.8 PBT / PB2

**PBT** trains a population (typically 16–64) in parallel; periodically, poor workers *exploit* (copy weights + HPs of a good worker) and *explore* (perturb HPs). It finds a **schedule**, not a point — which is why it wins for LR/entropy-coefficient/augmentation-strength in RL and long LLM/vision training, and why it can beat any static-config searcher on those problems.

**PB2** replaces PBT's random perturbation with a time-varying GP bandit (GP-UCB over a non-stationary kernel), giving theoretical regret guarantees and, empirically, good results with **much smaller populations** (which is the whole practical objection to PBT). `PB2-Mix` extends this to categorical/mixed HPs.

Caveats the platform must handle: PBT is **synchronous-ish and stateful** (needs weight copying between workers → fast shared storage), it **confounds the "best config"** (the winning trajectory's final HPs are meaningless in isolation — you must export the *schedule*), and it is **not reproducible** without logging the full exploit/explore lineage. Log the lineage DAG as a first-class artifact.

### 1.9 CMA-ES and Nevergrad

**CMA-ES**: evolution strategy that adapts a full covariance matrix; essentially a derivative-free quasi-Newton. Strong for `D` 10–100 continuous, budget ≥ ~`10·D` to `100·D` evaluations, moderately noisy objectives. Poor with categoricals (use CatCMA / one-hot with care) and wasteful at tiny budgets. Available as Optuna `CmaEsSampler`.

**Nevergrad**: a portfolio/wizard library. `NGOpt` is an **algorithm-selection wizard** whose dispatch rules were themselves auto-configured (via irace-style automated algorithm configuration) over a large benchmark suite. Use it as a strong "I don't know what this landscape looks like" default for expensive black-box problems that are *not* multi-fidelity. It is the best off-the-shelf answer when the objective has no learning curve (e.g. tuning a data pipeline, a feature-engineering config, a serving config).

### 1.10 Multi-objective and constrained

**Scalarization** (random weights / Chebyshev / hypervolume scalarization) + any single-objective optimizer is the cheapest thing that works, and with **random weights re-drawn each trial (ParEGO / "BO+RW")** it recovers a front. Linear scalarization provably cannot reach non-convex parts of the Pareto front; **hypervolume scalarization** can.

**qEHVI / qNEHVI** (BoTorch) directly optimize (noisy) expected hypervolume improvement, support batch (`q > 1`) and outcome constraints, and are the sample-efficiency winners for 2–3 objectives at small budgets. **qNEHVI** is the right default over qEHVI when observations are noisy (the "N" is noisy-EHVI via a sampled-Pareto-front formulation) — which is always true for validation metrics.

**qNParEGO** scales better to more objectives and batch sizes, at some hypervolume cost.

**NSGA-II / NSGA-III**: genetic, no surrogate, needs hundreds-to-thousands of evaluations. Use only when evaluations are cheap (< 1 min) or you have massive parallelism. The 2025 DL-focused MOHPO survey is blunt that EAs are "quite compute-inefficient" in DL budgets. **MOASHA** (multi-objective ASHA, in Syne Tune) is the cheap multi-fidelity multi-objective option and is used as PriMO's initial design (5 configs, η=3).

**Constrained BO**: model each constraint `c_j(x) ≤ 0` with its own GP and multiply the acquisition by `∏_j P(c_j(x) ≤ 0)` (feasibility-weighted EI), or use BoTorch's `constraints` machinery on qLogNEI/qNEHVI. This is strictly better than a penalty scalarization because it (a) keeps the objective model clean and (b) lets the optimizer *learn the feasible region* rather than being shoved away from it by an arbitrary penalty weight. For the platform this is the right way to express "must fit in 24 GB", "p99 latency ≤ 50 ms", "model size ≤ 2 GB".

### 1.11 LLM-agent optimizers — current honest assessment

`AgentHPOBench` (2026): 30 executable ML tasks across seven research categories, each starting from a validated baseline; agents observe configs, metrics and logs and propose the next config. Twelve agents and conventional HPO baselines under a unified protocol. Conclusion: agents show "measurable experimental optimization ability across domains" but have "clear limitations in sustained iterative refinement, complex log diagnosis, and consistent progress toward reported reference performance." Earlier `AgentHPO` (CPAL 2025) showed LLM agents matching or beating human-expert baselines on a set of tasks with far fewer trials, largely because the LLM encodes good *priors*.

**Design conclusion:** do not make the LLM the optimizer. Make the LLM **the prior, the space designer, and the diagnostician**:
1. It proposes the search space (which HPs, which ranges, log vs linear, conditional structure).
2. It proposes the *prior* for PriorBand/πBO and the initial portfolio.
3. It reads failure logs (OOM, divergence, NaN loss, dead ReLU, data-loader starvation) and *edits the space* mid-campaign — a thing no classical optimizer does.
4. A classical algorithm (ASHA + TPE/GP/ifBO) does the numeric optimization.
This partition plays to the measured strengths and avoids the measured failure mode (sustained iterative refinement).

---

## 2. Multi-fidelity & early stopping

### 2.1 Fidelity types

Fidelity = anything monotone-ish that trades cost for signal:
- **epochs / steps / iterations** (the canonical one; supports resume)
- **dataset subsample fraction** (good for tabular/GBDT; careful — subsample rank-correlation with full data degrades fast for high-capacity models)
- **resolution** (vision), **sequence length** (LLM), **number of folds** (CV)
- **model size proxies** (width/depth multiplier — the shakiest, rank correlation is weakest)
- **number of MC samples / eval-set size** (for evaluation-side fidelity)

The 2025 review of multi-fidelity HPO catalogs these and notes the same recurring failure: **multi-fidelity only helps to the extent that low-fidelity rank order predicts high-fidelity rank order.** Measure this. Concretely: once per task family, run ~20 configs to full budget and compute **Spearman ρ(rung_k, final)** for each rung. If ρ at your first rung is < ~0.3, raise `grace_period` or abandon that fidelity axis. Store ρ per (task-family, fidelity-axis) in the meta-DB and let the agent read it.

### 2.2 Median stopping rule

Google Vizier's default: at step `t`, stop trial `i` if its *running average* objective is worse than the median of the running averages of all previously-completed trials at step `t`. Properties: model-free, hyperparameter-free (other than a min-steps grace period), robust, and — critically — it does **not** require the rung structure that SH needs, so it works with heterogeneous trial lengths and continuous arrivals.

- **Use when**: trials arrive continuously, trial lengths vary, you want something with essentially zero configuration risk. Syne Tune ships `MedianStoppingRule` as a wrapper around *any* searcher.
- **Weaker than ASHA when**: you have a fixed rung ladder and high parallelism — ASHA's explicit top-`1/η` promotion gives better budget concentration.
- **Failure mode**: the median is computed over *all* prior trials including early garbage, so late in a campaign it becomes too permissive. Use a rolling window or the median of the top-50%.

### 2.3 Learning-curve extrapolation

Three generations:
1. **Parametric ensembles** (Domhan et al. 2015): fit a weighted ensemble of 11 saturating parametric families (pow³, log-power, MMF, Weibull, Janoschek, …) by MCMC, stop if `P(final > best_so_far) < δ` (δ = 0.05 typical). Works, but MCMC per trial is slow and priors are hand-made.
2. **Learned/joint surrogates**: DPL (deep power laws), DyHPO (deep-kernel GP over (config, budget)) — model the curve *and* the config jointly so you can transfer across configs.
3. **PFNs — current SOTA.** **LC-PFN** (Adriaensen et al., NeurIPS 2023) trains a transformer on synthetic curves sampled from a prior over curve shapes; at inference it does approximate Bayesian posterior inference over the *continuation* of a partial curve in **a single forward pass**, over **10,000× faster** than the MCMC approach while being at least as accurate, and it beats MCMC-based extrapolative stopping on real LC-benchmarks. **FT-PFN** (inside ifBO) extends this to condition on the configuration too.

**Recommendation**: implement stopping as a policy over a posterior, not a point estimate:
```
stop trial i at step t  if   P( final_i > incumbent + δ_practical | curve_i[0:t] ) < p_stop
```
with `δ_practical` = the smallest improvement you would actually ship (see §5) and `p_stop` ≈ 0.05–0.10. Using LC-PFN this is a forward pass per trial per rung — negligible cost. This is strictly better than median stopping because it accounts for *how much* of the curve remains and for curve shape.

### 2.4 Pause/resume vs stop-and-discard

| | stop-and-discard (ASHA, median rule) | pause/resume (freeze-thaw, PBT, SHA-with-checkpoints) |
|---|---|---|
| checkpoint requirement | none | full resumable state |
| storage | O(survivors) | O(all paused trials) — can be huge |
| risk | discards a late bloomer forever | resume cost may exceed benefit |
| best when | curves separate early; storage expensive | curves cross late; checkpoints cheap; GPU already warm |

Concrete policy: enable pause/resume only if `checkpoint_write + checkpoint_read + warmup < 0.15 × rung_duration` **and** you have the object storage to hold `n_bottom_rung` checkpoints. Otherwise use stop-and-discard with a slightly larger `grace_period`. Always garbage-collect: keep checkpoints only for trials in the top `2/η` of their rung, plus the global incumbent.

### 2.5 ASHA × noisy validation metrics — the sharpest practical hazard

This is where most production HPO quietly fails. ASHA promotes on an *observed* rung metric. If the metric's noise σ is comparable to the spread between configs at that rung, promotion becomes close to random — worse, ASHA has a **winner's-curse bias**: the top-`1/η` at each rung are selected partly for having a lucky validation draw, so the incumbent's reported score is optimistically biased, and that bias compounds across rungs.

Mitigations, in rough order of cost-effectiveness:
1. **Don't promote on the last-step metric.** Use a smoothed statistic: EMA of the last `k` evaluations, or `max` over the last `k` (for monotone-ish metrics), or the mean over the last 3 evaluations. Cheap, large effect.
2. **Raise `grace_period`** past the noisy early phase; raise the eval-set size at higher rungs (evaluation-side fidelity).
3. **Soft ranking (PASHA).** Treat performance differences smaller than ε as ties, with ε *estimated from the observed ranking noise* — PASHA sets ε to the 90th percentile of the gaps between config pairs that swap ranks across consecutive epochs. Then grow `max_t` only while the top-two rungs disagree in ranking. 2.3–3.4× speedup on NAS-Bench-201, 15.5× on WMT, 1.9× on ImageNet, with equal accuracy after full retrain.
4. **Statistical pruning when you have independent replicates.** Optuna's `WilcoxonPruner` applies when a trial's objective is a mean over independent evaluations (CV folds, problem instances, per-question LLM scores): it runs a **Wilcoxon signed-rank test** of the current trial vs the best trial on the *paired* per-instance results and prunes when the current trial is confidently worse. Default `p_threshold = 0.1`; requires stable instance IDs for pairing and ≥ 2 startup steps; cannot handle NaN/inf. This is the statistically-correct way to prune CV early. (The same idea appears as `SignificanceRepeatedTrainingPruner` in HPOflow.)
5. **Re-evaluate the incumbent.** SMAC's intensification does this natively: before crowning a new incumbent, run it on additional instances/seeds. Budget ~5–10% of total compute to re-running the top-3 candidates with fresh seeds. This is what actually prevents shipping a lucky seed.
6. **Never report the max-over-trials validation score as your expected performance.** It is biased upward by `≈ σ · E[max of n standard normals] ≈ σ·√(2 ln n)`. With 200 trials and σ = 0.3%, that is a ~0.8% phantom gain. Report the *held-out test* score of the selected config, selected on validation.

---

## 3. Meta-learning, warm-starting, and reusing the experiment database

This is the part that a platform with a *history* can do and a one-off script cannot. It is also the highest-ROI section.

### 3.1 Task/dataset representations — what actually works

Candidates, ordered by how well they hold up empirically:

1. **Trivial meta-features: `n_rows`, `n_features`** (+ `n_classes`, task type, metric). Auto-sklearn 2.0 deliberately uses **only these two** after finding richer meta-features were expensive, undefined on categorical/missing data, and of unclear benefit. This is a strong, humbling result.
2. **Performance-based ("landmarker") representation — the best one.** Represent a task by the *vector of performances of a fixed reference set of configurations* on it. Two tasks are similar if the same configs work on both. This sidesteps meta-feature engineering entirely and is what TabRepo/zero-shot portfolios and quantile-based transfer (Salinas et al. 2020) exploit. To make it cheap, evaluate the reference set at *low fidelity* (few epochs / subsample).
3. **Classical statistical/information-theoretic meta-features** (skewness/kurtosis of features, class entropy, mutual information, PCA spectrum, feature concentration). A 2026 study explicitly testing whether meta-features can *route* between tabular foundation models and GBDTs across 51 datasets found: for non-TFM-vs-TFM, **only one** meta-feature survived FDR-controlled screening (`attr_ent.skewness`), and it was "descriptive rather than decision-useful" — it failed to improve routing on held-out datasets. For NN-vs-tree, **no** meta-feature survived. Their own caveat: 51 datasets is probably too few. **Conclusion: do not build your routing on statistical meta-features alone.**
4. **Learned embeddings (Dataset2Vec)** — a permutation-invariant encoder trained to make same-dataset patches close. Elegant, but needs a large meta-dataset and hasn't displaced (2) in practice.
5. **Free-text task descriptors + LLM embedding.** Underrated and newly practical: the experiment DB has a natural-language description, the code diff, the model family, the dataset name. Embedding these gives a surprisingly good similarity kernel for *cold-start* retrieval before any trial has run. Use it as the prior, then switch to (2) once you have 5–10 landmarker results.

**Recommended representation for the platform: a concatenation.** `[trivial meta-features] ⊕ [low-fidelity landmarker vector over a fixed 16-config reference portfolio] ⊕ [LLM embedding of task description + model family]`. Retrieval uses the LLM embedding at t=0, blends in landmarkers as they arrive.

### 3.2 Portfolios (zero-shot HPO) — the single best use of history

**The construction (Auto-sklearn 2.0 / TabRepo).** Given a meta-dataset of `M` tasks × `K` candidate configs with recorded performance:

```
greedy_portfolio(perf[M][K], size S):
    P = []
    for s in 1..S:
        best_c, best_score = None, +inf
        for c in candidates \ P:
            # loss if we ran P ∪ {c} and picked (or ensembled) the best per task
            score = mean over tasks m of  f( perf[m][P ∪ {c}] )
            if score < best_score: best_c, best_score = c, score
        P.append(best_c); candidates.remove(best_c)
    return P
```
`f` = `min` for "pick the best single config per task"; `f` = greedy-ensemble-selection score for "build an ensemble per task" (TabRepo does this, which is why its portfolios are so strong). The objective is **submodular** (maps to sensor placement), so greedy is provably within `1 − 1/e` — it "closes at least 63% of the gap between the worst score and the best possible portfolio."

**Evidence it works:**
- Auto-sklearn 2.0 uses a **32-pipeline** portfolio (sized so successive halving gets two full iterations) and reports normalized error **3.58 vs 16.21** for Auto-sklearn 1.0 at a 10-minute budget on 39 test datasets — ~78% relative error reduction.
- **TabRepo** (200 datasets, 1,310 configs, 10 model families, 8-fold bagging, 786k stored prediction sets): a **portfolio of 15 configurations beat AutoGluon** in accuracy *with lower latency and comparable training time*, despite AutoGluon stacking 100+ models. Even a **portfolio of 3** beat every AutoML system except AutoGluon. AutoGluon now ships `zeroshot_portfolio_2025.py` in-repo.
- Because TabRepo stores **raw predictions**, any ensemble's score can be computed by table lookup with no retraining — which is what makes portfolio search tractable.

**Build this.** The platform's experiment DB should store, for every finished trial: config, full learning curve, per-fold **raw validation predictions**, resource usage, and the task signature. That last item (raw predictions) is what unlocks zero-cost ensemble simulation and portfolio mining offline.

### 3.3 Transfer BO across tasks

Given a new task `t*` and history on tasks `t_1..t_M`:

- **RGPE (ranking-weighted GP ensemble)**: fit a GP per past task; weight each by its *ranking agreement* with observations on `t*` (fraction of pairwise orderings it gets right); predict as the weighted mixture; weights shift to the target GP as data accrues. Robust, simple, the default "just works" transfer method. Avoids the scale-mismatch problem because it uses ranks.
- **Quantile/copula transform (Salinas et al. 2020)**: map each task's observed objectives to their **quantiles within that task**, then fit a *single* joint model across all tasks on the transformed scale. This fixes the #1 practical obstacle to transfer — different tasks have wildly different metric scales and noise levels. Cheap, scales to thousands of past tasks, and is the method I'd implement first.
- **FSBO / deep-kernel surrogates**: meta-train a deep kernel on the meta-dataset, few-shot adapt on the target. Better asymptotics, more machinery, needs a big meta-dataset.
- **MALIBO**: meta-learns a likelihood-free (density-ratio / classifier) acquisition, avoiding GP scaling entirely.
- **Bounding-box transfer** (Syne Tune `BoundingBox`): the cheapest transfer trick — restrict the new search space to the bounding box of the best configs across past tasks. Dumb, fast, and often captures most of the gain. **Use this as your baseline transfer method**; anything fancier must beat it.
- **ZeroShotTransfer** (Syne Tune): deterministic replay of a meta-learned portfolio — i.e. §3.2 as a scheduler.
- **ASHA-CTS**: ASHA + cross-task-similarity-based sampling; multi-fidelity + transfer combined.

### 3.4 Learned optimizers / "learning to optimize"

**OptFormer** (Google, 2022) trained a transformer on Vizier's historical study database to imitate/improve upon multiple optimizers *and* predict objective values — a single model that can emulate different HPO policies conditioned on textual metadata.

**ZeroShotOpt** (2026) is the state of this line: a **200M-parameter** transformer trained with offline RL (Decision-Transformer style) on **~20M synthetic optimization trajectories** from **1.6M GP-sampled functions** (78 kernel combinations), generated by running 12 BO variants — ~150,000 vCPU-hours of data generation. It handles 2D–20D with budgets ≤50 evaluations. Results: best normalized performance on synthetic GP/BBOB/VLSE suites (0.647–0.881 vs 0.629–0.878 for the best GP baseline), rank #1 on out-of-distribution BBOB/VLSE. On real HPO-B it is behind the best BO (0.885 vs 0.915) zero-shot but nearly matches after fine-tuning (0.913). **The headline practical number is speed: 5.56 s vs 87.75 s per decision at 20D.**

**Verdict for the platform (Sept 2026):** learned optimizers are not yet a drop-in replacement for GP-BO on *accuracy*, but they are 10–15× cheaper per decision and they are the natural way to consume your own experiment history. The pragmatic move is: use PFN-style learned surrogates where they are already clearly SOTA — **learning-curve extrapolation (LC-PFN/FT-PFN)** — and use classical transfer (quantile-transform + RGPE) for config-space transfer.

### 3.5 Concrete warm-start recipe for a NEW task

```
1. Embed the task: LLM embedding of (description, model family, dataset card) ⊕ (n, d, task_type, metric).
2. Retrieve the k=20 most similar past tasks from the experiment DB.
3. Quantile-transform each retrieved task's objective values to within-task quantiles.
4. Emit the initial design:
     - 8–16 configs from the offline greedy portfolio (task-family-specific if one exists),
     - + 2–4 configs = per-task incumbents of the top-3 most similar tasks,
     - + 2 random configs (surrogate-hostility canary, §1.1).
5. Run that design under ASHA (η=3, grace_period from the stored rank-correlation table).
6. Fit the transfer surrogate (RGPE over retrieved tasks + target GP/TPE) on the quantile scale.
7. Hand the remaining budget to the model-based searcher; refresh the LLM-proposed prior for
   PriorBand from the top-5 observed configs.
8. On completion, write back: config, full curves, raw val predictions, resources, rank-correlation
   per rung, and the final task embedding. This closes the loop.
```

---

## 4. AutoML systems in 2025–26: have foundation models displaced classic AutoML on tabular?

**Short answer: for small-to-medium tabular data, largely yes as single models — but the winning production answer is a foundation model *inside* an AutoML ensemble.**

### 4.1 The tabular foundation models

| model | max rows | max features | max classes | license notes |
|---|---|---|---|---|
| Mitra | 10,000 | 500 | 10 | Apache-2.0 |
| TabPFNv2 (Real-) | 10,000 | 500 | 10 | free for commercial |
| TabPFN-2.5 | 50,000 | 2,000 | — | commercial license |
| TabPFN-2.6 | 100,000 | unlimited | 10 | commercial license |
| **TabPFN-3** | 500,000 (AG integration); **1M rows / 200 feats** per its own report | unlimited | 160 | commercial license |
| TabICLv2 | 500,000 | 2,000 | unlimited | BSD-3 |
| TabDPT | 100,000 | 2,500 | 160 | Apache-2.0 |
| Nori | 50,000 | unlimited | unlimited | Apache-2.0, **regression only** |

(Row/feature limits as surfaced by AutoGluon's integration; model reports quote larger native limits.)

**TabPFN-2.5** (Nov 2025): up to 50k rows / 2k features (20× the data cells of v2); on TabArena it **matches AutoGluon 1.4's four-hour tuned ensemble** in a single forward pass; 100% win rate vs default XGBoost on ≤10k×500 classification, 87% on larger. Ships a **distillation engine** that compiles the ICL model into a compact MLP or tree ensemble, preserving most accuracy at orders-of-magnitude lower latency — this is the answer to "ICL inference is too slow/expensive for serving."

**TabPFN-3** (2026): up to **1M rows / 200 features**, **many-class** (ranks first on multi-class), extends to **time-series (TabPFN-TS-3, 2nd on fev-bench), relational (SOTA on RelBenchV1), and tabular-text**. Adds **test-time compute scaling** ("Thinking mode", TabPFN-3-Plus). Up to **20× faster** than 2.5 via reduced KV cache + row chunking (1M-row inference on a single H100). TabPFN-3-Plus beats a 4-hour-tuned AutoGluon 1.5 ensemble by **200+ Elo** on standard benchmarks and **420 Elo** on the largest datasets, while being **10× faster**; it outperforms 8-hour-tuned GBDT baselines on 1M-row datasets.

**TabArena** (living benchmark, NeurIPS 2025 D&B; 51 curated datasets) is the reference scoreboard. As of the 2026 board: AutoGluon (4h ensemble) Elo ≈1695, **TabPFN-3 ≈1673 (statistically tied, and it is a single forward pass)**, TabPFN-2.6 ≈1624, tuned+ensembled LightGBM ≈1433, XGBoost ≈1375. On the ≤10k-sample slice (36/51 datasets): TabPFN-3 1642 vs AutoGluon 1643 vs LightGBM 1389 — a **253-Elo gap** over the best GBDT. The top four *single* models on the 2026 board are all tabular foundation models. Inference: TabPFN-3 ≈0.58 s/1k rows vs LightGBM ≈2.64 s/1k rows.

**Where GBDTs still win:** CPU-only deployment, datasets beyond context limits, cost-per-inference without a GPU, operational simplicity, and anything requiring bespoke objectives/monotonic constraints/native categorical handling at scale.

### 4.2 AutoGluon — still the system to beat

Current line is **1.6.x** (1.5 stable / 1.6 dev in the docs at time of writing). Trajectory:
- **1.4**: baseline with the `extreme` preset and early foundation-model integration.
- **1.5**: added RealTabPFN-2.5, TabDPT, TabPrep-LightGBM; **70% win rate vs 1.4** on TabArena, 2.8% mean relative error reduction.
- **1.6**: adds **Nori, TabPFN-3, TabDPT-Turbo, TabPFN-2.6, TabICLv2**; new `extreme` preset with **67% win rate vs 1.5's `extreme` at 27× faster training and 4× faster inference**; a `noncommercial` preset that includes TabPFN-3; a **`validation_structure`** parameter for grouped/temporal (non-IID) splits across all validation splits; and **TabPrep** feature engineering (group-by aggregates, random-subset compression, target encoding) applied selectively per model.

AutoGluon's mechanism is *not* HPO. It is: fixed zero-shot **portfolio** of configs (§3.2) → multi-layer **stacking** with out-of-fold predictions → **greedy weighted ensemble** (Caruana) on the final layer. The `validation_structure` addition in 1.6 is the single most important feature for a serious platform, because leakage through IID splits on grouped/temporal data is the most common way production AutoML silently lies.

### 4.3 The others

- **H2O AutoML**: solid, JVM-native, great for enterprise/Spark shops; typically behind AutoGluon on benchmarks. Choose for operational reasons, not accuracy.
- **FLAML** (Microsoft): **CFO** (cost-frugal randomized direct search with a cost model) and **BlendSearch** (BO for global + CFO for local). Its pitch is *cost-awareness* — it prefers cheap configs early and escalates, which gives unusually good anytime performance on a wall-clock budget. Its `tune` module composes with Ray Tune. Best pick when the binding constraint is *dollars/minutes*, not final accuracy.
- **Auto-sklearn**: still the cleanest reference implementation of portfolio + meta-learning + post-hoc ensembling; the 2.0 design (§3.2) is the intellectual template even where the code isn't used.
- **AutoKeras / NAS-for-vision-text**: largely superseded by fine-tuning pretrained backbones. Do not build NAS into a general platform in 2026; support "pick a backbone + tune the fine-tuning recipe" instead.

### 4.4 Verdict

For a production platform, the tabular default should be:
```
if n_rows <= 500k and n_features <= 2000:
    run TabPFN-3 / TabICLv2 / Mitra (license-permitting) as one forward pass  -> baseline B0
    run AutoGluon `extreme` (which already includes the FMs + portfolio + stacking) -> B1
    if B1 - B0 < practical_delta: ship the distilled FM (cheap serving)
    else: ship B1
    tune only if both plateau, and then tune the *ensemble/portfolio*, not individual model HPs
else:
    AutoGluon best_quality (GBDT-heavy portfolio) + targeted HPO on the top-2 model families
```
Licensing is a first-class gate: TabPFN ≥2.5 requires a commercial license; Mitra/TabICL/TabDPT/Nori are permissive. Encode this as a hard constraint in the agent's model-selection policy.

---

## 5. Declaring "A is better than B" defensibly

### 5.1 The problem

Bouthillier et al. (*Accounting for Variance in Machine Learning Benchmarks*, MLSys 2021) enumerate the variance sources: **data sampling (largest)**, hyperparameter optimization, weight init, data ordering, augmentation/dropout/numerical noise. Comparing a single seed of A against a single seed of B measures noise.

The "gains are noise" literature is consistent across fields: Dacrema et al. (RecSys 2019) could reproduce only a handful of ~18 neural recommenders, and most were beaten by properly-tuned simple baselines; similar audits exist in NLP, RL and metric learning. The common root cause is not fraud — it is **tuning A harder than B and reporting the max**.

### 5.2 The protocol

**Bouthillier's recommendations, concretely:**
- **Randomize as many sources as possible** rather than fixing seeds. Counterintuitively, adding randomization sources to the *cheap (biased)* estimator makes it approximate the expensive ideal estimator at **51× less compute** (21 vs 1,070 GPU-hours at k=100). The "ideal" estimator (independent HPO per replicate, O(k·T) fits) is unaffordable; the practical one fixes HPs after one HPO run and randomizes everything else — and that is fine.
- **Sample size**: **k ≈ 29 runs** per arm to detect `P(A > B) ≥ 0.75` at α=0.05, β=0.05. If you are declaring wins from 3 seeds, you are guessing.
- **Use out-of-bootstrap resampling** of the data split, not one fixed test set.
- **Pairing**: use identical seeds/splits across A and B to cancel shared variance.
- **Decision rule**: declare A better than B iff
  `P(A > B) ≥ 0.75` **and** `P(A > B) − CI_lower > 0.5`.
  This is a *probability of outperforming*, not a p-value on means, and it is far more robust to non-normal, heavy-tailed metric distributions.

**Multiplicity.** If you compare `m` candidates:
- **Holm–Bonferroni** for strict FWER control when a false positive is expensive (e.g. promoting a model to production).
- **Benjamini–Hochberg (FDR)** when you are *screening* many candidates and can tolerate a known false-discovery proportion — the right tool for "which of my 200 HPO trials are genuinely distinguishable from the incumbent."
- Do **not** use Nemenyi/critical-difference diagrams as the decision instrument. The MCM critique (Ismail-Fawaz et al.) shows CD diagrams are **unstable** (adding or removing an unrelated method can flip the significance of two others), **magnitude-blind** (rank-based, so many trivial wins beat few large ones), and Holm-corrected pairwise significance depends on which other methods you included — i.e. gameable. Their recommendation: report a **Multiple Comparison Matrix** — for every pair, the mean performance difference, win/tie/loss counts, and Wilcoxon p-value, treating p-values as *descriptive* divergence measures. Pairwise outcomes are then invariant to the comparate set.

**Bayesian comparison.** Benavoli/Corani's "Time for a Change" framework replaces NHST with a posterior over the mean difference plus a **ROPE** (region of practical equivalence). You get `P(A ≫ B)`, `P(A ≈ B)`, `P(B ≫ A)` directly. The Bayesian correlated t-test corrects for the dependence induced by overlapping CV folds — a correction the plain paired t-test on CV folds *needs* and does not have. **This is the right default for a platform**, because it makes the practically-important threshold explicit rather than hiding it: you must name a `δ_practical` (e.g. "+0.2% AUC or it doesn't count"), and everything else follows.

**Sequential / always-valid inference.** In a platform, you *peek* — the agent watches results stream in and wants to stop early. Fixed-sample p-values are invalid under optional stopping. Use:
- **e-values / test martingales**: an e-value `E` with `E[E] ≤ 1` under H₀ gives `P(sup_t E_t ≥ 1/α) ≤ α` (Ville's inequality) — you may stop **whenever you like** and reject at `E ≥ 1/α`. They multiply across independent experiments, which makes evidence accumulation across a campaign trivial, and they combine cleanly with **e-BH** for FDR control under optional stopping.
- **Confidence sequences**: `CI_t` valid *uniformly over all t*. Betting-based constructions (Waudby-Smith & Ramdas, JRSS-B 2024, "Estimating means of bounded random variables by betting") are variance-adaptive and near-optimal for bounded metrics (accuracy, AUC, any [0,1] score) — exactly our case. Reference implementation: `gostevehoward/confseq`.
- The 2025 SAVI tutorial literature makes these practical; the price is ~10–25% wider intervals than a fixed-n CI at the planned n, in exchange for unlimited peeking.

**Recommended platform protocol for "promote candidate A over incumbent B":**
```
0. Fix δ_practical and the primary metric BEFORE the campaign. Register them.
1. Search phase: any HPO method. Its winner is a CANDIDATE, not a result. Its
   validation score is inadmissible as an estimate (winner's curse, §2.5).
2. Confirmation phase: re-run A and B with k paired replicates, randomizing
   seed, init, data order, augmentation, and the data split (out-of-bootstrap).
   Target k ≈ 20–30. Reuse identical split/seed pairs for A and B.
3. Evaluate on a held-out test set untouched by search.
4. Decide with an anytime-valid confidence sequence on the paired difference
   (betting-based, bounded metric) + a ROPE of ±δ_practical:
      promote if CI_lower(t) > +δ_practical
      reject  if CI_upper(t) < +δ_practical
      else keep sampling (up to k_max), then declare "practically equivalent"
5. If >1 candidate is being compared to B: Holm for promotion decisions,
   BH/e-BH for screening.
6. Log the full comparison (all replicate scores, seeds, splits) to the experiment DB.
```
The key discipline: **the search loop and the decision loop are separate, use different data, and different statistics.**

---

## 6. Resource allocation policy

### 6.1 Bandit framing

Successive halving is **non-stochastic infinite-armed best-arm identification** (Jamieson & Talwalkar, AISTATS 2016): arms = configurations, pulling arm `i` at budget `r` reveals `ℓ_i(r)` which converges to `ν_i` at an unknown rate. Hyperband is a hedge over the unknown budget-vs-arms trade-off with an `O(log n)` factor loss relative to knowing it. ASHA is the asynchronous relaxation that trades a small amount of statistical efficiency for near-perfect worker utilization (95.7% at 1,280 workers).

Useful mental model for the agent: **at every scheduling decision, you are choosing between (a) more breadth (new arm at low budget) and (b) more depth (promote an existing arm).** ASHA's fixed `1/η` promotion rule is a crude but robust answer. A better answer, when you have a learning-curve posterior, is an explicit value-of-information calculation:

```
score(action a) = E[ improvement in final incumbent | a ] / cost(a)

# depth on trial i to budget r':
#   E[ max(0, f_i(r') - incumbent) ]   from the LC-PFN posterior
# breadth (new config x):
#   E[ max(0, f(x, r_min) ... ] via the config-space surrogate, marginalized over curves
```
This is exactly what freeze-thaw BO / ifBO's MFPI-random approximates, and it is the principled version of "give extra compute to promising branches."

### 6.2 Killing unproductive work — a layered policy

Apply these in order; each is cheap and catches a different failure:
1. **Hard guards (immediate kill):** NaN/Inf loss, loss > 3× initial after 5% of training, OOM, grad-norm explosion beyond a threshold, throughput below X% of the expected rate (data-loader starvation), zero-variance predictions. These are *bugs*, not bad hyperparameters, and should surface to the agent as a diagnostic, not as a data point for the surrogate.
2. **Rung-based (ASHA / PASHA)** with smoothed metrics and noise-aware soft ranking.
3. **Curve-based (LC-PFN posterior)**: kill when `P(final > incumbent + δ_practical) < 0.05`.
4. **Statistical (Wilcoxon pruner)** when replicates exist.
5. **Cost-aware override:** never kill a trial whose *remaining* cost is below the cost of starting a new one (checkpoint + warmup + low rungs). Conversely, kill aggressively when the GPU queue is deep.

### 6.3 Diminishing-returns / campaign stopping

Optuna's `Terminator` formalizes the right criterion: **stop when the estimated remaining improvement falls below the statistical error of your own evaluation.** Concretely it pairs an *improvement evaluator* (a regret bound / best-value stagnation / Expected Minimum Model Regret over the GP posterior) with an *error evaluator* (cross-validation error across folds, or a static constant), and terminates when `E[improvement] ≤ statistical_error`. (Note: `optuna.terminator` is deprecated in 4.9 and moves to OptunaHub — reimplement rather than depend on it.)

Implement this as the campaign-level stop rule:
```
stop the campaign when ANY of:
  (a) E[simple regret remaining]  <=  SE(val metric across folds/seeds)     # Optuna Terminator logic
  (b) E[simple regret remaining]  <=  delta_practical                        # you wouldn't ship the gain
  (c) incumbent unchanged for  max(20, 0.25 * trials_so_far)  trials
  (d) budget/deadline exhausted
  (e) marginal improvement per GPU-hour < the org's threshold (dollars-per-point)
```
(b) and (e) are the ones that matter commercially and are almost never implemented. An agent operator should report, at stop time: incumbent, anytime-valid CI on the incumbent-vs-baseline difference, GPU-hours spent, and the estimated remaining regret — so a human can decide whether to buy more search.

### 6.4 Allocation across *candidates*, not just trials

At the campaign level you are also allocating between qualitatively different branches (architecture A vs B, dataset v1 vs v2, loss L1 vs L2). Treat this as a nested bandit: each branch is an arm whose reward is its own incumbent's improvement rate. Allocate with a simple non-stationary rule (discounted UCB or Thompson sampling over branch improvement rates), rebalancing at fixed checkpoints. Enforce a floor (each live branch keeps ≥5% of workers) so a slow-starting branch is not starved — this is the multi-armed analogue of Hyperband's bracket hedging.

---

## 7. Ensembling and post-hoc optimization

Post-hoc steps are where the *reliable* remaining gains are: they are cheap, low-risk, and their benefit does not shrink as base models improve nearly as fast as HPO gains do.

### 7.1 Greedy ensemble selection (Caruana)

```
greedy_ensemble_selection(preds[K][n], y, iters T, with_replacement=True, bagging=0.5):
    ensemble = []                     # multiset of model indices
    for t in 1..T:                    # T ~ 25-100
        pool = random_subset(models, frac=bagging)    # Caruana's bagging trick
        best = argmin_{c in pool} loss(y, mean(preds[ensemble + [c]]))
        ensemble.append(best)
    return weights = counts(ensemble) / T
```
Properties that make it the AutoML standard (Auto-sklearn, AutoGluon):
- **with replacement** → implicit non-negative, sum-to-one weights;
- weights are **pseudo-discrete** (multiples of `1/T`) and **sparse** — these two implicit constraints are precisely what prevents validation overfitting;
- monotone non-increasing validation loss; trivially parallel; works on stored predictions, so it needs **no retraining**.

The CMA-ES-vs-GES study (2023) is the clean experiment: unconstrained CMA-ES over ensemble weights **beats** GES on balanced accuracy but **badly overfits validation for ROC AUC** — GES overtakes it when moving validation→test. Restoring GES's implicit constraints explicitly (rounding weights to discrete fractions + sparsity, "CMA-ES-ExplicitGES") makes CMA-ES match GES without the overfitting. **Recommendation: use GES as the default; if you use continuous weight optimization, impose discreteness + sparsity + a separate ensembling holdout.** Auto-sklearn's 33% single holdout is noted as more overfit-prone than proper CV-based out-of-fold predictions — so **always ensemble on out-of-fold predictions, never on a single small holdout.**

### 7.2 Stacking

AutoGluon's multi-layer stacking: layer-1 models produce out-of-fold predictions; layer-2 models train on `[original features ⊕ OOF preds]`; final layer is a weighted (GES) ensemble. Non-negotiable rules:
- Use **out-of-fold** predictions everywhere, with the *same* fold assignment across all base models.
- Respect group/temporal structure in the folds (AutoGluon 1.6's `validation_structure`). Random K-fold on grouped or time-series data leaks and will make stacking look brilliant and deploy terribly.
- Bag the base models (AutoGluon's default 8-fold bagging) so OOF preds are not from a single fit.
- `PSEO` (2025) explores treating the post-hoc stacking ensemble's own structure as a hyperparameter search problem — a reasonable direction once base-model gains are exhausted.

### 7.3 Snapshot ensembles, checkpoint averaging, model soups

- **Snapshot ensembles**: cyclic LR schedule, save a checkpoint at each cycle's minimum, ensemble the *predictions* of M snapshots. Cost: one training run. Gain: most of a deep ensemble's accuracy benefit, less of its calibration/diversity benefit (snapshots are correlated — same basin lineage). Inference cost is M×.
- **Weight averaging (SWA / EMA)**: average *weights* along the trajectory. Free at inference. Reliable small gains; essentially mandatory for modern LLM/vision training.
- **Model soups** (Wortsman et al., ICML 2022): average the weights of multiple models fine-tuned from the **same pretrained initialization** with *different hyperparameters*. **Greedy soup**: sort candidates by validation accuracy, add each to the soup only if it improves validation accuracy of the average. This is the perfect post-hoc step for an HPO platform, because an HPO sweep over fine-tuning HPs produces exactly the required ingredient set **for free** — the runs you were going to throw away become the soup. Gains hold in- and out-of-distribution, with zero inference cost.
  - Hard precondition: all ingredients must be linearly mode-connected — same pretrained init, no architecture changes, same tokenizer/head shape. Averaging across different inits produces garbage.
- **WiSE-FT**: interpolate between the zero-shot/pretrained weights and the fine-tuned weights (`θ = (1−α)θ_pre + α θ_ft`); tune α on validation. Robustness-accuracy trade-off dial, costs one scalar search.

**Ordering for an agent:** greedy soup (free at inference) → prediction ensemble/GES (M× inference but best accuracy) → stacking (needs OOF infra). Choose based on the serving budget, which the platform knows.

### 7.4 Probability calibration

The current best evidence is **CalArena** (2026): ~2000 experiments over TabRepo (832 binary + 520 multiclass), TabArena (314 binary + 84 multiclass), and CV (13 binary + 20 multiclass + 8 ImageNet), covering **30+ calibration methods** — temperature, Platt, Platt-on-logits, beta, quadratic scaling, isotonic, Venn-Abers, splines, histogram binning, tree-based calibrators, and the multiclass natives (matrix/vector/**structured matrix scaling (SMS)**, Dirichlet) plus one-vs-rest variants. Their evaluation metric is **Post-Hoc Improvement (PHI) in Brier score**, deliberately preferred over ECE because ECE rewards binning artifacts and ignores whether you damaged discrimination.

Findings:
- **Binary**: quadratic scaling, **Platt-on-logits**, and **beta calibration** dominate — logistic-family transformations applied to logits.
- **Multiclass, few classes**: **SMS** leads; spline-OvR competitive.
- **Multiclass, many classes**: **SMS overwhelmingly superior; OvR approaches fail at scale.** Native multiclass methods are essential above roughly 4 classes.
- **Smooth calibration maps beat binning-based methods**, even though binning flatters simple calibration metrics.
- Generic ML regressors (gradient boosting as a calibrator) underperform purpose-built calibrators lacking calibration-specific constraints.

Practical rules:
- Always calibrate on **held-out data not used for training or ensembling** (a dedicated calibration split, or nested CV).
- **Isotonic** is non-parametric and powerful but needs data: below ~1,000 calibration points it overfits badly; prefer Platt/beta/temperature. Above ~5,000, isotonic is safe but per CalArena still usually loses to smooth logistic-family maps.
- **Temperature scaling** (single scalar on logits) is the minimum viable option for neural nets: it cannot change the argmax, so accuracy is preserved exactly. Use it when you must not perturb ranking.
- **Beta calibration** handles the asymmetric miscalibration typical of bagged/boosted models better than Platt (which assumes a symmetric sigmoid).
- **Venn-Abers** gives *interval-valued*, validity-guaranteed probabilities (distribution-free) — the right choice when you need calibration **guarantees** rather than good average calibration (regulated settings). Implementation: `ip200/venn-abers`.
- Ensembling changes calibration: a GES ensemble of calibrated models is not itself calibrated. **Calibrate last**, after ensembling.

### 7.5 Decision-threshold optimization under asymmetric costs

Distinct from calibration and often confused with it. Given a cost matrix, the Bayes-optimal threshold on a *calibrated* probability is closed-form for the binary two-cost case:
```
t* = (C_fp - C_tn) / (C_fp - C_tn + C_fn - C_tp)
```
With a general utility, tune the threshold empirically to maximize expected utility on a held-out split. scikit-learn ≥1.5 provides `TunedThresholdClassifierCV` (`sklearn.model_selection`), which cross-validates the threshold against any scorer — including a custom cost-matrix scorer via `make_scorer` — and supports `cv="prefit"`; the docs' *"Post-tuning the decision threshold for cost-sensitive learning"* example is the canonical reference.

Rules:
- **Never tune the threshold on the training or the model-selection split.** It is a parameter; it overfits like any other.
- **Calibrate first, then threshold.** A well-calibrated model plus the closed-form threshold beats an uncalibrated model plus a tuned threshold in almost every case, and generalizes when costs change (you recompute `t*`, no refit).
- If costs are **instance-dependent** (fraud amount, patient risk), do not use a global threshold: maximize expected utility per instance, `argmax_a Σ_y p(y|x)·U(a,y)`.
- If the deployment **prevalence** differs from training prevalence, adjust with prior correction (`p' ∝ p·(π'/π)`) before thresholding rather than re-tuning the threshold on mismatched data.
- Report the threshold and the cost matrix as versioned model artifacts. A model whose threshold was tuned against a stale cost matrix is a silent production failure.

---

## 8. Concrete platform design implications

1. **Experiment DB schema is the moat.** Store per trial: config (with the space definition it came from), **full learning curve at every logged step**, **raw per-fold validation predictions**, resource usage + wall-clock, code/data version, seed, failure class, and the task signature. Raw predictions enable zero-cost ensemble/portfolio simulation offline (TabRepo's whole trick). Curves enable LC-PFN/FT-PFN warm-starting and per-rung rank-correlation tables.
2. **Two loops, two statistics.** A *search* loop (biased, greedy, multi-fidelity, optimistic) and a *decision* loop (paired replicates, randomized nuisance sources, anytime-valid CIs, ROPE). Never let a search-loop number be a reported number.
3. **Portfolio-first.** Mine a greedy submodular portfolio nightly from the DB, per task family. The agent's first move on a new task is to run the portfolio, not to start a search. Expect this alone to recover most of the achievable gain (Auto-sklearn 2.0: 78% relative error reduction; TabRepo: 15 configs > AutoGluon).
4. **ASHA as the default scheduler**, `η=3`, smoothed rung metrics, `grace_period` from the stored rank-correlation table, PASHA when `max_t` is unknown. Add ifBO/LC-PFN stopping on top once curve logging is reliable.
5. **Resumable checkpoints are an optimizer feature, not an ops feature.** Freeze-thaw, PBT and pause/resume schedulers are all gated on them.
6. **The LLM operator's job is priors, spaces, diagnostics and narrative** — not numeric optimization. Benchmarks (AgentHPOBench) support exactly this partition.
7. **Post-hoc is not optional**: greedy soup over the sweep's own checkpoints, GES over stored predictions, calibration on a dedicated split, threshold from the cost matrix. These are the cheapest points on the board.
8. **Register `δ_practical` before every campaign.** Every stopping rule, every promotion decision and every "is this noise?" question dissolves once that number exists; none of them are answerable without it.

---

## Sources

**Algorithms & surveys**
- Bischl et al., *Hyperparameter Optimization: Foundations, Algorithms, Best Practices and Open Challenges* (WIREs DMKD 2023) — https://arxiv.org/abs/2107.05847 · https://wires.onlinelibrary.wiley.com/doi/10.1002/widm.1484
- Jamieson & Talwalkar, *Non-stochastic Best Arm Identification and Hyperparameter Optimization* (AISTATS 2016) — http://proceedings.mlr.press/v51/jamieson16.pdf
- Li et al., *A System for Massively Parallel Hyperparameter Tuning* (ASHA, MLSys 2020) — https://arxiv.org/pdf/1810.05934
- Falkner, Klein & Hutter, *BOHB: Robust and Efficient Hyperparameter Optimization at Scale* (ICML 2018) — https://proceedings.mlr.press/v80/falkner18a/falkner18a.pdf · https://www.automl.org/blog_bohb/
- Awad, Mallik & Hutter, *DEHB: Evolutionary Hyperband* (IJCAI 2021) — https://arxiv.org/abs/2105.09821 · https://www.automl.org/dehb/
- Li et al., *Hyper-Tune: Towards Efficient Hyper-parameter Tuning at Scale* (VLDB 2022) — https://www.vldb.org/pvldb/vol15/p1256-li.pdf
- Mallik et al., *PriorBand: Practical HPO in the Age of Deep Learning* (NeurIPS 2023) — https://papers.neurips.cc/paper_files/paper/2023/hash/1704fe7aaff33a54802b83a016050ab8-Abstract-Conference.html
- Hvarfner, Hellsten & Nardi, *Vanilla Bayesian Optimization Performs Great in High Dimensions* (ICML 2024) — https://arxiv.org/abs/2402.02229 · https://arxiv.org/html/2402.02229v5
- Meunier et al., *Improving Nevergrad's Algorithm Selection Wizard NGOpt through Automated Algorithm Configuration* (PPSN 2022) — https://arxiv.org/abs/2209.04412
- Parker-Holder et al., PB2 / *Tuning Mixed Input Hyperparameters on the Fly* (NeurIPS 2021) — https://proceedings.neurips.cc/paper/2021/file/82debd8a12b498e765a11a8e51159440-Paper.pdf
- *Generalized Population-Based Training for HPO in RL* (2024) — https://arxiv.org/html/2404.08233v1
- Morales-Hernández et al., *Multi-Objective HPO in Machine Learning — An Overview* (ACM TELO 2023) — https://dl.acm.org/doi/10.1145/3610536 · https://arxiv.org/html/2206.07438v3
- *Multi-objective Hyperparameter Optimization in the Age of Deep Learning* (PriMO, 2025) — https://arxiv.org/html/2511.08371v1
- BoTorch multi-objective tutorials (qEHVI/qNEHVI/qNParEGO) — https://botorch.org/docs/v0.17.2/tutorials/multi_objective_bo · constrained: https://botorch.org/docs/v0.14.0/tutorials/constrained_multi_objective_bo · https://botorch.org/docs/constraints
- SAASBO (Ax/BoTorch) — https://botorch.org/docs/tutorials/saasbo · https://ax.dev/docs/0.5.0/tutorials/saasbo_nehvi/

**Multi-fidelity & early stopping**
- *A review on multi-fidelity hyperparameter optimization in machine learning* (2025) — https://www.sciencedirect.com/science/article/pii/S2405959525000244
- *Speeding Up HPO of DNNs: A Review of Multi-Fidelity-Based Methods* (ACM CSUR) — https://doi.org/10.1145/3815108
- Adriaensen et al., *Efficient Bayesian Learning Curve Extrapolation using Prior-Data Fitted Networks* (LC-PFN, NeurIPS 2023) — https://arxiv.org/abs/2310.20447 · https://www.automl.org/lc-pfn/
- Rakotoarison et al., *In-Context Freeze-Thaw Bayesian Optimization* (ifBO, ICML 2024) — https://arxiv.org/abs/2404.16795 · https://proceedings.mlr.press/v235/rakotoarison24a.html · code: https://github.com/automl/ifBO
- *Cost-Sensitive Freeze-Thaw Bayesian Optimization* (NeurIPS 2025) — https://papers.nips.cc/paper_files/paper/2025/file/f7b75ff5fa544897ed71217de2aad865-Paper-Conference.pdf
- Bohdal et al., *PASHA: Efficient HPO and NAS with Progressive Resource Allocation* (ICLR 2023) — https://arxiv.org/abs/2207.06940 · https://arxiv.org/pdf/2207.06940
- Golovin et al., *Google Vizier: A Service for Black-Box Optimization* (KDD 2017; median stopping rule) — https://research.google.com/pubs/archive/46180.pdf
- *Open Source Vizier* — https://arxiv.org/pdf/2207.13676
- *A Systematic Study on Early Stopping Metrics in HPO and the Implications of Uncertainty* (VLDB 2025) — https://dl.acm.org/doi/10.14778/3725688.3725689
- Optuna `WilcoxonPruner` — https://optuna.readthedocs.io/en/stable/reference/generated/optuna.pruners.WilcoxonPruner.html · https://medium.com/optuna/wilcoxonpruner-pruning-by-statistical-tests-in-optuna-cd80aad7adbc
- mlr3 asynchronous optimization benchmarks (1,280 workers) — https://mlr-org.com/benchmarks/benchmarks_async.html

**Meta-learning / warm-starting**
- Feurer, Eggensperger et al., *Auto-Sklearn 2.0: Hands-free AutoML via Meta-Learning* (JMLR 2022) — https://arxiv.org/pdf/2007.04074v3 · https://www.automl.org/auto-sklearn-2-0-the-next-generation/
- Salinas, Shen & Perrone, *A Quantile-based Approach for Hyperparameter Transfer Learning* (ICML 2020) — http://proceedings.mlr.press/v119/salinas20a/salinas20a.pdf
- Salinas et al., *TabRepo: A Large Scale Repository of Tabular Model Evaluations* — https://arxiv.org/pdf/2311.02971
- AutoGluon zero-shot portfolios in-repo — https://github.com/autogluon/autogluon/blob/master/tabular/src/autogluon/tabular/configs/zeroshot/zeroshot_portfolio_2025.py
- transfer-HPO framework (RGPE and friends) — https://deepwiki.com/automl/transfer-hpo-framework
- *MALIBO: Meta-learning for Likelihood-free Bayesian Optimization* — https://arxiv.org/html/2307.03565v2
- Jomaa, Schmidt-Thieme & Grabocka, *Dataset2Vec: Learning Dataset Meta-Features* — https://arxiv.org/abs/1905.11063
- *Explaining Tabular Foundation Model Differences Through Meta-Features* (2026) — https://arxiv.org/html/2605.28418
- Chen et al., *OptFormer: Towards Learning Universal Hyperparameter Optimizers with Transformers* — https://arxiv.org/abs/2205.13320
- *ZeroShotOpt: Towards Zero-Shot Pretrained Models for Efficient Black-Box Optimization* (2026) — https://arxiv.org/html/2510.03051
- Syne Tune (AWS) — https://syne-tune.readthedocs.io/ · https://github.com/syne-tune/syne-tune · paper: https://proceedings.mlr.press/v188/salinas22a/salinas22a.pdf

**AutoML systems & tabular foundation models 2025–26**
- TabArena: *A Living Benchmark for Machine Learning on Tabular Data* (NeurIPS 2025 D&B) — https://arxiv.org/html/2506.16791v1 · https://papers.neurips.cc/paper_files/paper/2025/file/1697e3fb412da11dc9488249f9e7bbc9-Paper-Datasets_and_Benchmarks_Track.pdf
- TabArena 2026 leaderboard snapshot — https://www.codesota.com/tasks/tabular-ml
- *TabPFN-2.5: Advancing the State of the Art in Tabular Foundation Models* — https://arxiv.org/abs/2511.08667 · https://priorlabs.ai/technical-reports/tabpfn-2-5-model-report
- *TabPFN-3: Technical Report* — https://priorlabs.ai/technical-reports/tabpfn-3 · https://arxiv.org/pdf/2605.13986 · https://docs.priorlabs.ai/changelog/tabpfn-3
- AutoGluon release notes (1.4 → 1.6) — https://auto.gluon.ai/dev/whats_new/index.html
- AutoGluon tabular foundation models (limits + licensing table) — https://auto.gluon.ai/dev/tutorials/tabular/tabular-foundational-models.html
- FLAML (CFO / BlendSearch) — https://microsoft.github.io/FLAML/docs/Research/ · https://github.com/microsoft/FLAML/tree/main/flaml/tune
- Optuna releases (v4.9.0, deprecations) — https://github.com/optuna/optuna/releases
- Optuna `AutoSampler` — https://medium.com/optuna/autosampler-full-support-for-multi-objective-constrained-optimization-c1c4fc957ba2
- Optuna `Terminator` — https://optuna.readthedocs.io/en/stable/reference/terminator.html
- SMAC3 — https://github.com/automl/SMAC3 · https://automl.github.io/SMAC3/
- Ray Tune schedulers — https://docs.ray.io/en/latest/tune/api/schedulers.html · FAQ: https://docs.ray.io/en/latest/tune/faq.html

**Statistics of comparison**
- Bouthillier et al., *Accounting for Variance in Machine Learning Benchmarks* (MLSys 2021) — https://arxiv.org/abs/2103.03098 · https://arxiv.org/pdf/2103.03098
- Benavoli, Corani, Demšar & Zaffalon, *Time for a Change: a Tutorial for Comparing Multiple Classifiers Through Bayesian Analysis* (JMLR 2017) — https://www.jmlr.org/papers/volume18/16-305/16-305.pdf · https://arxiv.org/abs/1606.04316
- Ismail-Fawaz et al., *An Approach to Multiple Comparison Benchmark Evaluations that is Stable Under Manipulation of the Comparate Set* (MCM) — https://arxiv.org/pdf/2305.11921
- Ferrari Dacrema, Cremonesi & Jannach, *Are We Really Making Much Progress?* (RecSys 2019) — https://arxiv.org/abs/1907.06902v3
- Ramdas et al., *Game-Theoretic Statistics and Safe Anytime-Valid Inference* (Statistical Science 2023) — https://projecteuclid.org/journals/statistical-science/volume-38/issue-4/Game-Theoretic-Statistics-and-Safe-Anytime-Valid-Inference/10.1214/23-STS894.pdf
- Waudby-Smith & Ramdas, *Estimating Means of Bounded Random Variables by Betting* (JRSS-B 2024) — https://academic.oup.com/jrsssb/article-abstract/86/1/1/7043257
- *A Tutorial on Safe Anytime-Valid Inference* (2025) — https://www.alexander-ly.com/wp-content/uploads/2025/08/saviTutorial.pdf
- Benjamini & Hochberg (1995), FDR — https://academic.oup.com/jrsssb/article/57/1/289/7035855
- `confseq` reference implementation — https://github.com/gostevehoward/confseq

**Ensembling, calibration, thresholds**
- Caruana et al., *Ensemble Selection from Libraries of Models* (ICML 2004) — https://www.researchgate.net/publication/221345642_Ensemble_Selection_from_Libraries_of_Models
- Purucker & Beel, *CMA-ES for Post Hoc Ensembling in AutoML: A Great Success and Salvageable Failure* — https://arxiv.org/html/2307.00286
- *PSEO: Optimizing Post-hoc Stacking Ensemble Through Hyperparameter Tuning* (2025) — https://arxiv.org/html/2508.05144v1
- Wortsman et al., *Model Soups* (ICML 2022) — https://arxiv.org/pdf/2203.05482 · https://github.com/mlfoundations/model-soups
- Berta et al., *CalArena: A Large-Scale Post-Hoc Calibration Benchmark* (2026) — https://arxiv.org/html/2605.30188
- Venn-ABERS implementation — https://github.com/ip200/venn-abers
- scikit-learn `TunedThresholdClassifierCV` — https://scikit-learn.org/stable/modules/generated/sklearn.model_selection.TunedThresholdClassifierCV.html · cost-sensitive example: https://scikit-learn.org/stable/auto_examples/model_selection/plot_cost_sensitive_learning.html

**LLM agents as optimizers**
- Liu et al., *AgentHPO: Large Language Model Agent for Hyper-Parameter Optimization* (CPAL 2025) — https://arxiv.org/abs/2402.01881 · https://proceedings.mlr.press/v280/liu25c.html
- *AgentHPOBench: A Benchmark For Evaluating LLM Agents as Sequential Hyperparameter Optimizers* (2026) — https://arxiv.org/abs/2607.29626
- *LLMSYS-HPOBench: HPO Benchmark Suite for Real-World LLM Systems* (2026) — https://arxiv.org/abs/2605.08305 · https://arxiv.org/html/2605.08305v1
