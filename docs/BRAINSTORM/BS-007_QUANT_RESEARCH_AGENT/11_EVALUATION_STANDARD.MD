# BS-007 / 11 — Evaluation and the quant research standard

**Spec status:** spec-ready draft · **Req prefix:** EV- · **Formal spec:**
BACKTEST_SUITE_CORE_SPEC v2 (extends Set J) · **Plan set:** Q · **Decisions:** D-10, D-13 ·
**Keeps:** INV-1/2/3, the Null Library, the Gate 0→4 funnel, `SelectionRule`, DSR/PBO,
`crates/research` sweeps (01 §7)

A real quant is judged less on finding strategies than on not fooling themselves. This
spec is the methodology the agent must follow and the platform must enforce. It
distills Arnott–Harvey–Markowitz and 20 topics of high-quality quant literature; the
sources are listed in [research/R2_QUANT_TECHNIQUES.MD](research/R2_QUANT_TECHNIQUES.MD)
and at the end of this doc. Every rule is **(E) platform-enforced** or **(A)
agent-required** and checked by the `final_report` validator.

---

## 1. Set J adaptations for a sandboxed agent

| Change | Detail |
|---|---|
| Holdout enforcement moves to the Data API | Project research cutoff (06 §3). The vault gate is the only reader past it |
| Run kinds | `definition` (SLv2) and `position_series` (Layer 1), both counted identically |
| Model trials | HPO and training trials count on a model trial counter. A model's Experiment links to the strategy Experiments that use its prediction series |
| Persistence and parallelism | Suite, sweep and funnel state moves into the job service (05). The funnel stops running under the suite lock |
| Gate 0 additions | Truncation test (08 §3.3); prediction-series training-window overlap (10 §5); `data_qc` grade (06 §5); an auditor suite of planted violations run in CI (16 §3) |
| Forward stage (after G-11) | A paper-trading gate whose pass/fail is backtest-consistency of paper fills |

## 2. Hypothesis registration (pre-registration)

