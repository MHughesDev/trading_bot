# BS-007 / R2 — Quant techniques to give the agent as tools (research, 2026-09-10)

> **Informative.** This was doc 10 before the 2026-09-11 restructure. The normative rules
> derived from it live in **11 EVALUATION_STANDARD** (QC rules, dossier, evaluation
> tools), **10 MODELS** (families, scoring, calibration), **07 FEATURES** and
> **14 TOOLBOX**. Old refs map as in research/R1's header: 03 → 14, 06 → R3,
> 08 §x → 03/05–11, 09 → R1, 11 → 11.

Status: research summary + tool proposals. Complements 06 (the knowledge cards) with
**what the recent evidence says works**, and turns each technique into a tool or platform
capability. Evidence strength: **Strong** (replicated, standard) · **Moderate** (several
studies) · **Weak** (single study or vendor claim) · **Negative** (evidence against).

---

## 0. The one framing the evidence supports

A September 2026 review of AI in equity and crypto markets concludes that capability has
advanced but "no general AI architecture is shown to deliver persistent, cross-regime,
capacity-aware net alpha". It organises evidence along an **alpha-translation chain**:
point-in-time information → stable signal → feasible positions → executable orders → net
alpha after costs. A gain at one stage matters only if it survives every later stage.
Stronger evidence: nonlinear equity models, **cost-embedded portfolio optimisation over
cost-agnostic prediction**, LLM-extracted news signals, and execution algorithms. Weaker:
foundation-model forecasting, agent systems, and crypto-specific AI.
([arXiv 2609.04917](https://arxiv.org/abs/2609.04917))

**Tool:** `alpha_chain_audit(experiment_id)` returns a five-stage checklist for a
candidate. Each stage gets pass/fail/unknown, with evidence: PIT audit result, signal
stability across CPCV paths, position feasibility (leverage, shorting, lot sizes),
execution realism (fill model, latency, costs), and net-of-cost metrics plus capacity. The
agent can't report a candidate without it.

---

## 1. Validation: making results trustworthy

| Technique | Evidence | Tool / capability | Status here |
|---|---|---|---|
| **Matched-budget random-search baseline** | **Negative for agents** (Moderate): an adaptive agent tied random search on BTC perps under PIT audit; holdout had positive IC but negative net Sharpe ([2608.25348](https://arxiv.org/abs/2608.25348)) | Shadow random arm in every campaign; `compare_to_random(campaign)` | New |
| **Data-snooping-robust multiple testing**: White's Reality Check, Hansen SPA, stepwise SPA, Romano–Wolf | Strong method; on BTC/ETH, technical rules showed **no profitable strategies after snooping adjustment** in one comprehensive study ([Step-SPA](https://homepage.ntu.edu.tw/~ckuan/pdf/Step-SPA-20090720.pdf), [SPA](https://www.researchgate.net/publication/4724332_A_Test_for_Superior_Predictive_Ability)) | `family_test(experiment_ids, method=spa\|stepwise_spa\|romano_wolf)` over a campaign's candidates | New (Set J has DSR/PBO, not SPA) |
| **CPCV path distributions** + Bagged/Adaptive CPCV variants; a narrow, positive path distribution = robust | Strong for detecting overfit in synthetic tests ([KBS 2024](https://www.sciencedirect.com/science/article/abs/pii/S0950705124011110)) | Expose the per-path distribution in `get_diagnostics`; add bagged variant | CPCV exists |
| **Synthetic controlled environments**: Heston, Merton jumps, drift-burst, regime switching | Moderate: used to benchmark validators themselves | Eval-suite generators; `simulate_paths` process families | GARCH null exists |
| **Regime-aware segmentation** of train/test windows | Moderate | `run_study kind=regime_conditional` fed by the jump model | Study kind exists; labels don't |
| **PIT auditor validated on planted violations** | Moderate ([2608.25348](https://arxiv.org/abs/2608.25348)) | Gate 0 + CI suite of known violations | New |
| **Prospective paper test** (precommitted) | Strong recommendation ([2609.04917](https://arxiv.org/abs/2609.04917)) | Forward gate after live wiring (08 §8) | Blocked on G-11 |
| **Capacity analysis** | Strong (a common failure in the literature) | `capacity_report(experiment)`: expectancy vs size under square-root impact and spread | New |

## 2. Labels, sampling and sizing

| Technique | Evidence | Tool / capability |
|---|---|---|
| **Triple-barrier labels + meta-labeling** (primary rule gives side; secondary model filters/sizes) | Moderate: practitioner studies show large precision gains ([Hudson & Thames](https://hudsonthames.org/does-meta-labeling-add-to-signal-efficacy-triple-barrier-method/)); a crypto study combining information-driven bars, triple-barrier labels and deep learning ([Financial Innovation 2025](https://link.springer.com/article/10.1186/s40854-025-00866-w)); Korean equities with tuned barriers ([2504.02249](https://arxiv.org/html/2504.02249v2)), where **LSTM ≈ XGBoost** | `build_meta_labeler(strategy, experiment)` (03 §G); `DatasetSpec.target = triple_barrier/meta_label` (08 §7.1). Fix: barriers should use intrabar high/low, not close only |
| **Information-driven bars** (volume/dollar/tick) + **CUSUM event sampling** | Moderate | `tbot.data.bars(kind=dollar)`; `DatasetSpec.sampling = cusum` |
| **Volatility-scaled positions** | Strong for trend/TS-momentum (scaling is where much of the risk-adjusted benefit comes from) | Sizing block in SLv2 (`vol_target`); position-series runs |
| **Ensemble across lookbacks** instead of picking one | Moderate: a Donchian-ensemble trend model across lookbacks with volatility sizing on a survivorship-free crypto universe ([Zarattini, Pagani, Barbon, SSRN 5209907](https://papers.ssrn.com/sol3/Delivery.cfm/5209907.pdf?abstractid=5209907&mirid=1)) | A `lookback_ensemble(signal, [10,20,…])` operator in SLv2. Fits INV-2's plateau philosophy: average the plateau rather than choose a point |
| **Cost-embedded objectives** (optimise positions with costs in the loss, not afterwards) | Moderate–Strong ([2609.04917](https://arxiv.org/abs/2609.04917)) | `Objective` gains net-of-cost defaults; `DatasetSpec` allows decision-aware targets; turnover penalty in sweeps |

## 3. What works in crypto trend (the archetype the platform can already express)

| Finding | Evidence | Use |
|---|---|---|
| Time-series momentum from EMA signals with volatility normalisation beat cross-sectional momentum on 8 major coins, Jan 2020–Oct 2025 | Moderate ([VU journal](https://www.journals.vu.lt/BATP/en/article/download/44540/42590/138419)) | Seeds the first archetype family; the right null is "GARCH clustering, zero drift" |
| Adaptive trend-following over 150+ pairs reports Sharpe 2.41 for 2022–2024 | **Weak** (one window; treat as optimistic) ([2602.11708](https://arxiv.org/html/2602.11708v1)) | Hypothesis source only |
| Technical rules on BTC/ETH fail after data-snooping correction | Moderate, **Negative** (§1) | Why the family test and random arm matter |

**Implication:** the first high-value agent campaign is a *properly sized, lookback-ensembled*
trend family with costs, tested against random search and SPA. It's expressible as soon as
sizing (G-05) and Layer 1 position series land.

## 4. Volatility and forecasting models

| Finding | Evidence | Tool / capability |
|---|---|---|
| Zero-shot time-series foundation models (TSFMs) **don't uniformly beat Log-HAR** for realised vol; only Tiny Time Mixers edged Log-HAR at all horizons; an **equal-weight TTM + Log-HAR** combination was most consistent across 50 assets; much of the short-horizon "gain" was scaling, removed by Mincer–Zarnowitz recalibration | Moderate ([2607.05291](https://arxiv.org/abs/2607.05291)) | `volatility_report` uses HAR/Log-HAR as the primary baseline; add a **forecast-combination** model family and an MZ-recalibration step to `evaluate_model` |
| TSFMs are **weak at daily excess returns** zero-shot (Chronos-large OOS R² ≈ −1.37%, TimesFM ≈ −2.80%); GBMs (CatBoost/LightGBM) stronger | Moderate ([2606.27100](https://arxiv.org/abs/2606.27100)); FinVerse benchmark ranks vary by model ([2608.03259](https://arxiv.org/html/2608.03259v2)) | TSFMs as **baselines only**, with a contamination label; GBM-on-lagged-features stays the hard baseline |
| Architecture choice matters more than "foundation vs econometric" | Moderate ([2607.05291](https://arxiv.org/abs/2607.05291)) | Model tree search (09 §2.1) over families, not a single bet |

## 5. Uncertainty that survives drift

| Technique | Evidence | Tool / capability |
|---|---|---|
| **Adaptive conformal inference** (ACI) and variants for time series | Strong method ([Zaffran et al. ICML 2022](https://proceedings.mlr.press/v162/zaffran22a/zaffran22a.pdf)) | Online recalibration layer on `InferenceOutput` quantiles |
| **ACI with CUSUM change-point resets** of the calibration window | Moderate ([SSRN 5946791](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=5946791)) | Reset on detected breaks; hook into `quality_monitor.rs` |
| **Temporal conformal prediction** for financial risk intervals; **drift-aware spectral CP** reached ~0.90 coverage vs rolling CP/ACI on S&P 500 absolute returns | Weak–Moderate ([TCP 2507.05470](https://arxiv.org/html/2507.05470v1), [DASC 2606.15953](https://arxiv.org/pdf/2606.15953)) | Candidates to benchmark against ACI in the model eval suite |

**Tool:** `calibrate_intervals(model, method=aci|aci_cusum|tcp)` returns a wrapper model
version with coverage-through-time plots. Sizing nodes consume the calibrated `spread_90`.

## 6. Regimes

| Technique | Evidence | Tool / capability |
|---|---|---|
| **Statistical jump models** (jump penalty → persistent regimes), **continuous JM** (probabilistic, smooth transitions), **sparse JM** (feature selection) | Moderate: regime-aware allocation and downside-risk reduction studies ([2402.05272](https://arxiv.org/html/2402.05272v1), [Ann. Oper. Res.](https://link.springer.com/article/10.1007/s10479-024-06035-z), [J. Asset Mgmt](https://link.springer.com/article/10.1057/s41260-024-00376-x)); Python `jump-models` library ([GitHub](https://github.com/Yizhan-Oliver-Shu/jump-models)) | `fit_regime_model(instr, features, kind=jm\|cjm\|sjm\|hmm)` → labels (filtered only) → `regime_report`, `RegimeConditional` studies, SLv2 `regime()` function. Matches FEAT-003 §10 |

## 7. Features and representations

| Technique | Evidence | Tool / capability |
|---|---|---|
| **Path signatures**: universal nonlinear path features; the 2nd-order signed area between two paths encodes **lead–lag**; signature kernels handle irregular sampling | Moderate method, mixed trading evidence ([overview](https://www.emergentmind.com/topics/path-signatures), [2506.01815](https://arxiv.org/pdf/2506.01815)) | Feature-engine family `sig(level, window, transform=lead_lag)`; must pass ablation vs plain lags |
| **Signature-kernel metrics** for tail/probabilistic forecast evaluation | Weak ([2602.10182](https://arxiv.org/pdf/2602.10182)) | Candidate for scoring scenario generators (§8) |
| **Fractional differencing, range volatility estimators, Hurst/VR** | Strong (standard) | See 06; feature engine families |

## 8. Synthetic markets

| Technique | Evidence | Tool / capability |
|---|---|---|
| **Diffusion-model market generators**: CoFinDiff (controllable trend/volatility conditions; reproduces stylized facts; IJCAI 2025), wavelet-DDPM multi-series generation | Moderate ([CoFinDiff](https://www.ijcai.org/proceedings/2025/1040), [2410.18897](https://arxiv.org/abs/2410.18897)) | `simulate_paths(kind=diffusion, conditions={trend, vol})` for (a) stress scenarios, (b) eval-suite planted-edge/noise instruments with realistic texture, (c) `SyntheticPaths` studies. Must pass a stylized-facts check before use; keep parametric nulls (GARCH/jump) as the interpretable default |

## 9. Text and LLM-derived signals

| Technique | Evidence | Tool / capability |
|---|---|---|
| **Look-ahead contamination is measurable**: "Lookahead Propensity" (date-only recall query) is positive in-sample and collapses to ~0 after the model's training cutoff; forecast skill is amplified on high-LAP pairs | Moderate ([2512.23847](https://arxiv.org/abs/2512.23847)) | `contamination_probe(model, entity, date)`; label every text feature with model + cutoff; prefer post-cutoff evaluation |
| **MemGuard-Alpha**: filter memorisation-contaminated signals via membership inference + cross-model disagreement | Weak ([2603.26797](https://arxiv.org/pdf/2603.26797)) | Optional filter on text-signal series |
| **FinCAD**: context-aware decoding that attenuates memorised outcomes without retraining; improved in-sample→OOS rank correlation (0.779 → 0.846) across 11 models | Weak ([2605.24564](https://arxiv.org/pdf/2605.24564)) | Research option if LLM text signals become central |
| LLM-extracted news signals show some incremental value | Moderate ([2609.04917](https://arxiv.org/abs/2609.04917)) | Quarantined reader extraction (09 §6) → PIT text features |

## 10. Factor discovery (when multi-instrument universes land)

| Technique | Evidence | Adopt |
|---|---|---|
| Operator DSL + AST sandbox + family-aware selection (Hubble) | Weak–Moderate ([2604.09601](https://arxiv.org/abs/2604.09601)) | Reuse SLv2 expressions as the factor DSL |
| Regularised exploration: complexity, originality (AST similarity), hypothesis alignment (AlphaAgent) | Moderate ([2502.16789](https://arxiv.org/abs/2502.16789)) | Proposal scoring (09 §4.4) |
| Low-redundancy libraries under the "correlation red sea" (FactorMiner); process memory (AlphaMemo); evolutionary mining (QuantaAlpha) | Weak–Moderate ([2602.14670](https://arxiv.org/abs/2602.14670), [2606.20625](https://arxiv.org/html/2606.20625v1), [2602.07085](https://arxiv.org/html/2602.07085)) | `factor_library` with correlation de-dup; memory per 09 §3 |
| Reproducibility audits find LLM-trading papers report architecture more clearly than execution/cost/PIT assumptions | Moderate ([2606.08285](https://arxiv.org/abs/2606.08285)) | Every agent report includes the execution-assumptions block automatically |

---

## 11. New tools from this doc (additions to 03)

| Tool | Group | Priority |
|---|---|---|
| `alpha_chain_audit(experiment_id)` | evaluation | High |
| `compare_to_random(campaign_id)` + shadow random arm | evaluation / campaign | High |
| `family_test(experiment_ids, method)` (SPA, step-SPA, Romano–Wolf) | evaluation | High |
| `capacity_report(experiment_id)` | evaluation | High |
| `calibrate_intervals(model, method)` | models | Medium |
| `fit_regime_model(…, kind=jm\|cjm\|sjm\|hmm)` | analysis | Medium (FEAT-003 Ph3) |
| `lookback_ensemble` operator, `vol_target` sizing | SLv2 / feature engine | High (with G-05) |
| `simulate_paths(kind=diffusion)` | analysis | Low–Medium |
| `contamination_probe(model, entity, date)` | text | Medium (before any text strategy) |
| Forecast-combination family + MZ recalibration in `evaluate_model` | models | Medium |

**Refined by 11 (2026-09-10):** volatility defaults move to **HARQ** (not plain HAR);
spreads use the **EDGE** estimator (not Roll/Corwin–Schultz); `capacity_report` uses
√-impact confirmed on BTC; signal/model combination defaults to **equal weights**
(forecast combination puzzle); complex models must beat a **vol-timed momentum**
baseline (virtue-of-complexity critiques). New tools from 11: `posterior_alpha`,
`collider_check`, `decompose_strategy_returns`, `factor_attribution`, `covariance(nls)`,
`ofi_features`, `basis_report`, `expected_decay`, `data_qc`, `universe(as_of)`.
