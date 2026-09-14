# 05 — Vector/Matrix Representations of Assets, Regimes, Configurations and Outcomes

**Scope:** how to represent assets, market regimes, strategy configurations and experiment outcomes as vectors/matrices/tensors on a multi-tenant, multi-asset (crypto / futures / options / ETF / DeFi), minute-bar quant platform — and the mathematics for mining an accumulated trial ledger to steer new experiments.

**Date of research:** 2026-09-12/13. Evidence cut: papers and benchmarks through Sep 2026.

**Epistemic key used throughout:**
- **[WS] Well-supported** — replicated, or supported by controlled benchmark/simulation with public code, or a theorem.
- **[PS] Partially supported** — one credible study, or strong in-domain evidence but not in *our* domain (minute bars, multi-asset, real capital).
- **[SPEC] Speculative** — plausible architecture/mechanism, no direct evidence. Build behind a flag; measure.

---

## 0. Executive posture (read this before designing anything)

Five findings dominate every design choice below.

1. **Minute-bar financial features are computable and stable; minute-bar financial *alpha* is not.** The most directly relevant 2026 study — 3.4M minute observations, 6 major crypto pairs, Binance spot + perps, Aug 2025–Feb 2026 — found that all 12 classical microstructure features (Corwin–Schultz spread, realized vol, VPIN proxy, Kyle's λ, Amihud, OFI, trade intensity) *pass stability selection*, yet a properly purged/embargoed OLS gained only **+1.23% R²** over random walk (not significant), LightGBM went to **−10.94% R²** once temporal purging was enforced, and net Sharpe after realistic Binance VIP-0 fees was **−52.05 (spot) / −18.42 (futures)** ([Frontiers in Blockchain 2026](https://www.frontiersin.org/journals/blockchain/articles/10.3389/fbloc.2026.1811716/full)). **[WS]** Implication: build the feature vectors as *descriptors for routing and retrieval*, never as a standalone alpha claim.

2. **The same study found cross-asset transfer fails but cross-venue transfer succeeds**: "models trained on one cryptocurrency do not transfer to others, although they transfer well between the spot and futures venues of the same asset." **[PS]** This is the single most important structural fact for an asset-embedding space: the useful neighborhood metric is probably *(instrument family, venue)*, not *(statistical profile)* alone. Design the space so venue/underlying identity is a first-class coordinate, not something the embedding has to rediscover.

3. **Frozen time-series-foundation-model embeddings are not yet a free win.** The one 2026 study that evaluates TSFM *embeddings* rather than forecasts (Chronos-2 and MOMENT, global-average-pooled to 512-d) found frozen embeddings "fail to offer consistently strong performance"; fine-tuned Chronos-2 beat an MLP baseline by 28% F1, and the best model was **fusion** of MLP features + Chronos-2 embeddings (F1 .876 vs .839 fine-tuned TSFM vs .704 MLP) ([arXiv 2606.27672](https://arxiv.org/html/2606.27672)). **[PS]** Plan for *concatenation with hand-crafted features*, not replacement.

4. **Benchmark numbers for TSFMs are contaminated.** An audit of 22 published TSFMs against 401 datasets found **only 6% of datasets were unused by any model**; a controlled experiment showed that adding 7 correlated stock indices (out of 484 series) to training improved S&P 500 forecasting by **~37% during market crashes** despite the S&P 500 never appearing in training ([arXiv 2510.13654](https://arxiv.org/html/2510.13654v3)). TSFMAudit independently estimates contamination rates of **85–96% for Moirai-1/2 and Kairos, 14% for Chronos and TiRex, 9% for TimesFM-2.0** ([arXiv 2605.26161](https://arxiv.org/html/2605.26161)). **[WS]** Never accept a leaderboard rank as evidence for your data.

5. **Portfolios beat meta-features.** Auto-sklearn 2.0 *deliberately removed* meta-features (including landmarkers) and replaced them with a static greedy-selected portfolio of 32 pipelines, citing cost, complexity, coverage gaps, and lack of guidance on which meta-features work ([Auto-sklearn 2.0, JMLR](https://ml.informatik.uni-freiburg.de/wp-content/uploads/papers/21-ARXIV-ASKL2.pdf)). TabRepo confirms: a **3-config** portfolio beats most AutoML systems and a **15-config** portfolio beats AutoGluon, saturating by ~150 ([arXiv 2311.02971](https://arxiv.org/html/2311.02971v3)). **[WS]** Your "landmarker" budget should be spent on *a small complementary portfolio of cheap strategies*, chosen by greedy submodular coverage of your own trial ledger — not on a taxonomy of statistical meta-features.

**Consequence for build order:** hand-crafted fingerprint + landmarker portfolio first; outcome-tensor factorization second; learned/foundation embeddings last and only as an additive channel with a measured lift gate.

---

## 1. Asset fingerprinting from minute bars

### 1.1 The estimator set — with the 2024–2026 corrections

The classical list in the brief is right, but three items have been superseded and two are near-useless as usually computed.

**Spread estimation — use EDGE, retire Roll and Corwin–Schultz.**
Ardia, Guidotti & Kroencke (JFE 2024) derive the **EDGE** estimator from OHLC prices under discrete trading. Simulation at a 0.50% true spread with a monthly window: RMSE **EDGE ≈ 0.38%**, Corwin–Schultz ≈ 0.57%, Abdi–Ranaldo ≈ 0.83%, Roll ≈ 1.72%. Empirically vs TAQ effective spreads 1993–2020: **EDGE 1.22%, AR 1.39%, CS 2.08%** RMSE; correlation with true spread **76.5% (EDGE) vs 66.9% (AR)**. Non-positive estimates: **5% for EDGE vs 20–30%** for alternatives with one year of data. EDGE remains unbiased under infrequent trading (<30 trades/day) where CS and AR converge slowly, and the paper explicitly demonstrates accurate **minute-level** estimation where competitors degrade ([paper PDF](https://acfr.aut.ac.nz/__data/assets/pdf_file/0016/570202/Efficient_Estimation_of_Bid_Ask_Spreads_from_OHLC_Prices-39.pdf); [R/Python `bidask`](https://github.com/eguidotti/bidask)). **[WS]**
→ *Decision:* EDGE is the default spread coordinate at all horizons. Keep CS and Roll only as legacy comparators in the ledger (they are cheap), flagged as deprecated.

**Hurst / long memory — compute it, but never on prices, and pin the protocol.**
Cont & Das show that even when spot volatility is a *Brownian* (H = 0.5) OU process, realized volatility estimated from discrete data yields **Ĥ ≈ 0.08–0.22**, and S&P 500 realized vol gives Ĥ ∈ [0.05, 0.25] — i.e., measured "roughness" is largely an artifact of estimating volatility, not a property of volatility ([arXiv 2203.13820](https://arxiv.org/pdf/2203.13820)). The 2026 cross-asset study (3,926 US equities, 34 CME futures roots, options on 44 underlyings, 2010–2025) argues roughness survives a mean-reversion-contamination correction and reports **livestock 0.05; rates/FX/ag/energy/metals 0.07–0.10; single stocks 0.13; equity indices 0.20 realized, 0.21–0.28 implied** ([arXiv 2608.16749](https://arxiv.org/abs/2608.16749)). **[PS — contested]**
→ *Decision:* Ĥ-on-realized-vol is a usable *asset-class-discriminating coordinate* (it separates livestock from equity indices by 4×), but it is **protocol-dependent**. Store `estimator_id`, sampling frequency, window, and pre-averaging choice with the value. Never compare Ĥ across different protocols. Hurst-on-price is dropped: variance-ratio statistics with explicit heteroskedasticity-robust standard errors carry the same information more interpretably.

**Realized skew/kurtosis — down-weight.** Sample third and fourth moments at minute frequency are dominated by a handful of jump minutes and are bounded by sample size (the classic Stata result: |skew| and kurtosis have hard algebraic bounds as functions of n). **[WS]** Keep *robust* substitutes as primary: quantile-based skewness (Bowley/Hinkley), Moors kurtosis, and the **jump fraction** from bipower variation, which is what realized skew is mostly proxying anyway.

**Jump detection — bipower / threshold-bipower, at a fixed sampling frequency.** BNS bipower variation and Corsi–Pirino–Renò threshold bipower are the standard. At minute bars microstructure noise is modest for liquid crypto/futures but not negligible; the practical fix is to compute the jump-robust estimators on **5-minute** subsampled returns and report the jump *ratio* (1 − BV/RV)⁺ rather than a jump count, which is threshold-sensitive. **[WS]**

**Kyle's λ and Amihud.** From minute bars: λ̂ from a regression of |Δp| (or signed Δp) on signed volume over rolling windows; Amihud as mean(|r| / dollar volume). Both are usable descriptors and both showed up as *stable* in the crypto minute-bar study above. **[PS]** Caveat: without true trade signs, use the taker-buy-volume field where the venue provides it (Binance does; Coinbase does via trade side), otherwise Lee–Ready on minute bars is noise.

**Order-flow imbalance.** Where available (crypto venues expose taker buy/sell base volume in the kline payload; Alpaca/equity minute bars do not), OFI is one of the two or three highest-signal microstructure features. Cont–Kukanov–Stoikov linear price-impact-in-OFI is the reference ([arXiv 1011.6402](https://arxiv.org/pdf/1011.6402)). **[WS for LOB data, PS for minute-aggregated]** Put it behind a *capability flag* per instrument — an embedding space whose dimensions silently become NaN for half the universe is a bug factory.

### 1.2 Session structure: crypto vs futures vs options underlyings

This is where the three asset families genuinely diverge and where naive code breaks.

**Crypto (24/7, no sessions).** There *is* strong periodicity, it is just not a session. Documented on BTC/ETH across Binance, Coinbase Pro, Uniswap V2 ([arXiv 2109.12142](https://arxiv.org/pdf/2109.12142)) **[WS]**:
- Day-of-week: **Saturday volatility 30–50% below weekly average**; Thursday peaks.
- Hour-of-day: vol peaks ~**16:00 UTC**, troughs ~**05:00 UTC**; Binance illiquidity ~**30% higher** in East-Asian night hours; Coinbase illiquidity peaks 10:00 UTC ranging 80–150% of average.
- **Minute-of-hour**: largest vol spike at minute :00, secondary at :15/:30/:45, smaller every 5 minutes, intensifying every 4 hours — driven by algo schedules and **perp funding times** (Binance 00/08/16 UTC).
- Adding day-of-week periodicity to EGARCH improved out-of-sample log-likelihood by **34 units (BTC/USD)**.
→ *Decision:* the crypto "seasonality profile" coordinate is a **(168-hour × venue) tensor plus a minute-of-hour vector**, not a U-shape. Always compute realized-vol features on *seasonality-deflated* returns (divide by the estimated periodic factor) or you will cluster assets by their venue's funding schedule. Store the deflator as a versioned artifact.

**Futures (continuous contracts, rolls).** Two distinct contaminations:
- *Roll method* changes the return series. Back-adjustment (difference or ratio/"Panama") alters volatility, skew, and every autocorrelation statistic. **[WS]** Ratio-adjustment keeps returns well-defined but makes distant historical prices meaningless; difference-adjustment can go negative.
- *Roll windows* inject spurious jumps and volume spikes.
→ *Decision:* store the fingerprint keyed by `(root, roll_rule_id)` and always compute features on **ratio-back-adjusted returns with roll-day minutes masked** (an explicit `is_roll_window` mask column). Additionally carry **term-structure coordinates** (front/second log price ratio, roll yield, days-to-expiry) — these are the features that actually distinguish CL from ES, far more than any catch22 statistic.

**Options.** Do not fingerprint individual contracts. Cardinality is the enemy: a single liquid underlying generates 10³–10⁴ live contracts × 390 minutes. The tractable representation is a two-level one **[WS as standard practice; PS as a ranking-useful representation]**:
- *Level 1 — underlying*: full minute-bar fingerprint of the underlying (spot/futures), as above.
- *Level 2 — surface*: fit a parsimonious arbitrage-free parameterization per timestamp (SVI/SSVI per slice, or a low-dimensional VAE/normalizing-flow latent) and use the **parameter vector plus its time-series statistics** as the option-complex fingerprint. SVI is 5 params/slice; SSVI is 3 global + a θ(t) curve. Generative latent-space approaches (VAE, VolGAN, flow matching) compress surfaces to **~4–8 latent dims** with arbitrage penalties ([VAE arb-free surfaces, SIAM J. Fin. Math](https://epubs.siam.org/doi/10.1137/21M1443546); [VolGAN, Applied Math. Finance 2025](https://www.tandfonline.com/doi/full/10.1080/1350486X.2025.2471317)).
- Derived scalars that earn their place: 30d ATM IV, 25Δ risk reversal, 25Δ butterfly, IV term-structure slope, IV–RV spread (variance risk premium), open-interest concentration by moneyness bucket.
→ Individual contracts get *coordinates*, not embeddings: `(underlying_id, log-moneyness bucket, tenor bucket, type)`. That is a 4-tuple, and it is sufficient for routing.

**ETFs.** Mostly behave like equities but require an explicit **overnight/intraday decomposition**, because that is where their return actually lives. The overnight-vs-intraday split is a real, persistent cross-sectional feature (see [JFE 2021 cross-section of intraday and overnight returns](https://www.sciencedirect.com/science/article/abs/pii/S0304405X21000854); [Finance Research Letters 2025, 11.6M observations](https://www.sciencedirect.com/science/article/abs/pii/S1544612325018926)). **[WS]** Store `var_overnight / var_total`, `mean_overnight_ret`, `mean_intraday_ret` separately. For crypto this ratio is *undefined* — encode it as NULL with a `session_model` discriminator, do not impute zero.

**DeFi pools.** Different object entirely. Price on an AMM is a function of reserves, and Uniswap V2 correlated only **0.328** with CEX ETH prices while CEX–CEX pairs correlated **0.968**, with Uniswap lagging **3–12 minutes** ([arXiv 2109.12142](https://arxiv.org/pdf/2109.12142)). **[WS]**
→ *Decision:* DeFi pools get a **separate feature family**: TVL level and volatility, reserve imbalance, fee tier, realized fee APR, volume/TVL turnover, LP concentration (for v3/v4 tick ranges), and **LVR** (loss-versus-rebalancing), which has largely superseded "impermanent loss" as the correct adverse-selection metric for LPs ([LVR bounding, arXiv 2605.19267](https://arxiv.org/html/2605.19267); [SoK on IL, IACR 2026/1073](https://eprint.iacr.org/2026/1073.pdf)). Pools must carry a `venue_lag_minutes` coordinate; never put a pool and a CEX pair in the same kNN space without it.

### 1.3 Canonical feature sets: catch22 / tsfresh / TSFEL — what the evidence actually says

- **Cost** (per 1,000-sample series): catch22 **<10 ms**, TSFEL **0.03 s**, Kats 0.06 s, feasts 0.47 s, tsfresh **2.53 s**, tsfeatures **6.18 s**, hctsa **16.5 s** ([Henderson & Fulcher, arXiv 2110.10914](https://arxiv.org/pdf/2110.10914)). **[WS]** At our scale (10⁴ instruments × ~10 windows × daily recompute) tsfresh's default set is a 3-order-of-magnitude cost multiplier for marginal gain.
- **Redundancy**: TSFEL captures 90% of its variance in **4 of 390** components; tsfresh needs 55 of 779; catch22 needs **11 of 22** — i.e., catch22 is by far the most information-dense per feature. **[WS]**
- **Accuracy**: the 2026 statistical comparison over **124 UCR problems × 30 resamples** found **85.3% of pairwise feature-set comparisons were ties**; win rates tsfresh 29.0%, tsfeatures 18.3%, TSFEL 18.2%, with catch22/Kats/feasts lower; within-problem variance 0.003 vs between-problem 0.03 ([arXiv 2608.01586](https://arxiv.org/html/2608.01586)). **[WS]** Translation: *which* canonical set you pick is nearly irrelevant compared to *which problem* you are solving.
- **For algorithm selection specifically** (our actual use case), a 2025 benchmark found TSFRESH/TSFEATURES/TSFEL comparable and **catch22 notably weaker**, with TSFEL the best dimensionality/performance trade-off ([Springer LNCS](https://link.springer.com/chapter/10.1007/978-3-032-05176-9_21)). **[PS]** This matters: catch22 was *selected* for classification accuracy on UCR, and that selection does not transfer to meta-learning.
- **Wrapping matters more than the set**: in the 112-dataset Bake Off Redux, FreshPRINCE (tsfresh + rotation forest) scores **0.855** vs raw TSFresh pipeline **0.799** — a 5.6-point swing from the classifier, larger than any feature-set difference. Best overall were MultiROCKET-Hydra **0.884** and HIVE-COTEv2 ([arXiv 2304.13029](https://arxiv.org/html/2304.13029)). **[WS]**

→ **Decision:** ship **catch22 + TSFEL-subset (~40 hand-picked, non-FFT-redundant) + the finance-specific block** (§1.1). Do not ship tsfresh's 783-feature default. Add **MiniROCKET/MultiROCKET features (fixed random seed, frozen kernels) as an optional cheap 1k-dim channel** — it is the strongest cheap generic representation in the literature and it is deterministic once seeded.

### 1.4 Stability: which features survive

Direct evidence is thinner than one would like. What exists:
- In the crypto minute-bar study, **all 12 features passed a 0.5 randomized-Lasso stability-selection threshold**, with realized volatility and the CS spread ranking highest. **[PS]** So *feature stability* is easy at minute frequency; *economic content* is what is missing.
- Realized volatility, spread proxies, volume/turnover, and the seasonality profile are stable across months in essentially every study that measures it. **[WS]**
- Autocorrelation/variance-ratio statistics are the classic *unstable* ones — the adaptive-markets literature documents time-varying and sign-flipping efficiency measures ([Economics Letters 2009](https://www.sciencedirect.com/science/article/abs/pii/S0165176509000408); [IRFA 2013](https://www.sciencedirect.com/science/article/abs/pii/S1057521913000379)). **[WS]**
- Higher moments and Hurst are protocol-dependent and noisy (§1.1). **[WS]**

→ **Decision:** don't guess — *measure stability as a first-class product metric*. Ship a nightly job computing, per feature j: (a) rank-autocorrelation of the cross-sectional ranking at lags 1d/1w/1m; (b) split-half reliability within a window; (c) fraction of instruments with non-degenerate values. Features below a rank-autocorr threshold get demoted from the retrieval vector into an "observational" block that is logged but not used as a coordinate. This is cheap and turns a literature gap into an internal dataset.

---

## 2. Time-series foundation models 2025–2026: do they give usable *embeddings*?

### 2.1 State of the field (Sept 2026)

| Model | Type | Embeddings exposed? | Notes |
|---|---|---|---|
| **Chronos-2** (Amazon, Oct 2025) | encoder-only, group attention, in-context covariates | Encoder hidden states accessible; **not a designed product** | fev-bench win 90.7%/skill 47.3%; GIFT-Eval win 81.9%, WQL skill 51.4% ([arXiv 2510.15821](https://arxiv.org/pdf/2510.15821)) |
| **TimesFM-2.5** (Google) | decoder-only patched | patch embeddings accessible | fev-bench 79.6%/42.2% |
| **TiRex** (NX-AI) | xLSTM | hidden state accessible | fev-bench 83.3%/42.6% ([arXiv 2505.23719](https://arxiv.org/abs/2505.23719)) |
| **Moirai / Moirai-2** (Salesforce) | masked encoder | yes | **85–96% benchmark contamination** per TSFMAudit |
| **MOMENT** (CMU) | masked-reconstruction encoder | **yes, explicitly designed for embeddings** | weakest of the frozen-embedding results in the 2026 e-nose study |
| **Toto-2** (Datadog, 2026) | decoder, 4m–2.5B, Apache-2.0 | not documented | trained on observability + synthetic, *excluded public forecasting data*; leads BOOM ([Datadog](https://www.datadoghq.com/blog/ai/toto-2/)) |
| **Sundial**, **FlowState**, **Kairos**, **VisionTS++** | various | varies | Kairos heavily contaminated |
| **TabPFN-TS** | tabular FM + calendar features | n/a | beats specialist TS models using *simple features* ([arXiv 2501.02945](https://arxiv.org/html/2501.02945v2)) — a strong argument that the "foundation" part is doing less than claimed |
| **Lag-Llama** | decoder | yes | superseded; no longer competitive |
| **FinCast** (Aug 2025) | 1B decoder + sparse MoE, 20B+ points, minute→weekly, frequency embeddings | plausible | claims 20%/23% error reduction vs TSFM/statistical ([arXiv 2508.19609](https://arxiv.org/html/2508.19609v1)) — **unverified externally** |
| **Kronos** (2025/26) | BSQ tokenizer + decoder over OHLCVA K-lines, 12B records, 45 exchanges, 7 granularities | discrete token embeddings | claims **+93% RankIC** over leading TSFM ([arXiv 2508.02739](https://arxiv.org/html/2508.02739v1)) — **self-reported splits** |

### 2.2 Benchmark reality

- **fev-bench** (Sept 2025) is the better-designed benchmark: 100 tasks, 7 domains, **46 tasks with covariates** (GIFT-Eval has 0), win-rate + skill-score with confidence intervals. Chronos-2 91.4% win / 47.3% skill; Seasonal-Naive 19.6% / 0.0%. **It contains no dedicated financial datasets** — its highest-frequency data is cloud observability (5-min), energy prices (hourly), taxi demand (15-min) ([arXiv 2509.26468](https://arxiv.org/pdf/2509.26468)). **[WS]**
- **GIFT-Eval**: 23 datasets, heavily weighted to low-frequency economics/finance (99,974 series) and daily (38,625); only **22 secondly series**. The authors themselves acknowledge partial pretraining leakage for TimesFM, Chronos and Moirai ([arXiv 2410.10393](https://arxiv.org/html/2410.10393v2)). The public leaderboard as of **12 Sep 2026** is topped by agentic/proprietary submissions (STRIDE w/ Synapse avg-rank 14.62, EXAONE-Forecast-Agent 18.87, LS-MoE 19.31), with Seasonal-Naive at rank 124/130 ([TSFM.ai mirror](https://tsfm.ai/benchmarks/gift-eval)). **[Treat as WS for "TSFMs beat Seasonal-Naive on GIFT-Eval's mix", SPEC for anything else.]**
- **"It's TIME" (2026)** rebuilds a benchmark on fresh data specifically to dodge contamination; includes three finance datasets (crypto, US term structure, oil) but reports **no finance-specific findings**. It confirms Chronos-2 / TimesFM-2.5 / TiRex as the leaders and notes rankings *shift with stationarity* ([arXiv 2602.12147](https://arxiv.org/html/2602.12147v3)). **[PS]**

### 2.3 Do they beat simple baselines on *financial minute bars*?

**There is no published benchmark that answers this cleanly.** That absence is itself the finding. What we have:

- **Realized volatility (not returns)**: TimesFM zero-shot is "reasonable" but **incremental fine-tuning is essential** to beat HAR/GARCH on Diebold–Mariano / Giacomini–White tests ([arXiv 2505.11163](https://ideas.repec.org/p/arx/papers/2505.11163.html)). **[PS]** This matches the general pattern: TSFMs are competitive on *volatility* (a persistent, forecastable quantity) and unproven on *returns*.
- **Returns**: the only honest minute-bar result in hand is the crypto microstructure study, and its verdict is that *nothing* — OLS, LightGBM, MAML — survives fees. **[WS for that universe/period]**
- Retrieval-augmented forecasting (RAFT) wins 86% across 10 benchmarks but is explicitly **weaker on the Exchange-Rate dataset** and was not validated on noisy financial series ([arXiv 2505.04163](https://arxiv.org/pdf/2505.04163)). **[PS]** Same pattern appears in SimTS: seasonal-trend disentanglement helps stationary data and *hurts* Exchange and ETTm2.

→ **Decision.** Use TSFMs for three things and nothing else, each behind a measured gate:
1. **Volatility/liquidity forecasting** as a landmarker (§4) — most likely to pay.
2. **Embeddings as an *additional* channel** concatenated to hand-crafted features, admitted only if it clears a retrieval-lift gate (§8.4).
3. **Synthetic-path generation** for null-model falsification (§8.3) — Kronos's generative fidelity claim, if it holds, is more useful to us as a *null generator* than as a forecaster.
Do **not** put a TSFM in the live return-prediction path. Prefer **Toto-2 (Apache-2.0, trained without public forecasting data → least contaminated relative to our eval)** and **Chronos-2** as the two encoders to trial; both are self-hostable, which matters for multi-tenant data isolation.

---

## 3. Self-supervised representation learning for time series

### 3.1 The methods and their honest status

- **TS2Vec** (AAAI'22): hierarchical contrasting over augmented contexts, with instance-wise and temporal contrastive losses at multiple scales. Still the default baseline. **[WS as a baseline, PS as a winner]**
- **TF-C** (time–frequency consistency), **TS-TCC** (temporal + contextual contrasting), **CoST** (disentangled seasonal/trend), **SimMTM** (masked modeling with multi-view neighborhood reconstruction).
- **The load-bearing critique** comes from SimTS ([arXiv 2303.18205](https://arxiv.org/html/2303.18205v2)) **[PS, but with ablations]**:
  - *Negative pairs actively hurt.* Adding InfoNCE with negatives degraded results on most datasets ("false repulsion" — time series that genuinely resemble each other get pushed apart).
  - *Augmentations are counterproductive.* Removing CoST's augmentations sometimes **improved** results; adding random masks typically worsened them.
  - *Disentanglement assumptions fail off-stationary.* Season–trend decomposition helped only stationary datasets and **impaired forecasting on Exchange and ETTm2** — the two most finance-like series in the standard suite.

### 3.2 Why standard augmentations break financial semantics

This is not a vague worry; each standard augmentation violates a specific financial invariance **[WS by construction; PS empirically]**:

| Augmentation | What it destroys |
|---|---|
| **Jitter / Gaussian noise** | Changes realized variance, the single most informative property of the window. Adding σ_noise to minute returns directly biases RV upward and destroys the vol-of-vol coordinate. |
| **Scaling (amplitude)** | Makes two windows with different volatility "the same sample" — but volatility level *is* the label for most downstream routing decisions. |
| **Time warping** | Breaks the √t scaling of diffusion, destroys autocorrelation and variance-ratio structure, and makes intraday seasonality incoherent. |
| **Permutation / shuffling segments** | Destroys volatility clustering (the one robust stylized fact) and any jump/lead-lag structure. |
| **Cropping / random masking** | Least harmful *if* crops respect session and roll boundaries; harmful if a crop straddles a futures roll or a funding minute. |
| **Window slicing with overlap** | Creates near-duplicate positives that leak across train/test splits — a direct route to the "spurious predictability" failure of §8.3. |

**Financially valid augmentations** (invariances that actually hold): (i) **venue swap** for the same underlying — supported by the observed spot↔futures transfer; (ii) **numéraire/quote-currency change** (BTC/USDT ↔ BTC/USD); (iii) **block bootstrap of standardized residuals** after devolatizing (preserves the vol process, resamples the shocks); (iv) **subsampling to a coarser bar grid with a phase offset** (1-min → 5-min at offsets 0..4); (v) **contract roll-rule perturbation** for futures. **[SPEC — no paper validates this list for finance; it is derived from the invariances, and should be A/B'd.]**

### 3.3 When do learned embeddings beat hand-crafted vectors?

Evidence in both directions, and the split is legible:

**Learned wins when the supervision signal is structural, not predictive.**
- Contrastive asset embeddings trained on *co-occurrence in top-k correlation neighborhoods*, with a hypothesis test separating real from noise co-occurrence, produce 16-d embeddings that beat correlation baselines: industry-classification F1 **0.69 vs 0.66** prior embedding work; hedged-portfolio out-of-sample vol **19.1% vs 23.8% (Pearson)** ([arXiv 2407.18645](https://arxiv.org/html/2407.18645v1)). **[PS]** Note the sharp task-dependence: the loss that wins classification (sigmoid-softmax) *loses* hedging (26.1% vol).
- **Asset Embeddings** from institutional holdings (Gabaix, Koijen, Richmond, Yogo, NBER w33651): **4 dimensions explain 50%** of cross-sectional valuation variation vs **15% for four standard firm characteristics**; 128 dims reach **70%**. Text embeddings from OpenAI/Cohere perform poorly — the holdings data is what carries the signal ([NBER PDF](https://www.nber.org/system/files/working_papers/w33651/w33651.pdf)). **[WS within equities]** *But:* no holdings data exists for crypto, futures or DeFi pools. The lesson transfers even if the method does not: **relational/behavioral data beats statistical summaries of the price path**. Our analogue is the **trial ledger itself** (§7) and co-holding/co-trading across tenants (with privacy care).

**Hand-crafted wins when signal-to-noise is low.** The Korean investor-flow study (2.79M obs, 2,439 stocks, 2020–24): a theory-grounded linear normalization (net buy / market cap, i.e., Kyle-style impact scaling) gave **Sharpe 1.30 / +272.6% cumulative**; ICA gave **Sharpe 0.07**; a raw LSTM had a **47.5% hit rate** (worse than chance) and collapsed to the unconditional mean. Gap: **18.6× on Sharpe**. SNR was ~0.8% ([arXiv 2601.07131](https://arxiv.org/html/2601.07131v1)). **[PS — single market, but the mechanism is general]**

→ **Decision.** Do **not** train an SSL encoder on returns to predict returns. Train one (if at all) on a **structural objective**: predict co-membership in correlation neighborhoods / co-regime / venue-pair identity, with only the financially valid augmentations, and evaluate it *only* on retrieval and transfer-lift probes (§8.4). Budget it as a Phase-3 item behind a hard gate.

---

## 4. Performance-based landmarkers as task representation

### 4.1 The evidence

This is the best-supported single idea in the whole brief.

- **Auto-sklearn 2.0** removed meta-features entirely. Their five stated reasons — meta-feature computation cost, its own time/memory management burden, meta-features that don't handle categorical/missing data, no guidance on which meta-features work, and no mechanism to prevent duplicate pipelines — map 1:1 onto problems we would hit. They replaced them with a **greedy submodular-selected static portfolio of 32 pipelines** with the standard (1 − 1/e) approximation guarantee. They note they had *already* dropped landmarking meta-features earlier because of runtime ([JMLR 23](https://ml.informatik.uni-freiburg.de/wp-content/uploads/papers/21-ARXIV-ASKL2.pdf)). **[WS]**
- **TabRepo**: 1,310 configs × 200 datasets × 3 seeds × 8-fold bagging = 786,000 stored prediction sets. Greedy portfolio selection (each step picks the config minimizing average error *in combination with those already chosen* — i.e., complementarity, not individual quality). Results at 4h budget: Portfolio-ensemble **0.365 normalized error** vs AutoGluon 0.389 vs Auto-sklearn-2 0.455. **3 configs** beat everything but AutoGluon; **15 configs** beat AutoGluon; performance saturates around **150 offline configs or datasets**; optimal ensemble size 15 ([arXiv 2311.02971](https://arxiv.org/html/2311.02971v3)). **[WS]**
- **Counter-evidence for pure landmarkers:** AutoForecast (ACM TKDD 2025) *does* use landmarkers — among 800+ meta-features across simple/statistical/information-theoretic/spectral/landmarker categories, where landmarkers contribute both model structure and **output performance scores**. It achieves Hit@10 **79.2% better than Global Best**, **423.2% better than ISAC**, and a **42× median speedup** over brute force, at 1.7 s inference ([Purdue DCSL PDF](https://engineering.purdue.edu/dcsl/wp-content/uploads/2025/02/AutoForecast_ACM_TKDD.pdf)). **[PS]** So landmarkers *can* work — but note the baseline: "Global Best" (always pick the globally best model) is a weak baseline, and a *portfolio* baseline was not run.
- **Landmarker theory**: the classic line is Pfahringer/Bensusan/Giraud-Carrier landmarking and *relative* landmarkers (rank-transform the landmarker scores, which removes task-level difficulty and keeps only the *relative ordering*, which is what actually transfers). Sample-based **active testing** (Leite/Brazdil/Vanschoren) is the cost-aware version: iteratively run the next most-informative cheap evaluation.

### 4.2 Design for our platform

**How many?** Evidence says **8–16 is the right order of magnitude** for the landmarker vector, and **~32 configs** for a warm-start portfolio. TabRepo's saturation at 15 ensemble members and Auto-sklearn's 32-pipeline portfolio bracket it. **[WS by analogy]**

**Which ones?** Not hand-picked. Run greedy submodular selection *over your own trial ledger*:
> Given the ledger matrix `R[task, config] = metric`, greedily choose configs c₁..c_k maximizing Σ_tasks max(R[t, c₁..c_k]) (coverage) — equivalently minimizing average regret. Re-run monthly. This is exactly TabRepo's `zeroshot_portfolio` procedure and it is ~50 lines.

**Concretely, the seed landmarker portfolio** (before the ledger exists) — chosen for complementarity across the hypothesis space, not for expected profitability:
1. Buy-and-hold (the null that most crypto strategies fail against).
2. Time-series momentum, fast (e.g., 60-min lookback) — trend at high frequency.
3. Time-series momentum, slow (e.g., 1-day/5-day) — trend at low frequency.
4. Mean reversion on z-scored minute returns (Ornstein–Uhlenbeck-ish).
5. Cross-sectional momentum within the instrument's cluster.
6. Volatility-targeted carry (funding rate for perps, roll yield for futures, dividend/borrow for equities).
7. Breakout / range-expansion (Donchian).
8. Opening-range / session-boundary effect — NULL for crypto, which is itself informative.
9. HAR-RV volatility forecast skill (R² OOS) — a *forecastability* landmarker rather than a P&L one.
10. Simple OFI/imbalance signal (where available).
11. Liquidity-provision proxy (fade the move, pay the spread) — measures whether the asset rewards or punishes liquidity supply.
12. A GBDT on the §1 fingerprint with fixed hyperparameters and a fixed 5-minute forward label — the "learned baseline" landmarker.

**What to record per landmarker** — and this is the crucial bit — is not just the Sharpe. Record a **vector of diagnostics**: OOS Sharpe, deflated Sharpe, hit rate, turnover, gross-minus-net gap, max drawdown, t-stat, R²_OOS, and **decay profile across horizons**. That turns each landmarker into ~8 coordinates and gives you a 100-dim landmark fingerprint from 12 cheap runs. **[SPEC but low-risk]**

**Keeping them cheap at minute-bar scale.** The whole point collapses if landmarkers cost as much as real experiments.
- Compute on a **fixed evaluation slice**: last N days (e.g., 90), a fixed set of 3 disjoint windows, vectorized, no parameter search.
- **Vectorize across the universe**: all 12 landmarkers are matrix operations over an (instruments × minutes) panel. On 10⁴ instruments × 90 days × 1,440 min ≈ 1.3×10⁹ cells this is a few minutes of NumPy/Polars per nightly run, not a training job.
- **Cache by content hash** of `(instrument, window, bar_version, landmarker_version)`.
- **Rank-transform** within each evaluation batch → relative landmarkers, which removes the market-wide regime level and keeps the cross-sectional signal. This is the documented improvement over raw landmarkers.

→ **Decision:** landmarkers are **Phase 1**, before any learned embedding. They are cheap, interpretable, directly comparable to the outcome tensor's row space, and the meta-learning literature's strongest result.

---

## 5. Clustering and correlation structure

### 5.1 What the 2024–2026 evidence says about the popular methods

- **HRP/HERC do not reliably beat simple risk-based portfolios out of sample.** A 2026 study over Brazilian and US equities, 2011–2025, 756-day rolling estimation, monthly rebalance, comparing HRP/HCAA/HERC/CHRP against MV/RP/MD/MDE/IV/EW concluded: *"There is no evidence that hierarchical risk clustering techniques outperform established risk-based approaches in the datasets analysed, even when alternative covariance matrix estimators are used."* Minimum-variance had the lowest realized vol in both markets; hierarchical methods had **higher turnover and transaction costs** ([Empirical Economics 2026](https://link.springer.com/article/10.1007/s00181-026-02900-x)). **[WS]**
  → Use hierarchical clustering **as a taxonomy for the embedding space and for the outcome tensor's row grouping**, which is what we actually need it for, and *not* as an allocation method.
- **Covariance cleaning does help, but the winner is market-dependent.** A 2026 six-market study (DJ30, NIFTY50, FTSE100 in n>p; S&P500, Russell 1000, TOPIX 1500 in p>n) over 5 mean and 11 covariance shrinkage estimators found GMV + **Ledoit–Wolf two-parameter shrinkage** best for most investor profiles, that **8 of 11 covariance estimators are simply inapplicable when p>n**, and that *"the choice of shrinkage estimators is more dependent on underlying market data"* — no universal winner. Sample mean + sample covariance is uniformly bad ([arXiv 2601.20643](https://arxiv.org/html/2601.20643v1)). **[WS]**
- **RMT denoising (Marchenko–Pastur eigenvalue clipping) + detoning** remains the standard preprocessing for correlation-distance clustering: clip eigenvalues below λ₊ = σ²(1+√(p/n))², rescale to preserve trace, then optionally remove the top ("market") eigenvector before clustering so you cluster on *residual* co-movement rather than beta. **[WS as method; PS on magnitude of benefit]** Detoning matters enormously in crypto, where the first eigenvalue routinely explains 60–80% of variance and every clustering without detoning returns "one big cluster."

### 5.2 Distance and linkage

- Correlation distance `d = √(2(1−ρ))` (a proper metric, Mantegna) on **denoised, detoned** correlations, with Ward or average linkage.
- Compute ρ on **overlapping-window-free, seasonality-deflated, synchronized** returns. Synchronization is a real problem: crypto trades 24/7, US futures have a daily halt, equities have 6.5h sessions, DeFi pools update per block. The clean solution is a **common minute grid with explicit NaN masking and pairwise-complete correlation**, then a **positive-semidefinite repair** (nearest correlation matrix, Higham) — pairwise-complete correlation matrices are routinely non-PSD.
- For asynchronous/illiquid pairs use **Hayashi–Yoshida** rather than forcing a grid. **[WS]**

### 5.3 Graph/GNN embeddings

A 2024 ACM Computing Surveys systematic review covers GNN methods for stock forecasting ([10.1145/3696411](https://dl.acm.org/doi/10.1145/3696411)). The honest read: the sub-literature is large, results are usually reported on small self-chosen universes with no deflated-Sharpe correction, and reproductions are rare. **[SPEC for our purposes.]**
→ *Decision:* use **graph structure** (correlation MST, cluster membership, shared underlying, shared venue, shared collateral) as **side information in the tensor factorization** (§7), where it is cheap and testable, rather than training a GNN. Node2vec/DeepWalk on the MST is a 20-line, fully deterministic fallback that gives you a graph-aware coordinate block.

### 5.4 Asset-class-specific clustering problems

- **Options:** do **not** cluster contracts. Cluster *underlyings* using surface-derived coordinates (ATM IV level/slope/curvature term structure, skew, VRP). Contracts get deterministic bucket coordinates `(underlying_cluster, moneyness_bucket, tenor_bucket, cp_flag)`. The cardinality argument is decisive: 44 underlyings × 10³ live contracts is 4×10⁴ objects that carry ~5 degrees of freedom each.
- **Futures:** cluster on the **term-structure shape** (PCA of the log-price curve across maturities: level/slope/curvature) plus the §1 fingerprint. Two contracts on the same root in different roll regimes are *not* the same asset.
- **DeFi pools:** cluster on **liquidity/TVL dynamics and fee-tier economics**, not price. Pools sharing a token but on different chains/fee tiers have very different LVR and turnover profiles. Given the 0.328 CEX–DEX correlation, a DEX pool is a *different instrument* from the CEX pair of the same token.
- **Crypto:** detone aggressively; otherwise everything clusters with BTC.

### 5.5 Cluster stability — make it a metric, not a vibe

- **Bootstrap/jackknife ARI** per cluster (Hennig's cluster-wise stability via bootstrap Jaccard is the canonical method: resample, re-cluster, track each cluster's maximum Jaccard against the original; mean Jaccard < 0.6 = dissolved, > 0.85 = stable) ([Hennig, CSDA 2007](https://www.homepages.ucl.ac.uk/~ucakche/papers/clusta.pdf)). **[WS]**
- **Temporal ARI**: ARI between the partition at t and at t−Δ for Δ ∈ {1w, 1m, 3m}. Publish this as a time series. A cluster taxonomy whose 1-month ARI is 0.3 is not a taxonomy, it is noise, and the outcome tensor built on it will be unlearnable.
- **Never use raw Rand index** — ARI is chance-corrected and the correction is large at the cluster counts we'll use ([scikit-learn demonstration](https://scikit-learn.org/stable/auto_examples/cluster/plot_adjusted_for_chance_measures.html)).
- Fix **k** by stability, not by silhouette: choose the k that maximizes bootstrap Jaccard, which is the standard robust criterion.

---

## 6. Regime representation

### 6.1 The operational definition problem

Practitioners define regimes narratively; that is useless for a tensor index. Three defensible operational definitions, in increasing order of rigor:

**(a) Distributional (best-supported).** A regime is a **probability measure** on returns. Horvath, Issa & Muguruza formalize this: partition log-returns into overlapping windows (length h₁, stride h₂), map each to its empirical measure, and cluster in 𝓛_p(ℝ) under the **p-Wasserstein distance** with Wasserstein barycenters as centroids (WK-means). Validation uses **MMD** self-similarity within clusters vs distinctness between, plus Davies–Bouldin/Dunn/Silhouette. On Merton jump-diffusion synthetic data with known switches: **WK-means 91.28% ± 4.08% accuracy vs moment-based k-means 66.64% ± 3.42% vs HMM 75.05%**. On SPY hourly 2005–2020 with k=2 it recovered 2008, 2020, the 2010–11 European debt crisis and the 2015–16 China crash that moment-k-means missed ([arXiv 2110.11848](https://arxiv.org/html/2110.11848v1)). **[WS on synthetic; PS on real]**

**(b) Latent-state (HMM/HSMM).** Standard, but see the lookahead trap below. HSMMs add explicit duration distributions, which matters because geometric dwell times from an HMM are a poor fit to market regimes; a 2024 comparative study of HMM vs HSMM for regime-based asset allocation is the reference point ([SSRN 4796238](https://papers.ssrn.com/sol3/Delivery.cfm/SSRN_ID4796238_code2469387.pdf?abstractid=4796238&mirid=1)). **[PS]**

**(c) Change-point (BOCPD).** Adams–MacKay online changepoint detection gives a run-length posterior, which is genuinely causal and gives a calibrated "time since regime start" coordinate. Modern variants add autoregressive dynamics with time-varying parameters ([arXiv 2407.16376](https://arxiv.org/abs/2407.16376)). **[PS]**

### 6.2 The single most important operational rule

**Use filtered (online) state probabilities only. Never smoothed, never Viterbi.**

A controlled "lookahead ladder" on a 2-state Gaussian HMM: the same strategy scores **Sharpe 0.78** using filtered probabilities, **0.77** with full-sample *parameter* estimation (i.e., parameter lookahead is nearly harmless), and **1.74** with smoothed probabilities — a **2.2× inflation** purely from non-causal state inference. The author's summary: *"The danger is in the probabilities, not the parameters… many published HMM backtests quote smoothed or Viterbi states"* ([repo + writeup](https://github.com/dmitridefreitas-dev/regime-detection)). **[PS — single careful experiment, but the mechanism is airtight and the magnitude is consistent with the general leakage literature.]**

→ *Enforce in code:* the regime table stores `p_filtered` only; a `p_smoothed` column, if it exists at all, must be physically separated into a research-only schema that the backtester cannot join to.

### 6.3 Cross-asset regimes with different clocks

Regimes are **global** (a risk-off day is risk-off in Tokyo, Chicago and on Binance) but the *observation grid* differs. Two-layer design **[SPEC but structurally forced]**:
- **Global regime**: fitted on a small set of always-on, high-quality reference series on a **common UTC clock** — BTC/USDT perp, ES continuous, a liquid FX pair, a volatility index proxy, a rates proxy. Where a market is closed, carry the last observation forward *and set a staleness mask*; do not interpolate.
- **Local regime**: per-instrument, on that instrument's native clock, capturing idiosyncratic states (illiquid vs liquid, pre/post-listing-event, high/low funding).
- The regime coordinate in the outcome tensor is the **pair** `(global_regime_id, local_regime_id)`. This keeps the tensor's regime axis small (say 4 × 3 = 12) while remaining honest about clock mismatch.

### 6.4 Does regime-conditioning actually pay?

The strongest recent affirmative: Mulliner, Harvey, Xia & Fang (2025) identify regimes from seven economic state variables and build a strategy long factor exposures in *similar* historical regimes and short *dissimilar* ones, over six equity factors, 1985–2024. Reported alpha is **≈3 standard errors from zero**, positive in **80% of years**, with average outperformance **+13.3%** vs average underperformance **−5.1%**, and positive skew ([Alpha Architect summary](https://alphaarchitect.com/regime-detection/)). **[PS]** The authors themselves flag real-time identification as "one of the biggest challenges in finance," sensitivity to the choice of state variables, and a single period/factor set.

→ **Decision:** regimes are worth a tensor axis, but with a small cardinality (≤ 6 global states), filtered-only inference, and an explicit **regime-stability metric** (expected dwell time, ARI of the regime labeling under bootstrap) published alongside. Treat "regime-conditional Sharpe" results as hypotheses requiring the §8.3 falsification protocol, not as findings.

---

## 7. The Outcome Tensor and matrix-completion framing

### 7.1 The object

```
T[asset_cluster, regime, strategy_family, config_bucket, metric] → value
```
with observation indicator `D` and logged propensity `π`. This is the core asset of the platform: it is what turns thousands of agent-run experiments into a prior over what to try next.

Practical shaping decisions:
- **Metric axis should not be a tensor mode**; different metrics have incomparable scales and different noise models. Fit **one tensor per metric** (or a multi-output factorization sharing factors across metrics — a "coupled" factorization). Primary metric: deflated Sharpe or an IR-like statistic; secondary: turnover, max DD, capacity.
- **Config is continuous**, not categorical. Either bucket it (quantile bins per hyperparameter) or — better — keep `config` as a *content vector* and use the factorization with side information (below), which sidesteps cold-start entirely.
- Store **raw trials** immutably; the tensor is a **materialized view** with a `knowledge_time`.

### 7.2 Collaborative filtering with side information

The right frame is **cold-start-dominated CF**: nearly every (asset, config) cell is unobserved, and new assets and new configs arrive constantly. Standard approaches:
- **Inductive/hybrid MF**: factorize `R ≈ (X_a W_a)(X_c W_c)ᵀ + U Vᵀ`, where `X_a` is the asset fingerprint + landmarker vector and `X_c` is the config content vector. The content term handles cold start; the free-embedding term captures residual idiosyncrasy. This is "collective matrix factorization" / inductive matrix completion.
- **Non-negative tensor factorization (CP/Tucker)** when you want parts-based, interpretable factors — useful for "which strategy families does this cluster like" reporting. CP with `R` components and non-negativity is stable and cheap at our sizes.
- **The 2025 SOTA for our exact problem** is generalized tensor completion in the exponential family with CP low-rank structure ([arXiv 2509.06225](https://arxiv.org/pdf/2509.06225)) — see below, because it also solves the MNAR problem.

### 7.3 Missing-not-at-random — the hard part, and the part that will actually bite

Your agents choose what to run. That is a **logging policy**. Cells are missing precisely because the agent predicted they'd be bad, or because a run was killed. Ignoring this produces a tensor that says "everything we tried worked," which is exactly the failure mode.

**The IPS line.** Schnabel, Swaminathan & Joachims frame recommendation as treatment assignment: reweight each observed loss by `1/P(observed)`. Propensity-weighted MF simply divides each term of the MF objective by `P̂(u,i)`. Empirically, IPS/SNIPS gave "orders-of-magnitude more accurate" performance estimates than naive evaluation; MF-IPS reached **MAE 0.810 on Yahoo vs 1.154 naive** and beat Bayesian imputation at p<0.001. Propensities can be estimated by naive Bayes (needs a small MCAR sample) or logistic regression on observable features (no MCAR sample needed, well-calibrated marginals). Their Lemma 5.1 gives the bias under misspecified propensities: `Σ loss·(1 − P/P̂)/(U·I)`, revealing a bias–variance trade-off where *over*estimating small propensities can be beneficial ([paper PDF](https://www.cs.cornell.edu/~schnabts/downloads/schnabel2016mnar.pdf)). **[WS]**

**Doubly robust and its 2024–2026 successors.** DR combines an imputation model with IPS and is unbiased if *either* is correct. The 2024–2026 literature is mostly about relaxing DR's hidden requirements:
- Li et al. (ICML 2024) show standard DR needs *accurate pseudo-labels*, which rarely hold, and construct estimators unbiased when pseudo-labels have only user- or item-specific bias, plus attention-based propensity reconstruction for variance control ([PMLR v235](https://proceedings.mlr.press/v235/li24cq.html)). **[PS]**
- The 2025 "correlated latent exogenous variables" line shows *all* of IPS/DR/EIB degrade sharply as the correlation ρ between the latent drivers of observation and of outcome grows; they propose joint likelihood maximization recovering ρ, with stable MSE ~1.5–2.1 across ρ where baselines blow up ([arXiv 2506.07517](https://arxiv.org/html/2506.07517v1)). **[PS]** This is *exactly* our situation: the agent's decision to run a config and the config's true performance share latent causes (the asset's recent volatility, a human's prior).
- Full lineage worth citing in code comments: Schnabel 2016 → Wang et al. DR 2019 → MRDR → DR-BIAS/DR-MSE → TDR-CL, Multi-DR, ESCM²-DR → 2024–26 relaxations.

**The estimator I'd actually build.** Given the extreme-propensity problem (agents concentrate hard; some cells have π ≈ 10⁻⁴ where IPW variance explodes), the **joint-likelihood MNAR tensor completion** is the better default:
> Model `Y_ijk | X_ijk` in an exponential family with link `h`, with `X = Σ_r λ_r u_r ∘ v_r ∘ w_r` (CP), and model observation as `D_ijk ~ Bernoulli(g_θ(X_ijk))` with `g_θ = logit⁻¹(b₀ + b₁·X_ijk)`. Estimate factors and (b₀,b₁) by alternating maximization of the joint log-likelihood.
Non-asymptotic error bounds hold with observation probabilities **arbitrarily close to 0 or 1**, requiring only *slice-wise averages* to be bounded — much weaker than IPW's uniform-boundedness requirement. It includes a **sample-splitting hypothesis test of H₀: b₁ = 0 (MCAR) vs MNAR**, which is directly shippable as a platform diagnostic ([arXiv 2509.06225](https://arxiv.org/pdf/2509.06225)). **[WS theoretically; PS empirically — validated on simulations and InCarMusic, not on experiment ledgers]**

→ **Decision:** log propensities (you already plan to), fit **both** (a) propensity-weighted CP with clipped/self-normalized weights and (b) joint-likelihood MNAR CP, and **report the MCAR test statistic `b₁` as a platform health metric**. If `b₁` is significantly nonzero — it will be — that is quantitative proof the naive tensor is biased, and it is the number to show a skeptical user.

### 7.4 Right-censored outcomes from early-stopped runs

Early stopping (ASHA/Hyperband, or an agent's kill decision) produces **right-censored** performance: you know the run would have scored *at least* / *at most* something, not what. Treating the truncated value as the final value biases the tensor toward configs that look good early.

Three shippable treatments, in increasing sophistication **[PS]**:
1. **AFT / Tobit objective.** LightGBM and XGBoost both ship an **Accelerated Failure Time survival objective** with interval-censored labels `[y_lo, y_hi]`. For a run killed at rung r with observed partial metric m, encode the label as an interval. This is a one-line change from a regression objective and is the highest ROI move.
2. **Learning-curve extrapolation with uncertainty.** Fit a parametric curve family (pow₃, MMF, Weibull) per (strategy_family, asset_cluster) and impute the censored endpoint *with its posterior variance*, then feed a heteroskedastic-noise factorization. PASHA's progressive resource allocation is the reference for making the stopping itself less biased ([AutoML 2022](https://2022.automl.cc/wp-content/uploads/2022/07/pasha_efficient_hpo_with_progr.pdf)).
3. **Conformal quantile regression for HPO** (Salinas et al., ICML 2023) gives distribution-free prediction intervals on the final metric given partial curves ([PMLR v202](https://proceedings.mlr.press/v202/salinas23a/salinas23a.pdf)). Use these intervals as the interval labels in (1). **[WS as a method]**

Also record the **censoring reason** as a categorical: `completed | early_stop_rung | agent_kill | budget_exhausted | error | data_gap`. Only `completed` and `early_stop_rung` are informative censoring you can model; `error` and `data_gap` are MCAR-ish and should be excluded from propensity modeling.

### 7.5 Off-policy evaluation over the experiment log — and its failure mode

The seductive idea is: use the ledger to evaluate a *new* search policy without running it. This works, with a specific, documented trap.

**Hyperparameter optimization using off-policy estimators is actively dangerous.** Selecting hyperparameters by maximizing an IPS/DR estimate on logged validation data produces "an extremely optimistic proxy of generalization performance," and can select a policy **worse than the logging policy you started from**: with a near-optimal logger, baseline HPO selected policies **~15% worse than the logging policy itself**. The fix (CIR-HPO) is two-part: a **conservative surrogate** (optimize a high-probability *lower* bound via a Student-t bound rather than the point estimate) and **adaptive imitation regularization** (mix toward the logging policy with a weight set by a statistical test). Gains of 23.6% from the regularization alone in one regime ([arXiv 2404.15084](https://arxiv.org/html/2404.15084v1)). **[WS]**

→ **Decision:** every recommendation the platform makes from the ledger is scored by a **lower confidence bound**, never a point estimate, and is **shrunk toward the current default/logging policy** by an amount determined by a significance test. This is one function, it is the difference between a useful recommender and a confident liar, and it should be non-bypassable.

### 7.6 Acquisition: how the recommendation becomes an experiment

Given a completed tensor `T̂` with uncertainty, the agent's next-experiment choice is a **bandit over (asset, config) cells** with a nonstationary reward. Use Thompson sampling over the posterior of the factorization (sample factor matrices, compute implied cell values, pick argmax), which naturally handles exploration and is trivially parallelizable across the thousands of experiments/day. Log `π(cell)` = the empirical selection probability under the sampled policy — **this is the propensity that makes §7.3 work.** Add a floor `π ≥ ε` (ε-greedy mixture, ε ~ 0.05) so no cell has zero propensity; IPW is undefined otherwise and the MNAR model's slice-wise condition degrades. **[WS as standard practice]**

---

## 8. Mathematical caveats and embedding validation

### 8.1 Distance concentration and hubness

- **Concentration**: as d → ∞ with i.i.d.-ish coordinates, `(max‖x−q‖ − min‖x−q‖)/min‖x−q‖ → 0`; nearest neighbors stop being meaningful (Beyer et al. 1999; [JMLR 18 on intrinsically high-dimensional spaces](https://jmlr.org/papers/volume18/17-151/17-151.pdf)). **[WS]**
- **Hubness** is the more practically damaging effect: in high dimension, a few points become nearest neighbors of disproportionately many queries, wrecking kNN-based retrieval and recommendation ([Radovanović, Nanopoulos & Ivanović, JMLR 11:2487, 2010](https://www.jmlr.org/papers/volume11/radovanovic10a/radovanovic10a.pdf)). **[WS]**
- **What saves us**: *intrinsic* dimension, not nominal. A 150-dim fingerprint whose PCA spectrum shows 90% variance in 12 components behaves like a 12-dim space. So:
  → **Measure intrinsic dimension** (two-NN / MLE estimator, or participation ratio of the eigenvalue spectrum) and publish it next to the embedding.
  → **Apply hubness reduction**: mutual-proximity or local scaling on the similarity matrix. Cheap, ~10 lines, meaningfully improves retrieval precision.
  → **Keep the retrieval vector small** (target 32–64 dims after whitening/PCA) and keep the full fingerprint for modeling.

### 8.2 Metric choice and normalization

| Metric | Use for | Don't use for |
|---|---|---|
| **Correlation distance √(2(1−ρ))** | co-movement clustering of returns | comparing *feature vectors* (already a different space) |
| **Euclidean on whitened features** | fingerprint retrieval | raw unnormalized features (units dominate) |
| **Cosine** | sparse/count-like coordinates, direction-only landmark rank vectors | features where magnitude is the signal (volatility level!) |
| **Wasserstein-1/2** | comparing return *distributions* (regimes) — the Horvath result | high-dim feature vectors (cost + concentration) |
| **MMD (Gaussian kernel)** | validating regime cluster homogeneity | as a routing metric (no index support) |
| **DTW** | lead–lag alignment between two price paths | general asset similarity — warping destroys the √t time scaling and the whole point of a fixed sampling frequency |
| **Path signatures / rough paths** | order-sensitive path features; genuinely principled for non-Markovian path functionals ([signature methods in finance](https://github.com/kormilitzin/the-signature-method-in-machine-learning); [Neural RDEs for portfolio optimization, arXiv 2510.10728](https://arxiv.org/html/2510.10728v2)) | production v1 — truncation level 3 on a 5-dim stream is already 155 terms, and lead–lag/time augmentation choices are unstandardized. **[SPEC]** |

**Normalization discipline** (this is where leakage hides):
1. All feature statistics (mean, std, quantiles for rank transforms, PCA/whitening matrices) must be estimated on an **expanding window strictly before** the evaluation point, and **versioned**. Full-sample z-scoring is the canonical leak and is explicitly named as the mechanism in the spurious-predictability paper below.
2. Prefer **cross-sectional rank transforms within a (timestamp, asset_class)** group over time-series z-scores — they are leak-free by construction and robust to fat tails.
3. Whitening: use **shrunk** covariance (Ledoit–Wolf) for the whitening transform, not the sample covariance, or whitening amplifies noise directions.

### 8.3 The falsification requirement

The 2026 "Spurious Predictability in Financial Machine Learning" paper gives the cleanest statement of the problem and a shippable protocol ([arXiv 2604.15531](https://arxiv.org/html/2604.15531v1)) **[PS, but theoretically grounded]**:
- Under a global null (martingale-difference returns), the in-sample winner's apparent effect scales as **Θ(√log K_eff)** where K_eff is the *effective* number of independent strategies. Simulations suggest **400 nominal strategies ≈ 100–150 effective** under realistic correlation.
- Recommended: (i) **mandatory falsification screening** — run the complete pipeline on synthetic nulls (white noise, regime-switching vol, bid–ask bounce, factor null, GARCH); systematic non-zero performance on nulls invalidates the pipeline; (ii) strict walk-forward temporal disjointness including preprocessing; (iii) report a **Backtest Inflation Factor** and the in-sample-to-walk-forward gap ΔZ, not nominal K; (iv) audit the *end-to-end pipeline*, not components.
- Complementary: **Deflated Sharpe Ratio** (Bailey & López de Prado) corrects the observed SR for the number of trials, non-normality and sample length ([paper](https://www.davidhbailey.com/dhbpapers/deflated-sharpe.pdf)). With an immutable trial ledger we can compute the *true* trial count per hypothesis family — most practitioners cannot. **This is a genuine competitive advantage of the ledger design and should be surfaced in the UI.**
- **CPCV vs walk-forward:** a synthetic controlled-environment comparison found CPCV gives lower Probability of Backtest Overfitting and better DSR statistics than K-fold, purged K-fold and walk-forward, with walk-forward showing "notable shortcomings in false-discovery prevention" ([Arian, Norouzi & Seco, Knowledge-Based Systems 305, 2024](https://dl.acm.org/doi/10.1016/j.knosys.2024.112477); [SSRN](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4686376)). **[PS — synthetic only; and note the tension with the walk-forward insistence of the spurious-predictability paper. Resolution: use CPCV for *model selection* and a single final walk-forward for the *reported* number.]**

### 8.4 How to prove an embedding space is informative

Four probes, all cheap, all runnable nightly. **The rule: an embedding channel is admitted only if it clears all four against the stated baseline.**

1. **Retrieval precision vs random.** For each anchor asset, retrieve top-k neighbors in the embedding. Score `P@k` against *held-out* labels the embedding never saw: same sector/category, same venue family, same underlying, top-decile realized-correlation partner in a **future** window. Baseline: random assets, and a "same-asset-class" stratified random. Require a lift with a bootstrap CI excluding zero.
2. **Downstream transfer lift.** Take a target asset. Warm-start its config search from (a) its k embedding-neighbors' best configs in the ledger, vs (b) k random assets' best configs, vs (c) the global portfolio (§4). Measure regret-vs-oracle after n trials, n ∈ {1,3,10}. **The global portfolio is the baseline that matters** — if neighbor-based warm-start doesn't beat a static 15-config portfolio, the embedding is decorative (this is the Auto-sklearn 2.0 lesson).
3. **Temporal stability.** Rank-correlation of the neighbor list computed at t and t−Δ; ARI of the induced clustering. An embedding whose 1-month neighbor overlap is < 0.5 cannot support a tensor whose rows are asset clusters.
4. **Probing / linear readout.** Train a linear (ridge / logistic) probe on the frozen embedding to predict held-out properties: realized vol decile, spread decile, asset class, venue, jump frequency. Report R²/AUC vs the same probe on (a) raw catch22 and (b) the §1 finance block. If the learned embedding cannot beat catch22 on a *linear* probe, it has not learned anything catch22 doesn't have.

Additionally track **alignment/uniformity** (Wang & Isola) for contrastive embeddings as a training diagnostic, and **intrinsic dimension** (§8.1) as a capacity diagnostic.

---

## 9. Similarity-search infrastructure (2026)

### 9.1 Do the arithmetic first — you probably don't need a vector database

Our realistic scale:
- Instruments: ~10⁴–10⁵ (including options *underlyings*, not contracts).
- Embedding versions retained: ~10–20.
- Windows per instrument (multi-horizon fingerprints): ~5–10.
- → **10⁶–10⁷ vectors at the extreme upper bound; 10⁵–10⁶ realistically.**

At 64-dim float32 that is 256 B/vector → **256 MB for 10⁶ vectors**. An exact brute-force search is a single BLAS `sgemm`: 10⁶ × 64 = 6.4×10⁷ MACs per query, which is **~1 ms on one CPU core** and trivially batched. Even at 512 dims and 10⁷ vectors (20 GB) it is a bounded, parallelizable scan.

Meanwhile a survey of production pgvector experience puts real-world cost at **~20–25 KB per vector** including index overhead for 1,536-dim embeddings, with 1M vectors "naive implementations often succeed," ~10M needing quantization/partitioning, ~100M questioning in-memory HNSW, and **>1B meaning Postgres is no longer the simplest design** ([ClickHouse engineering guide](https://clickhouse.com/resources/engineering/scale-vector-search-postgres)). **[PS — vendor-adjacent but consistent with independent reports]**

→ **Decision: exact kNN in Postgres/NumPy is strictly better for v1 and probably for v3.** Concretely:
- **Primary**: `pgvector` with **no ANN index at all** — a sequential scan with `<->`/`<=>` over a tenant-filtered, class-filtered subset. Because our queries are *always* filtered (tenant, asset class, embedding version, knowledge_time), the candidate set after filtering is usually 10³–10⁵, where exact search is both faster and exact.
- **Escape hatch**: add HNSW only when a measured p99 exceeds budget. Then use `halfvec` (2× density, up to 4,000 dims) and **`hnsw.iterative_scan = 'relaxed_order'`** with `hnsw.max_scan_tuples` (default 20,000) to avoid the post-filter recall collapse. pgvector 0.8's iterative scans are the specific fix for filtered vector search ([pgvector 0.8 release](https://www.postgresql.org/about/news/pgvector-080-released-2952); [2026 DBA index guide](https://www.dbi-services.com/blog/pgvector-a-guide-for-dba-part-2-indexes-update-march-2026/)).
- **If index size becomes the binding constraint**: `pgvectorscale` StreamingDiskANN with Statistical Binary Quantization — reported **21 MB vs 193 MB** HNSW index for 25k × 3072-dim. Same tuning knob: `query_rescore` (default 50) for full-precision re-ranking of compressed candidates.

### 9.2 If/when a dedicated store is warranted

| System | Fit for us | Note |
|---|---|---|
| **pgvector / pgvectorscale** | **Best** | Same transaction as the ledger; RLS for tenancy; no second consistency domain |
| **Qdrant** | Good fallback | Best-documented multi-tenancy: **one collection + payload field with `is_tenant=true`**, which co-locates tenant data for sequential disk reads; user-defined sharding for few large tenants; tiered multitenancy (1.16) promotes growing tenants to dedicated shards. **Cloud default cap: 1,000 collections/cluster → collection-per-tenant does not scale** ([docs](https://qdrant.tech/documentation/manage-data/multitenancy/)) |
| **LanceDB** | Good for the *offline* side | Columnar-on-object-store; RaBitQ quantization; natural fit for the immutable trial ledger + feature snapshots as Lance/Parquet |
| **Milvus** | Overkill | IVF_RABITQ available; operational weight not justified below 10⁸ |
| **Turbopuffer** | Interesting cost profile | Object-storage-native; adds an external data dependency for a multi-tenant financial product |
| **Vespa** | Overkill | Excellent hybrid ranking, heavy to operate |

**Index families**: HNSW (graph, best recall/latency, high memory, poor with selective filters unless iterative), IVF (cheap build — ~5 s vs ~29 s for 25k×3072 — coarse quantization, `probes ≈ √lists`), DiskANN (SSD-resident, best memory/$, needs rescoring). **Quantization**: RaBitQ (SIGMOD 2025) is the current best-in-class with a *theoretical* error bound, now in LanceDB and Milvus (`IVF_RABITQ`); plain binary quantization is fine for a coarse first-stage with exact re-ranking ([RaBitQ](https://dl.acm.org/doi/abs/10.1145/3654970)).

### 9.3 Multi-tenant concerns

1. **Isolation must be enforced at the storage layer, not the query layer.** Postgres **RLS** with a `tenant_id` policy plus `FORCE ROW LEVEL SECURITY` on the table owner. The classic bypass is the table owner / `BYPASSRLS` role and functions marked `SECURITY DEFINER`; audit these.
2. **The vector index is a side channel.** A shared ANN index leaks *approximate* neighbor structure across tenants if filtering is applied post-search. With exact-scan-after-filter (§9.1) this class of bug cannot occur. This is an underrated security argument for exact kNN.
3. **Shared vs private embedding spaces.** Asset fingerprints and regimes are **platform-global** (derived from public market data — no tenant contamination). The **outcome tensor is the sensitive object**: it encodes what each tenant discovered. Design decision to make explicitly: a *global* tensor built from all tenants' trials is far more valuable and is a privacy/IP disclosure. Options: (a) tenant-private tensors only; (b) global tensor with **k-anonymity thresholds** (a cell is admitted to the global tensor only if ≥ k distinct tenants contributed); (c) opt-in federation with per-tenant differential-privacy noise on the factor updates. **[SPEC — needs a product/legal decision, not a technical one. Default to (a) + (b) with k ≥ 5 and an explicit opt-in.]**
4. **Noisy-neighbor**: per-tenant statement timeouts and a work-queue with fair scheduling; one tenant's 10,000-experiment agent sweep must not starve another's.

---

## 10. Concrete build order

**Phase 0 — Data spine (blocking everything).**
Bitemporal minute-bar store with `valid_time` and `knowledge_time`; instrument registry with asset-class discriminator, venue, session model, roll rule; explicit masks (`is_roll_window`, `is_stale`, `is_halted`, `has_orderflow`). Seasonality deflators as versioned artifacts. *Nothing below is trustworthy without this.*

**Phase 1 — Hand-crafted fingerprint + landmarker portfolio.** (highest confidence, lowest cost)
- §1 feature block: EDGE spread, RV/BV/jump-ratio at 3 horizons, vol-of-vol, robust skew/kurtosis, variance ratios, Amihud, Kyle λ, turnover, seasonality profile, overnight/intraday split (NULL-aware), term structure (futures), surface params (options), TVL/LVR block (DeFi).
- catch22 + TSFEL-subset, computed on **deflated, masked** returns.
- 12 landmarkers × ~8 diagnostics, rank-transformed (relative landmarkers).
- Nightly feature-stability job (§1.4).
- Exact kNN retrieval in Postgres. **Ship the §8.4 probe suite at the same time as the embedding, not after.**

**Phase 2 — Clustering + regimes.**
- Denoise (Marchenko–Pastur) → detone → correlation distance → Ward; k by bootstrap Jaccard stability; publish temporal ARI.
- Global regime (WK-means on Wasserstein distance over the reference basket; or a 3–4 state HMM with **filtered** probabilities) + local regime. Publish dwell times and regime-label stability.

**Phase 3 — Outcome tensor.**
- Materialize `T` from the ledger. Log propensities with an ε-floor from day one — **retrofitting propensities is impossible**.
- Fit: (a) CP with content side-information (cold start), (b) propensity-weighted variant, (c) joint-likelihood MNAR variant with the `b₁` MCAR test.
- AFT/interval-censored labels for early-stopped runs.
- Recommendations returned as **lower confidence bounds shrunk toward the logging/default policy** (CIR-HPO discipline).
- Thompson sampling over the posterior for next-experiment selection.

**Phase 4 — Learned embeddings (gated).**
- Frozen Chronos-2 / Toto-2 encoder embeddings, mean-pooled, PCA'd to 32–64 dims, **concatenated** to Phase-1 features.
- Optional SSL encoder with financially valid augmentations and a structural objective.
- **Gate:** must beat the Phase-1 fingerprint on all four probes of §8.4, with the global-portfolio baseline in probe 2. If it doesn't, don't ship it; log the negative result to the ledger.

**Phase 5 — Scale-outs, only if measured.** ANN index; Qdrant/LanceDB; signature features; GNNs.

---

## 11. Schema DDL sketches

Design rules encoded below: **(1)** every derived artifact carries `knowledge_time` (when the platform could have known it) *and* `valid_time` (the market time it describes); **(2)** every derived artifact carries the version of everything upstream; **(3)** nothing is ever updated in place — new rows supersede old ones; **(4)** tenant isolation is RLS, not application code.

```sql
-- =====================================================================
-- 0. CONVENTIONS
-- =====================================================================
CREATE EXTENSION IF NOT EXISTS vector;      -- pgvector >= 0.8
CREATE EXTENSION IF NOT EXISTS btree_gist;  -- for exclusion constraints on ranges

CREATE TYPE asset_class   AS ENUM ('crypto_spot','crypto_perp','future','option',
                                   'equity','etf','defi_pool','fx');
CREATE TYPE session_model AS ENUM ('continuous_24_7','exchange_session',
                                   'futures_session','block_time');
CREATE TYPE censor_reason AS ENUM ('completed','early_stop_rung','agent_kill',
                                   'budget_exhausted','error','data_gap');

-- =====================================================================
-- 1. INSTRUMENT REGISTRY (bitemporal: instruments get relisted, renamed,
--    change tick size, change roll rule)
-- =====================================================================
CREATE TABLE instrument (
  instrument_id   BIGINT      PRIMARY KEY,
  symbol          TEXT        NOT NULL,
  venue           TEXT        NOT NULL,
  asset_class     asset_class NOT NULL,
  session_model   session_model NOT NULL,
  quote_ccy       TEXT,
  underlying_id   BIGINT      REFERENCES instrument(instrument_id),  -- options/futures
  roll_rule_id    TEXT,          -- NULL unless continuous future
  -- options coordinates (deterministic, not learned)
  opt_strike      NUMERIC,
  opt_expiry      DATE,
  opt_cp          CHAR(1),
  -- defi coordinates
  chain_id        INT,
  pool_address    TEXT,
  fee_tier_bps    INT,
  venue_lag_min   REAL,          -- measured CEX->DEX price lag, see §1.2
  -- capability flags: which feature blocks are computable
  has_orderflow   BOOLEAN NOT NULL DEFAULT FALSE,
  has_session     BOOLEAN NOT NULL DEFAULT FALSE,
  valid_from      TIMESTAMPTZ NOT NULL,
  valid_to        TIMESTAMPTZ NOT NULL DEFAULT 'infinity',
  knowledge_time  TIMESTAMPTZ NOT NULL DEFAULT now(),
  EXCLUDE USING gist (
    instrument_id WITH =,
    tstzrange(valid_from, valid_to) WITH &&
  )
);

-- =====================================================================
-- 2. FEATURE / EMBEDDING DEFINITIONS (so a vector is never ambiguous)
-- =====================================================================
CREATE TABLE embedding_space (
  space_id        TEXT PRIMARY KEY,        -- 'fingerprint.v3', 'landmark.v2', 'chronos2.mean.v1'
  dim             INT  NOT NULL,
  kind            TEXT NOT NULL,           -- 'handcrafted'|'landmarker'|'tsfm'|'ssl'|'graph'
  metric          TEXT NOT NULL,           -- 'l2_whitened'|'cosine'|'corr'
  feature_names   TEXT[],                  -- ordered; NULL for opaque encoders
  estimator_spec  JSONB NOT NULL,          -- {sampling:'1min', window_days:30, deflator:'seas.v2',
                                           --  hurst_estimator:'rv_ols', jump:'threshold_bipower', ...}
  code_git_sha    TEXT NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  deprecated_at   TIMESTAMPTZ
);

-- =====================================================================
-- 3. ASSET FINGERPRINTS (append-only, bitemporal)
--    valid_time  = end of the market window the features describe
--    knowledge_time = when this row became computable/available
-- =====================================================================
CREATE TABLE asset_embedding (
  instrument_id   BIGINT      NOT NULL REFERENCES instrument(instrument_id),
  space_id        TEXT        NOT NULL REFERENCES embedding_space(space_id),
  valid_from      TIMESTAMPTZ NOT NULL,   -- window start
  valid_to        TIMESTAMPTZ NOT NULL,   -- window end == the "as-of" market time
  knowledge_time  TIMESTAMPTZ NOT NULL,   -- >= valid_to + computation lag
  vec             VECTOR(64)  NOT NULL,   -- whitened/PCA'd RETRIEVAL vector
  raw             JSONB       NOT NULL,   -- full named feature block incl. NULLs
  n_obs           INT         NOT NULL,   -- bars used; gate on this
  null_mask       BIT(256),               -- which features were NOT computable
  quality_score   REAL,                   -- coverage * (1 - stale_fraction)
  PRIMARY KEY (instrument_id, space_id, valid_to, knowledge_time)
);
-- Deliberately NO ANN index: queries always filter by space_id + valid_to +
-- asset_class, leaving 1e3-1e5 candidates where exact scan wins (see §9.1).
CREATE INDEX ON asset_embedding (space_id, valid_to DESC);
CREATE INDEX ON asset_embedding (instrument_id, space_id, valid_to DESC);

-- Point-in-time view: "what did we know at time T about window ending <= T"
CREATE VIEW asset_embedding_asof AS
SELECT DISTINCT ON (instrument_id, space_id) *
FROM   asset_embedding
WHERE  knowledge_time <= current_setting('app.asof')::timestamptz
ORDER  BY instrument_id, space_id, valid_to DESC, knowledge_time DESC;

-- =====================================================================
-- 4. LANDMARKER RESULTS (the performance-based task representation)
-- =====================================================================
CREATE TABLE landmarker_run (
  instrument_id   BIGINT      NOT NULL,
  landmarker_id   TEXT        NOT NULL,   -- 'tsmom.fast.v1', 'har_rv.v1', ...
  eval_window     TSTZRANGE   NOT NULL,
  knowledge_time  TIMESTAMPTZ NOT NULL,
  bar_version     TEXT        NOT NULL,
  -- diagnostics vector, NOT just a scalar (see §4.2)
  sharpe_oos      REAL, dsr REAL, hit_rate REAL, turnover REAL,
  gross_net_gap   REAL, max_dd REAL, t_stat REAL, r2_oos REAL,
  horizon_decay   REAL[],                 -- score by forward horizon
  runtime_ms      INT,
  PRIMARY KEY (instrument_id, landmarker_id, eval_window, knowledge_time)
);
-- The landmark fingerprint is the RANK-TRANSFORM of these within each
-- (eval_window, asset_class) batch -> relative landmarkers.

-- =====================================================================
-- 5. CLUSTERS AND REGIMES (versioned partitions, with stability metrics)
-- =====================================================================
CREATE TABLE cluster_run (
  cluster_run_id  BIGSERIAL PRIMARY KEY,
  space_id        TEXT NOT NULL REFERENCES embedding_space(space_id),
  method          TEXT NOT NULL,          -- 'ward_corr_denoised_detoned'
  params          JSONB NOT NULL,
  fit_window      TSTZRANGE NOT NULL,
  knowledge_time  TIMESTAMPTZ NOT NULL,
  k               INT NOT NULL,
  bootstrap_jaccard REAL[],               -- per cluster, Hennig stability
  ari_vs_prev_1w  REAL, ari_vs_prev_1m REAL, ari_vs_prev_3m REAL
);
CREATE TABLE cluster_member (
  cluster_run_id  BIGINT NOT NULL REFERENCES cluster_run(cluster_run_id),
  instrument_id   BIGINT NOT NULL,
  cluster_id      INT    NOT NULL,
  silhouette      REAL,
  PRIMARY KEY (cluster_run_id, instrument_id)
);

CREATE TABLE regime_state (
  regime_model_id TEXT        NOT NULL,   -- 'global.wkmeans.v2' | 'global.hmm4.v1'
  scope           TEXT        NOT NULL,   -- 'global' or instrument_id::text
  valid_time      TIMESTAMPTZ NOT NULL,
  knowledge_time  TIMESTAMPTZ NOT NULL,
  regime_id       SMALLINT    NOT NULL,
  p_filtered      REAL[]      NOT NULL,   -- ONLY causal probabilities (see §6.2)
  run_length      INT,                    -- BOCPD posterior mode: bars since change
  PRIMARY KEY (regime_model_id, scope, valid_time, knowledge_time)
);
-- p_smoothed / viterbi live in schema `research_only`, which has no GRANT to
-- the backtest role. Enforce with: REVOKE ALL ON SCHEMA research_only FROM bt_role;

-- =====================================================================
-- 6. IMMUTABLE TRIAL LEDGER (append-only; propensities logged at decision time)
-- =====================================================================
CREATE TABLE trial (
  trial_id        UUID PRIMARY KEY,
  tenant_id       UUID        NOT NULL,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  -- task coordinates
  instrument_id   BIGINT      NOT NULL,
  cluster_run_id  BIGINT,                 -- cluster membership AT DECISION TIME
  cluster_id      INT,
  regime_model_id TEXT, regime_id SMALLINT,
  -- what was run
  strategy_family TEXT        NOT NULL,
  config          JSONB       NOT NULL,
  config_vec      VECTOR(32),             -- content features for cold-start MF
  config_hash     TEXT        NOT NULL,
  -- data + code provenance
  bar_version     TEXT NOT NULL, feature_space_id TEXT, code_git_sha TEXT NOT NULL,
  train_window    TSTZRANGE NOT NULL, test_window TSTZRANGE NOT NULL,
  cv_scheme       TEXT NOT NULL,          -- 'cpcv(n=6,k=2,embargo=1d)' | 'walkforward'
  -- DECISION LOGGING  (§7.6) -- must be written BEFORE the run starts
  selector_policy_id TEXT NOT NULL,       -- which agent/policy chose this
  propensity      DOUBLE PRECISION NOT NULL CHECK (propensity > 0),
  propensity_floor DOUBLE PRECISION NOT NULL,
  candidate_set_size INT NOT NULL,
  -- OUTCOME (censoring-aware)
  status          censor_reason NOT NULL,
  metric_name     TEXT, metric_value DOUBLE PRECISION,
  metric_lo       DOUBLE PRECISION,       -- interval label for AFT/Tobit (§7.4)
  metric_hi       DOUBLE PRECISION,       -- = +inf for right-censored
  n_trades INT, gross_sharpe REAL, net_sharpe REAL, dsr REAL,
  rung_reached INT, budget_used_s REAL,
  metrics_full    JSONB
);
ALTER TABLE trial ENABLE ROW LEVEL SECURITY;
ALTER TABLE trial FORCE  ROW LEVEL SECURITY;
CREATE POLICY trial_tenant ON trial
  USING (tenant_id = current_setting('app.tenant_id')::uuid);
-- Immutability:
CREATE RULE trial_no_update AS ON UPDATE TO trial DO INSTEAD NOTHING;
CREATE RULE trial_no_delete AS ON DELETE TO trial DO INSTEAD NOTHING;
CREATE INDEX ON trial (tenant_id, strategy_family, created_at DESC);
CREATE INDEX ON trial (cluster_run_id, cluster_id, regime_id, strategy_family);

-- =====================================================================
-- 7. OUTCOME TENSOR (materialized view of the ledger, one per metric)
-- =====================================================================
CREATE TABLE outcome_cell (
  tensor_id       TEXT        NOT NULL,   -- 'dsr.global.v1' | 'dsr.tenant.<uuid>.v1'
  cluster_run_id  BIGINT      NOT NULL,
  cluster_id      INT         NOT NULL,
  regime_id       SMALLINT    NOT NULL,
  strategy_family TEXT        NOT NULL,
  config_bucket   INT         NOT NULL,
  knowledge_time  TIMESTAMPTZ NOT NULL,
  n_trials        INT         NOT NULL,
  n_distinct_tenants INT      NOT NULL,   -- k-anonymity gate for the global tensor
  y_mean          DOUBLE PRECISION,
  y_var           DOUBLE PRECISION,
  -- MNAR machinery (§7.3)
  mean_propensity DOUBLE PRECISION,
  ips_weight_sum  DOUBLE PRECISION,
  snips_estimate  DOUBLE PRECISION,
  censored_frac   REAL,
  PRIMARY KEY (tensor_id, cluster_run_id, cluster_id, regime_id,
               strategy_family, config_bucket, knowledge_time)
);

CREATE TABLE tensor_factorization (
  tensor_id       TEXT NOT NULL,
  fit_id          UUID PRIMARY KEY,
  knowledge_time  TIMESTAMPTZ NOT NULL,
  method          TEXT NOT NULL,          -- 'cp_side'|'cp_ips'|'cp_mnar_joint'
  rank            INT  NOT NULL,
  factors         JSONB NOT NULL,         -- or pointers to array storage
  -- MNAR diagnostics: the platform health metric of §7.3
  mnar_b1         DOUBLE PRECISION,       -- missingness slope
  mnar_b1_se      DOUBLE PRECISION,
  mnar_pvalue     DOUBLE PRECISION,       -- H0: MCAR
  holdout_rmse    DOUBLE PRECISION,
  holdout_rmse_ips DOUBLE PRECISION
);

CREATE TABLE recommendation (
  rec_id          UUID PRIMARY KEY,
  tenant_id       UUID NOT NULL,
  fit_id          UUID REFERENCES tensor_factorization(fit_id),
  instrument_id   BIGINT NOT NULL,
  strategy_family TEXT NOT NULL,
  config          JSONB NOT NULL,
  point_estimate  DOUBLE PRECISION,
  lower_conf_bound DOUBLE PRECISION NOT NULL,  -- what we actually rank on (§7.5)
  shrinkage_alpha REAL NOT NULL,               -- imitation weight toward default
  baseline_value  DOUBLE PRECISION,            -- logging/default policy value
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

**Notes on the DDL.**
- The `EXCLUDE USING gist` on `instrument` enforces non-overlapping validity intervals — the cheapest way to prevent the "two truths about the same symbol" bug.
- `propensity > 0` is a CHECK constraint, not a convention: an IPW estimator with a zero propensity is undefined and a zero here is a data-corruption event.
- `trial` writes the decision block **before** execution. If you write propensity after seeing the outcome, it is not a propensity.
- `n_distinct_tenants` on `outcome_cell` is the k-anonymity gate for any cross-tenant global tensor (§9.3).

---

## 12. Validation protocols (what runs nightly)

**P1 — Feature stability.** Per feature: rank-autocorrelation at 1d/1w/1m, split-half reliability, non-degenerate coverage. Demote below threshold. *Output: a stability dashboard and an auto-updated `retrieval_features` allowlist.*

**P2 — Embedding informativeness.** The four probes of §8.4 (retrieval precision vs random & stratified-random; transfer lift vs global portfolio; temporal neighbor stability; linear probe vs catch22 baseline). Every embedding space, every night, results appended to the ledger. **An embedding space with no green probe for 7 consecutive days is auto-deprecated.**

**P3 — Cluster/regime stability.** Bootstrap Jaccard per cluster; temporal ARI at 1w/1m/3m; regime dwell-time distribution; fraction of time in each regime. Alert on ARI(1m) < 0.5.

**P4 — Leakage falsification.** Run the *complete* pipeline (features → clustering → regime → strategy → backtest) on five synthetic nulls: iid Gaussian, GARCH(1,1), regime-switching vol, bid–ask-bounce, factor null. Any systematically non-zero net Sharpe is a **pipeline bug**, not a discovery. Publish the Backtest Inflation Factor per strategy family.

**P5 — Multiplicity accounting.** For every reported result, compute the Deflated Sharpe Ratio using the **actual trial count from the ledger** for that hypothesis family, plus the effective-multiplicity correction (eigenvalue-based K_eff from the correlation matrix of the family's trial returns). Show both nominal and effective K in the UI.

**P6 — MNAR health.** Fit the joint-likelihood model; report `b₁` and its p-value. Report the distribution of logged propensities (a heavy left tail near the floor means the agent has collapsed to exploitation — raise ε).

**P7 — OPE safety.** For each recommendation batch, compare the LCB-ranked selection's estimated value to the logging policy's value with a significance test. If the recommender cannot demonstrate improvement over the default at the chosen confidence, it must return the default (CIR-HPO discipline, §7.5).

**P8 — Point-in-time reproducibility.** Sample N historical trials; re-materialize their inputs using `knowledge_time <= trial.created_at`; assert bit-identical features. This catches the single most expensive class of bug on this platform.

---

## 13. Sources

**Feature sets & time-series characteristics**
- catch22 — https://arxiv.org/abs/1901.10200 · https://github.com/DynamicsAndNeuralSystems/catch22
- Henderson & Fulcher, *An Empirical Evaluation of Time-Series Feature Sets* — https://arxiv.org/pdf/2110.10914
- *Statistical comparisons of time-series feature sets on classification tasks* (2026) — https://arxiv.org/html/2608.01586
- Middlehurst et al., *Bake off redux* — https://arxiv.org/html/2304.13029
- *Benchmarking Time Series Feature Extraction for Algorithm Selection* — https://link.springer.com/chapter/10.1007/978-3-032-05176-9_21
- tsfresh feature filtering (Benjamini–Yekutieli FDR) — https://tsfresh.readthedocs.io/en/latest/text/feature_filtering.html

**Microstructure & minute-bar estimators**
- Ardia, Guidotti & Kroencke, *Efficient Estimation of Bid-Ask Spreads from OHLC Prices* (JFE 2024) — https://acfr.aut.ac.nz/__data/assets/pdf_file/0016/570202/Efficient_Estimation_of_Bid_Ask_Spreads_from_OHLC_Prices-39.pdf · code https://github.com/eguidotti/bidask
- Cont & Das, *Rough volatility: fact or artefact?* — https://arxiv.org/pdf/2203.13820
- *Rough Volatility Across Assets* (2026) — https://arxiv.org/abs/2608.16749
- *Periodicity in Cryptocurrency Volatility and Liquidity* — https://arxiv.org/pdf/2109.12142
- Pindza, *Microstructure alpha: hierarchical learning and cross-asset transfer in cryptocurrency markets* (Frontiers in Blockchain 2026) — https://www.frontiersin.org/journals/blockchain/articles/10.3389/fbloc.2026.1811716/full
- Cont, Kukanov & Stoikov, *The price impact of order book events* — https://arxiv.org/pdf/1011.6402
- *The cross-section of intraday and overnight returns* (JFE 2021) — https://www.sciencedirect.com/science/article/abs/pii/S0304405X21000854
- *Intraday and overnight return anomalies: 11.6 million price observations* (FRL 2025) — https://www.sciencedirect.com/science/article/abs/pii/S1544612325018926

**Time-series foundation models & benchmarks**
- fev-bench — https://arxiv.org/pdf/2509.26468 · https://github.com/autogluon/fev
- GIFT-Eval — https://arxiv.org/html/2410.10393v2 · https://github.com/SalesforceAIResearch/gift-eval · leaderboard mirror https://tsfm.ai/benchmarks/gift-eval
- *It's TIME: Towards the Next Generation of Time Series Forecasting Benchmarks* (2026) — https://arxiv.org/html/2602.12147v3
- *Rethinking Evaluation in the Era of TSFMs: (Un)known Information Leakage* — https://arxiv.org/html/2510.13654v3
- *TSFMAudit: Data Contamination Auditing* — https://arxiv.org/html/2605.26161
- Chronos-2 — https://arxiv.org/pdf/2510.15821 · https://huggingface.co/amazon/chronos-2
- TiRex — https://arxiv.org/abs/2505.23719 · https://github.com/NX-AI/tirex
- Toto 2.0 / BOOM — https://www.datadoghq.com/blog/ai/toto-2/ · https://arxiv.org/abs/2505.14766
- TabPFN-TS — https://arxiv.org/html/2501.02945v2
- *Are Time-Series Foundation Models Ready for E-Nose Data? An Empirical Assessment of Their Embeddings* (2026) — https://arxiv.org/html/2606.27672
- *Foundation Time-Series AI Model for Realized Volatility Forecasting* — https://ideas.repec.org/p/arx/papers/2505.11163.html
- FinCast — https://arxiv.org/html/2508.19609v1
- Kronos — https://arxiv.org/html/2508.02739v1

**Self-supervised representation learning**
- TS2Vec — https://ojs.aaai.org/index.php/AAAI/article/view/20881/20640
- CoST — https://arxiv.org/abs/2202.01575
- SimTS (critique of contrastive TS learning) — https://arxiv.org/html/2303.18205v2
- *Contrastive Learning of Asset Embeddings from Financial Time Series* — https://arxiv.org/html/2407.18645v1
- Gabaix, Koijen, Richmond & Yogo, *Asset Embeddings* (NBER w33651) — https://www.nber.org/system/files/working_papers/w33651/w33651.pdf
- Retrieval-Augmented Forecasting (RAFT) — https://arxiv.org/pdf/2505.04163

**Meta-learning, landmarkers, portfolios**
- Feurer et al., *Auto-sklearn 2.0* — https://ml.informatik.uni-freiburg.de/wp-content/uploads/papers/21-ARXIV-ASKL2.pdf
- Salinas & Erickson, *TabRepo* — https://arxiv.org/html/2311.02971v3 · https://github.com/autogluon/tabarena
- *AutoForecast: Evaluation-free Time-series Forecasting Model Selection via Meta-learning* (TKDD 2025) — https://engineering.purdue.edu/dcsl/wp-content/uploads/2025/02/AutoForecast_ACM_TKDD.pdf
- Montero-Manso et al., *FFORMA* — https://robjhyndman.com/papers/fforma.pdf
- PASHA (progressive resource allocation) — https://2022.automl.cc/wp-content/uploads/2022/07/pasha_efficient_hpo_with_progr.pdf
- Salinas et al., *Optimizing Hyperparameters with Conformal Quantile Regression* — https://proceedings.mlr.press/v202/salinas23a/salinas23a.pdf

**Clustering, covariance, correlation structure**
- *Hierarchical risk clustering versus traditional risk-based portfolios* (Empirical Economics 2026) — https://link.springer.com/article/10.1007/s00181-026-02900-x
- *Shrinkage Estimators for Mean and Covariance* (2026) — https://arxiv.org/html/2601.20643v1
- Hennig, *Cluster-wise assessment of cluster stability* — https://www.homepages.ucl.ac.uk/~ucakche/papers/clusta.pdf
- Adjusted Rand Index / chance correction — https://scikit-learn.org/stable/auto_examples/cluster/plot_adjusted_for_chance_measures.html
- GNNs for stock forecasting, systematic review (ACM CSUR 2024) — https://dl.acm.org/doi/10.1145/3696411
- RMT / Marchenko–Pastur denoising — https://portfoliooptimizer.io/blog/correlation-matrices-denoising-results-from-random-matrix-theory/

**Regimes**
- Horvath, Issa & Muguruza, *Clustering Market Regimes using the Wasserstein Distance* — https://arxiv.org/html/2110.11848v1
- HMM lookahead ladder (filtered vs smoothed Sharpe) — https://github.com/dmitridefreitas-dev/regime-detection
- HMM vs HSMM for regime-based asset allocation — https://papers.ssrn.com/sol3/Delivery.cfm/SSRN_ID4796238_code2469387.pdf?abstractid=4796238&mirid=1
- Bayesian autoregressive online change-point detection with time-varying parameters — https://arxiv.org/abs/2407.16376
- Alpha Architect on Mulliner/Harvey/Xia/Fang regime similarity — https://alphaarchitect.com/regime-detection/

**MNAR, matrix/tensor completion, off-policy**
- Schnabel, Swaminathan, Singh, Chandak & Joachims, *Recommendations as Treatments* (ICML 2016) — https://www.cs.cornell.edu/~schnabts/downloads/schnabel2016mnar.pdf
- *Missing Not at Random in Matrix Completion* (NeurIPS 2019) — https://arxiv.org/abs/1910.12774
- *Generalized Tensor Completion with Non-Random Missingness* (2025) — https://arxiv.org/pdf/2509.06225
- Li et al., *Relaxing the Accurate Imputation Assumption in Doubly Robust Learning* (ICML 2024) — https://proceedings.mlr.press/v235/li24cq.html
- *Addressing Correlated Latent Exogenous Variables in Debiased Recommender Systems* (2025) — https://arxiv.org/html/2506.07517v1
- *Hyperparameter Optimization Can Even be Harmful in Off-Policy Learning* (IJCAI 2024) — https://arxiv.org/html/2404.15084v1
- *Computationally Efficient and Minimax Optimal Nonignorable Matrix Completion* — https://arxiv.org/pdf/2504.04016

**Validation, overfitting, high-dimensional geometry**
- *Spurious Predictability in Financial Machine Learning* (2026) — https://arxiv.org/html/2604.15531v1
- *The Limits of Complexity: Why Feature Engineering Beats Deep Learning in Investor Flow Prediction* (2026) — https://arxiv.org/html/2601.07131v1
- Bailey & López de Prado, *The Deflated Sharpe Ratio* — https://www.davidhbailey.com/dhbpapers/deflated-sharpe.pdf
- Arian, Norouzi & Seco, *Backtest Overfitting in the Machine Learning Era* (KBS 2024) — https://dl.acm.org/doi/10.1016/j.knosys.2024.112477 · https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4686376
- Radovanović, Nanopoulos & Ivanović, *Hubs in Space* (JMLR 11:2487, 2010) — https://www.jmlr.org/papers/volume11/radovanovic10a/radovanovic10a.pdf
- *On the Behavior of Intrinsically High-Dimensional Spaces* (JMLR 18) — https://jmlr.org/papers/volume18/17-151/17-151.pdf

**Options / DeFi / futures representation**
- Arbitrage-free IV surface generation with VAEs (SIAM J. Fin. Math) — https://epubs.siam.org/doi/10.1137/21M1443546
- VolGAN (Applied Mathematical Finance 2025) — https://www.tandfonline.com/doi/full/10.1080/1350486X.2025.2471317
- Gatheral & Jacquier, arbitrage-free SVI — https://www.imperial.ac.uk/media/imperial-college/research-centres-and-groups/cfm-imperial-institute-of-quantitative-finance/events/distinguished-lectures/Gatheral-2nd-Lecture.pdf
- *Bounding LVR in AMMs* (2026) — https://arxiv.org/html/2605.19267
- *SoK: Impermanent Loss* (IACR 2026/1073) — https://eprint.iacr.org/2026/1073.pdf
- Continuous futures contract methodology — https://quantpedia.com/continuous-futures-contracts-methodology-for-backtesting/

**Infrastructure**
- pgvector 0.8.0 (iterative index scans) — https://www.postgresql.org/about/news/pgvector-080-released-2952
- pgvector index guide 2026 (HNSW/IVFFlat/DiskANN, halfvec, tuning) — https://www.dbi-services.com/blog/pgvector-a-guide-for-dba-part-2-indexes-update-march-2026/
- Scaling vector search in Postgres: memory, filtering, when to go hybrid — https://clickhouse.com/resources/engineering/scale-vector-search-postgres
- Qdrant multitenancy — https://qdrant.tech/documentation/manage-data/multitenancy/ · https://qdrant.tech/blog/qdrant-1.16.x/
- RaBitQ (SIGMOD 2025) — https://dl.acm.org/doi/abs/10.1145/3654970 · https://github.com/VectorDB-NTU/Extended-RaBitQ
- LanceDB RaBitQ — https://www.lancedb.com/blog/feature-rabitq-quantization
- Postgres RLS multi-tenant patterns and pitfalls — https://queryplane.com/blog/postgres-row-level-security-in-practice/
```