`POST /api/hypotheses` (`tbot exp hypothesis register`) records:
- `archetype`;
- `statement`;
- **`mechanism`, `counterparty`, `why_persists`** ("who pays you, and why does it
  persist?");
- **`falsifier`** and `kill_criterion`;
- `null` (a Null Library choice with its preserve/destroy rationale);
- `alternatives_considered`;
- optionally `causal_graph` (DAG over the signal, conditioning filters and returns);
- `source` (`original | literature:<ref> | memory:<fnd_…>`).

It is timestamped and immutable. Edits create a new version.

**(E):**
- `create_experiment` refuses without a registered hypothesis;
- the window is fixed on the Experiment, and a changed window is a new, counted
  Experiment.

## 3. Counting: trials, effective N, specifications, exploration

| Counter | Definition | Used by |
|---|---|---|
| Trials (INV-1) | Every evaluation-counted job (05 §2) | DSR, reports |
| **Effective N** | Trials clustered by return correlation (hierarchical clustering on run return series; N_eff = number of clusters at a declared threshold, or the eigenvalue-based estimate) | Feeds DSR, so near-duplicate sweeps neither make significance unreachable nor make it cheap |
| **Specification count** | Features, interactions, transforms and filters added across the Experiment's history (20 variables → 190 pairwise interactions) | Reported; feeds the protocol checklist |
| Alternatives considered | From the hypothesis record | Counts toward effective N |
| **Exploration ledger** (D-13) | Every Data API read and analysis job in the project (05 §7) | Reported with every verdict; not counted as trials |
| **`post_hoc` flag** | Set when a hypothesis is registered after exploration on the same instrument and window touching the variables it uses | Shown on the verdict and dossier |

## 4. The research protocol, made mechanical (Arnott–Harvey–Markowitz, 2019)

| # | Rule | Enforcement |
|---|---|---|
| 1a | Ex-ante economic foundation | (E) §2 required fields |
| 1b | Beware ex-post stories | (E) `post_hoc` flag from the exploration ledger |
| 2a | Track everything tried and the correlation between tries | (E) INV-1 plus effective N |
| 2b | Count interaction combinations | (E) specification count |
| 2c | Parallel-universe problem (choosing among models considered) | (A) `alternatives_considered`, counted in effective N |
| 3a | Fix the sample ex ante | (E) immutable Experiment window |
| 3b | Data quality first | (E) `data_qc` grade gate (06 §5) |
| 3c | Decide transformations in advance; results must survive small changes | (E) declared in the DatasetSpec or strategy params; robustness suite perturbs them |
| 3d–e | No arbitrary outlier exclusion; winsorisation level fixed before modelling | (E) part of the declared spec; a change is a new specification |
| 4a–b | Out of sample isn't really out of sample; iterated OOS is in-sample | (E) holdout readable only through the vault, once |
| 4c | Don't ignore trading costs | (E) costs on by default from dated cost models (09 §5) |
| 5a | Structural change | (A) regime-conditional study before G3; (E) sub-period stability in the robustness suite |
| 5b | Crowding and decay | (A) expected decay (T18) and capacity (T11) in the dossier |
| 5c | Don't tweak a live model | (E) deployed versions immutable; changes restart the funnel |
| 6a–c | Dimensionality; simplicity; interpretability | (E) `simplify` move before G3; complexity penalty in proposal scoring; (A) explain drivers for model-based candidates |
| 7 | Reward quality, not discoveries | (E) eval suite scores honest nulls on noise instruments as successes (16) |

## 5. The candidate dossier (Gate 3 → vault entry requirement)

A `dossier` artifact with 12 parts. The vault gate and the `final_report` validator
refuse a candidate without it:
1. Hypothesis card (§2), with registration time and the `post_hoc` flag.
2. `data_qc` grade, any waiver, and the as-of universe or instrument-selection note.
3. Specification count, trial count, effective N, exploration-ledger count.
4. Walk-forward result and CPCV path distribution (median, worst-5%, share of paths
   positive).
5. Primary null p-value, **DSR**, **PBO**, and **theme-posterior alpha** (T2).
6. Comparison with a matched-budget random search (§8).
7. Costs (EDGE-based), net-of-cost metrics, **capacity** (T11), turnover.
8. Sized vs unsized at matched risk, and left-tail metrics (T8).
9. Crypto market-factor beta and return decomposition (T5, T14).
10. Regime-conditional and sub-period stability; robustness to declared transformations.
11. Protocol checklist (§4), pass or flag per item, and the alpha-translation-chain audit
    (§8).
12. Expected live haircut (T18) and the forward-test plan.

## 6. Significance and acceptance thresholds (G3)

A candidate passes G3 only if all of the following hold:
- the primary null p-value is significant after selection-bias correction by the live
  trial counter (exists);
- **DSR ≥ 0.95** computed with effective N;
- **PBO ≤ 0.5**;
- **the theme-posterior alpha is positive with a credible interval excluding 0**;
- if part of a campaign, the candidate survives the family test (step-SPA or
  Romano–Wolf) over the campaign's candidates.

## 7. QC rules by topic (competency map T1–T20)

Strength: Strong / Moderate / Weak / Contested (evidence in research/R2 and the sources
below).

| T | Topic | Method (strength) | Capability | QC rule |
|---|---|---|---|---|
| T1 | Backtest protocol | AHM checklist (Strong) | §4 | Dossier item 11 |
| T2 | Multiple testing across correlated factors | Empirical-Bayes theme shrinkage, Jensen–Kelly–Pedersen (Strong) | `posterior_alpha` | DSR **and** posterior (§6) |
| T3 | Causal validity of filters | Causal factor investing; collider bias (Moderate) | `collider_check`; optional DAG | Filters added by moves state their causal role; flagged colliders need justification |
| T4 | OOS predictability | Goyal–Welch–Zafirov (Strong) | OOS R², Clark–West, DM (10 §6) | No forecast feature without OOS R² > 0 with CW significance |
| T5 | Model complexity | Virtue of complexity **Contested** (Nagel; Buncic) | `decompose_strategy_returns`; always an intercept | Complex models must beat vol-timed momentum |
| T6 | Combining signals | Forecast combination puzzle (Strong) | EW default (08 §4.2) | Estimated weights must beat EW OOS |
| T7 | Volatility forecasting | HAR → HARQ (Strong) | 07 §4 RQ; 10 §3 | Vol models must beat HARQ on QLIKE |
| T8 | Volatility targeting | Harvey et al. (Strong for risk assets; tail reduction everywhere) | `vol_target` sizing | Compare at matched risk; report left-tail metrics |
| T9 | Covariance | Ledoit–Wolf nonlinear shrinkage (Strong) | `covariance(method=nls)` | NLS default for multi-asset |
| T10 | Spreads from bars | EDGE estimator (Strong) | Cost models (09 §5) | Costs from dated per-instrument models |
| T11 | Impact and capacity | Square-root law, confirmed on BTC (Strong) | `capacity_report` | Report size at which net expectancy → 0 |
| T12 | Order flow | Integrated multi-level OFI (Strong) | `ofi_features` (when L2 history exists) | Intraday liquid strategies test OFI before price-only features |
| T13 | Trend following | Century of evidence; crypto decade (Strong / Moderate) | First archetype family | Null = GARCH zero drift; must beat random search |
| T14 | Crypto factor structure | Liu–Tsyvinski–Wu market/size/momentum (Strong) | `factor_attribution` | Report crypto-market beta |
| T15 | Carry / funding | Funding dislocations; only ~40% profitable after costs (Moderate) | `basis_report` (when a funding lane exists) | Never gross-of-cost |
| T16 | Volatility risk premium | BTC IV > RV on average (Weak) | Options archetype (forward snapshots, 06 §6) | Short-vol results show ES and crash behaviour |
| T17 | Statistical arbitrage | Pelger three-step decomposition (Moderate) | Multi-asset phase | Each step validated; residuals PIT |
| T18 | Alpha decay | McLean–Pontiff: −26% OOS, −58% post-publication (Strong) | `expected_decay` | Reports state an expected live haircut; literature ideas start with a decay prior |
| T19 | Survivorship and selection | Crypto dead coins; delisting bias (Weak–Moderate) | `universe(as_of)`, `data_qc` flag | As-of membership required; instrument-selection bias acknowledged |
| T20 | Risk-model validation | Acerbi–Szekely ES backtests (Strong) | 10 §6 | ES backtests before sizing on ES/VaR |

## 8. Evaluation tools (platform services; exposed through 14)

| Tool | Returns | Priority |
|---|---|---|
| `alpha_chain_audit(experiment)` | Five-stage pass/fail/unknown (PIT information → stable signal → feasible positions → executable orders → net alpha after costs, with capacity) | High |
| `compare_to_random(campaign)` plus a shadow random arm | Agent vs matched-budget random search on the sealed metric | High |
| `family_test(experiment_ids, method=spa\|stepwise_spa\|romano_wolf)` | Data-snooping-robust test over a candidate family | High |
| `capacity_report(experiment)` | Net expectancy vs order size under √-impact and EDGE spread; break-even size | High |
| `posterior_alpha(experiment)` | Theme-posterior (shrinkage toward the archetype prior from research memory) with a credible interval | High |
| `decompose_strategy_returns(experiment)` | Vol-timing, directional-drift and selection components | Medium |
| `factor_attribution(experiment, factors)` | Crypto market/size/momentum betas and alpha | Medium |
| `collider_check(strategy_ref)` | Filters plausibly caused by both signal and future returns | Medium |
| `expected_decay(archetype, source)` | Haircut prior for the dossier | Medium |
| `run_robustness_suite(experiment, preset)` | Cost ladder, delay-by-one-bar, sub-periods, neighbourhood, transformation perturbations, regime-conditional | High |
| `compare_candidates(a, b)` | Sealed-distribution comparison (worst-5%, median, overlap); never point estimates | High |
| `benchmark_run(experiment, kind)` | Buy-and-hold, random-entry-matched and simple-momentum baselines on the same slice | High |

## 9. Requirements

| ID | Requirement | Pri |
|---|---|---|
| EV-01 | INV-1/2/3, the Null Library, the funnel and `SelectionRule` MUST be kept unchanged in semantics | MUST |
| EV-02 | Hypothesis registration (§2) MUST be required by `create_experiment` | MUST |
| EV-03 | Effective N MUST be computed and used in DSR; specification count, exploration ledger and `post_hoc` MUST be reported | MUST |
| EV-04 | Both run kinds and model trials MUST be counted | MUST |
| EV-05 | Gate 0 MUST include the §1 additions, and the auditor MUST pass the planted-violation CI suite | MUST |
| EV-06 | The dossier (§5) MUST be required at the G3 → vault transition and by the report validator | MUST |
| EV-07 | G3 MUST apply the §6 thresholds, including the theme posterior, and the family test for campaigns | MUST |
| EV-08 | §8 High-priority tools MUST exist; Medium SHOULD | MUST/SHOULD |
| EV-09 | Every campaign MUST run a shadow random-search arm with a matched budget, reported alongside the agent's results | MUST |
| EV-10 | Suite, sweep and funnel state MUST be persisted in the job service | MUST |
| EV-11 | The forward paper gate MUST be added once G-11 is wired | MUST (Track) |

## 10. Acceptance criteria

1. A candidate without a dossier can't enter the vault, and the error lists the missing
   parts.
2. On the pure-noise suite, G3 pass rate ≤ nominal + 2 pp, with effective N active.
3. Twenty near-duplicate sweep members count as far fewer than 20 effective trials, and
   DSR reflects it.
4. A hypothesis registered after exploration on the same variables is flagged
   `post_hoc`.
5. A campaign report shows the agent vs random-arm comparison and the family-test
   result.

## Sources

- **Protocol:** Arnott, Harvey, Markowitz — [A Backtesting Protocol in the Era of Machine
  Learning](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=3275654)
  ([PDF](https://people.duke.edu/~charvey/Research/Published_Papers/SSRN-id3275654.pdf))
- **Multiple testing:** Jensen, Kelly, Pedersen — [Is There a Replication Crisis in
  Finance?](https://onlinelibrary.wiley.com/doi/full/10.1111/jofi.13249)
- **Causal validity:**
  - López de Prado et al. — [Why Has Factor Investing Failed?](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4697929)
  - [Causality and Factor Investing](https://rpc.cfainstitute.org/research/foundation/2025/causality-factor-investing)
- **OOS predictability:** Goyal, Welch, Zafirov — [RFS 2024](https://academic.oup.com/rfs/article/37/11/3490/7749383)
- **Model complexity:**
  - Kelly, Malamud, Zhou — [Virtue of Complexity](https://onlinelibrary.wiley.com/doi/full/10.1111/jofi.13298)
  - Nagel — [Seemingly Virtuous Complexity](https://voices.uchicago.edu/stefannagel/files/2025/07/Complexity_2.pdf)
  - [Buncic](https://papers.ssrn.com/sol3/Delivery.cfm/5239006.pdf?abstractid=5239006&mirid=1)
- **Forecast combination:** [Forecast combination puzzle](https://www.sciencedirect.com/science/article/abs/pii/S0169207016000327)
- **Volatility forecasting:** Bollerslev, Patton, Quaedvlieg — [HARQ](https://www.sciencedirect.com/science/article/abs/pii/S0304407615002584)
- **Volatility targeting:** Harvey et al. — [The Impact of Volatility Targeting](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=3175538)
- **Covariance:** Ledoit, Wolf — [Analytical nonlinear shrinkage](https://www.researchgate.net/publication/345182724_Analytical_nonlinear_shrinkage_of_large-dimensional_covariance_matrices)
- **Spreads:** Ardia, Guidotti, Kroencke — [EDGE](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=3892335)
  ([bidask](https://github.com/eguidotti/bidask))
- **Impact:**
  - [Square-root impact, Tokyo SE](https://arxiv.org/html/2411.13965v3)
  - [Bitcoin metaorders](https://www.worldscientific.com/doi/10.1142/S2382626615500082)
- **Order flow:** Cont, Cucuringu, Zhang — [Cross-Impact of OFI](https://arxiv.org/abs/2112.13213)
- **Trend following:**
  - Hurst, Ooi, Pedersen — [A Century of Trend Following](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=2993026)
  - [Trend following in crypto](https://arxiv.org/pdf/2009.12155)
- **Crypto factors:** Liu, Tsyvinski, Wu — [Common Risk Factors in Cryptocurrency](https://onlinelibrary.wiley.com/doi/abs/10.1111/jofi.13119)
- **Carry / funding:** [Funding rate arbitrage](https://www.sciencedirect.com/science/article/pii/S2096720925000818)
- **Volatility risk premium:** [Bitcoin VIX VRP](https://www.pm-research.com/content/iijaltinv/23/4/84)
- **Statistical arbitrage:** Guijarro-Ordonez, Pelger, Zanotti — [Deep Learning Statistical Arbitrage](https://arxiv.org/abs/2106.04028)
- **Alpha decay:** McLean, Pontiff — [Academic Research and Predictability](https://onlinelibrary.wiley.com/doi/abs/10.1111/jofi.12365)
- **Survivorship:** [Crypto survivorship and delisting bias](https://www.alexandria.unisg.ch/bitstreams/2bc8397d-47dd-4f66-8467-9004b2c9d212/download)
- **Risk-model validation:** Acerbi, Szekely — [Backtestable statistics](https://doi.org/10.2139/ssrn.2905109)
- **Additional sources:**
  - [ADIA Lab causal factor protocol](https://www.adialab.ae/research-series/a-protocol-for-causal-factor-investing)
  - [Forecast combinations: a 50-year review](https://arxiv.org/pdf/2205.04216)
  - Ledoit–Wolf, [The Power of (Non-)Linear Shrinking](https://dx.doi.org/10.2139/ssrn.3384500)
  - [Market impact survey](https://arxiv.org/pdf/2205.07385)
  - [Two-tiered funding-rate markets](https://www.mdpi.com/2227-7390/14/2/346)
  - [Fundamentals of Perpetual Futures](https://arxiv.org/pdf/2212.06888)
  - [Deribit: four years of volatility regimes](https://insights.deribit.com/industry/bitcoin-options-finding-edge-in-four-years-of-volatility-regimes/)
  - [StratBase: dead coins](https://stratbase.ai/en/blog/survivorship-bias-crypto)
  - [MSCI: Backtesting Expected Shortfall](https://msci.com/www/research-report/research-insight-backtesting/0128184734)
- **Alpha-translation chain and agent-vs-random:**
  - [arXiv 2609.04917](https://arxiv.org/abs/2609.04917)
  - [arXiv 2608.25348](https://arxiv.org/abs/2608.25348)
