# BS-007 / 11 — The agent's quant research standard (refined research, 2026-09-10)

Status: draft spec. This is the **methodology the agent must follow and the platform must
enforce**, distilled from a focused pass over high-quality quant literature (20 topics,
sources at the end). It refines 06 (knowledge) and 10 (tools). Where they differ, this
doc wins.

**Principle:** a real quant is judged less on finding strategies than on not
fooling themselves. Every rule below is either **(E) enforced by the platform** or
**(A) required of the agent** and checked by the `final_report` validator.

---

## 1. The research protocol, made mechanical

Built on the seven categories of Arnott, Harvey & Markowitz, "A Backtesting Protocol in
the Era of Machine Learning" (2019), mapped to platform mechanisms.

| # | Protocol rule | How it's enforced here |
|---|---|---|
| 1a | **Ex-ante economic foundation.** A hypothesis needs a logical mechanism *before* testing ("who pays you, and why does it persist?") | (E) `record_hypothesis` requires `mechanism`, `counterparty`, `why_persists`, `falsifier`; `create_experiment` refuses without one |
| 1b | **Beware ex-post stories** made up after data mining | (E) The exploration ledger timestamps every data look; a hypothesis registered *after* exploratory analyses on the same instrument/window is flagged `post_hoc` in its verdict |
| 2a | **Track everything tried, and the correlation between tries** (20 correlated tests ≈ 1 test; 20 uncorrelated need a much higher bar) | (E) INV-1 trial counter + **correlation-aware effective N** (cluster trials by return correlation) feeding DSR |
| 2b | **Count interaction combinations**: 20 variables means 190 possible pairwise interactions, not 22 | (E) `DatasetSpec` / SLv2 feature additions increment an Experiment's *specification count*; reports show it |
| 2c | **The parallel-universe problem**: choosing one model from the same history after considering others is still multiple testing | (A) The agent lists alternatives it considered in `record_hypothesis.alternatives`; they count toward effective N |
| 3a | **Fix the sample ex ante**; never move the start date or drop crises because they hurt | (E) The window is immutable on the Experiment; a changed window is a new Experiment and counted |
| 3b | **Ensure data quality first** | (E) `data_qc` runs before any Experiment on an instrument (§3) |
| 3c | **Decide transformations in advance** (vol scaling, standardisation); results must survive small changes to them | (E) Transformations are declared in `DatasetSpec`/strategy params; `run_robustness_suite` perturbs them |
| 3d–e | **No arbitrary outlier exclusion; choose winsorisation level before modelling** | (E) Winsorisation is part of the declared spec; changing it is a new specification |
| 4a–b | **Out of sample is not really out of sample; iterated OOS is in-sample** | (E) Holdout readable only through the vault gate; once read, the Experiment can't be iterated (Set J vault + Data API cutoff) |
| 4c | **Don't ignore trading costs** | (E) Costs on by default (`UnsafeFlags.costs_disabled` marks runs unsafe); the cost model is data-driven (§2 T10) |
| 5a | **Structural change**: be aware of regime shifts | (A) Regime-conditional study before Gate 3; (E) sub-period stability in the robustness suite |
| 5b | **Crowding / "Heisenberg"**: trading the effect erodes it | (A) Report expected decay (§2 T18) and capacity (§2 T11) |
| 5c | **Don't tweak a live model** | (E) Deployed strategies are immutable versions; changes restart the funnel |
| 6a–c | **Curse of dimensionality; simplicity and regularisation; interpretability** | (E) `simplify` move before Gate 3 (FEAT-003); complexity penalty in proposal scoring; (A) explain drivers for any model-based candidate |
| 7 | **Reward quality, not "discoveries"** | (E) The agent eval suite scores honest nulls on noise instruments as successes; reports that find nothing are first-class outcomes |

---

## 2. Competency map: 20 topics a senior quant masters

Each row: what the literature supports → the agent's tool or capability → the QC rule.
**Strength:** Strong / Moderate / Weak / Contested.

### Inference and research QC

| T | Topic | Best-supported method | Tool / capability | QC rule |
|---|---|---|---|---|
| T1 | Backtest protocol | Arnott–Harvey–Markowitz 7-category checklist (§1) | Enforced across the tool layer | Report includes the checklist with pass/flag per item |
| T2 | Multiple testing with many correlated factors | **Empirical-Bayes / hierarchical shrinkage**: Jensen–Kelly–Pedersen (JF 2023) find most factors replicate once modelled jointly; clustered into 13 themes; *more related evidence strengthens a theme* (Strong) | `posterior_alpha(experiment)`: shrink the candidate's performance toward its **theme/archetype prior** built from research memory; complements DSR (penalises search) with a posterior (pools related evidence) | Report both DSR and the posterior; a candidate needs DSR ≥ 0.95 **and** a positive posterior with a credible interval excluding 0 |
| T3 | Causal validity of conditioning variables | **Causal factor investing** (López de Prado et al.): confounder and **collider** biases make "significant" specifications wrong-signed out of sample (Moderate, growing) | `record_hypothesis.causal_graph` (optional DAG); `collider_check`: flags filters on variables plausibly *caused by* both signal and future returns (e.g., same-bar volume responding to the move, "was a winning trade" features) | Any filter added by a move must state its causal role; flagged colliders need justification |
| T4 | Is predictability real out of sample? | Goyal–Welch–Zafirov (RFS 2024): most of the published equity-premium predictors have negative or barely positive OOS R² vs the prevailing mean (Strong) | `evaluate_forecast` adds **Campbell–Thompson OOS R² vs historical mean**, **Clark–West** (nested models), Diebold–Mariano, and utility gain of a mean-variance investor | No forecast feature enters a strategy unless OOS R² > 0 with CW significance on walk-forward predictions |
| T5 | Model complexity | "Virtue of complexity" (Kelly–Malamud–Zhou, JF 2024) is **Contested**: Nagel (2025) shows small-sample random-feature forecasts mimic **volatility-timed momentum**; Buncic (2025) traces results to a zero-intercept restriction and an unusual aggregation | `decompose_strategy_returns`: split P&L into volatility-timing, directional-drift and selection components; always include an intercept; report effective parameters/sample ratio | A complex model must beat a **vol-timed momentum baseline**, not just buy-and-hold |
| T6 | Combining signals/models | **Forecast combination puzzle**: equal weights usually beat estimated "optimal" weights because weight-estimation error dominates (Strong) | SLv2 `Combine` and ensemble families default to equal weights; estimated weights need their own validation | Estimated weights must beat EW out of sample, or EW is used |

### Volatility, sizing and risk

| T | Topic | Best-supported method | Tool / capability | QC rule |
|---|---|---|---|---|
| T7 | Volatility forecasting | **HAR → HARQ** (Bollerslev–Patton–Quaedvlieg 2016): realised quarticity corrects attenuation from measurement error, giving more responsive forecasts when RV is precise (Strong); Log-HAR is a strong baseline even against foundation models (10 §4) | `volatility_report` defaults: RV from 5m (24/7 crypto), realised quarticity, HAR/Log-HAR/HARQ, EWMA; QLIKE scoring | Any volatility model must beat HARQ on QLIKE out of sample |
| T8 | Volatility targeting | Harvey et al. (JPM 2018), 60 assets from 1926: raises Sharpe for **risk assets** (equities, credit), negligible for bonds/FX/commodities, but **reduces extreme returns and left-tail severity across all assets** (Strong) | SLv2 `vol_target(σ*, forecaster)`; default sizing candidate in every campaign | Evaluate sized vs unsized **at matched average risk**, and report left-tail metrics (worst 1% bar, ES, max DD), not just Sharpe |
| T9 | Covariance estimation | **Analytical nonlinear shrinkage** (Ledoit–Wolf 2020): closed form, optimal for mean-variance portfolio choice (Strong) | `covariance(instruments, method=nls\|lw_linear\|sample)` | Default NLS for any multi-asset covariance (portfolio, signal combination, risk) |
| T20 | Risk-model validation | **Expected shortfall backtests** (Acerbi–Szekely): non-parametric ES tests more powerful than Basel VaR tests; elicitability is for *ranking* models (use joint VaR–ES scoring such as Fissler–Ziegel loss), not for testing (Strong) | `scoring.py` gains Acerbi–Szekely Z-tests and joint VaR–ES loss alongside Kupiec/Christoffersen (exist) | Any model serving `var_95`/`es_95` must pass ES backtests before a strategy may size on it |

### Costs, execution and microstructure

| T | Topic | Best-supported method | Tool / capability | QC rule |
|---|---|---|---|---|
| T10 | Spread estimation from bars | **EDGE estimator** (Ardia–Guidotti–Kroencke, JFE 2024): asymptotically unbiased, minimum-variance use of OHLC; the most accurate spread estimate without quotes (Strong); reference implementation `bidask` (R, Python port exists per authors' site) | `liquidity_cost_report` uses EDGE (rolling), with Roll/Corwin–Schultz as cross-checks; output becomes the instrument's `cost_model_ref` | Costs in every Run come from a dated, per-instrument cost model (fixes G-06) |
| T11 | Market impact and capacity | **Square-root law** $\Delta p \approx Y\sigma\sqrt{Q/V}$: confirmed across markets, including >1M reconstructed BTC/USD metaorders, and strongly universal on the Tokyo SE (Strong) | `capacity_report`: expectancy vs order size under √-impact + EDGE spread; break-even size | Report capacity at which net expectancy → 0; flag if below a meaningful size |
| T12 | Order flow | **Integrated multi-level OFI** (Cont–Cucuringu–Zhang, QF 2023): PCA-combined OFI across book levels explains impact better than best-level OFI; cross-impact adds little once integrated (Strong) | `ofi_features(instr, levels)` when L2 history exists (collector lane exists; history doesn't yet) | Intraday strategies on liquid pairs should test OFI before price-only features |

### Strategy families and their evidence

| T | Topic | Best-supported method | Tool / capability | QC rule |
|---|---|---|---|---|
| T13 | Trend following | Hurst–Ooi–Pedersen (JPM 2017): time-series momentum positive in **every decade since 1880**, and good in 8 of the 10 largest crises (Strong); crypto-specific: "A Decade of Evidence of Trend Following in Cryptocurrencies" (Moderate) | First archetype family: lookback-ensembled, vol-targeted trend (10 §3) | Null = GARCH clustering with zero drift; must beat random search (09 §2.4) |
| T14 | Crypto factor structure | Liu–Tsyvinski–Wu (JF 2022): **crypto market, size and momentum** capture the crypto cross-section; equity factor models don't (Strong) | `factor_attribution(strategy, factors=[crypto_mkt, size, mom])` | Single-asset: report beta to the crypto market factor ("is it just beta?"). Multi-asset (later): evaluate factor-neutral alpha |
| T15 | Carry / funding | Perp funding and basis dislocations are larger than in FX (Moderate), but **after costs only ~40% of top opportunities are profitable** in one CEX/DEX study (Moderate) | Carry archetype once a funding lane exists; `basis_report` | Fees, borrow/margin and spread reversal modelled explicitly; never gross-of-cost |
| T16 | Volatility risk premium | Implied > realised on average for BTC options (Bitcoin VIX VRP study; practitioner Deribit analysis reports ~+15 vol points in contango, **Weak**) | Options archetype once chain snapshots exist (08 §3.2) | Short-vol results must show tail/ES and crash-period behaviour |
| T17 | Statistical arbitrage | **Three-step decomposition** (Guijarro-Ordonez–Pelger–Zanotti): residual portfolios from conditional factors → signal extraction (convolutional transformer) → trading policy optimised for risk-adjusted return under constraints (Moderate) | Template for the multi-asset phase (lifting OQ-040) | Each step validated separately; residuals must be PIT |
| T18 | Alpha decay | McLean–Pontiff (JF 2016), 97 predictors: returns **26% lower out of sample** and **58% lower post-publication**; decay is larger for high in-sample returns and illiquid names (Strong) | `expected_decay(archetype, source)`: haircut expectations for literature-derived ideas; paper-vs-backtest decay monitoring once live is wired | Reports state an expected live haircut; ideas taken from published sources start with a decay prior |

### Data integrity

| T | Topic | Best-supported method | Tool / capability | QC rule |
|---|---|---|---|---|
| T19 | Survivorship and selection | Crypto: most listed tokens eventually die (practitioner counts >50%); a top-20 rebalanced backtest was inflated ~4× by using today's survivors (practitioner, Weak–Moderate); academic treatment of crypto survivorship/delisting bias exists (SSRN/St. Gallen) | `data_qc` + PIT universe membership (`universe(as_of)`), delisted instruments retained; catalog flags survivorship risk per dataset | Universe strategies must use as-of membership. Single-instrument research must acknowledge **instrument selection bias** (we study BTC *because* it survived) |

---

## 3. `data_qc`: run before any Experiment

A deterministic report per (instrument, timeframe, window), cached in the instrument
profile:
- Coverage and gaps (count, longest gap, share of bars missing); revised-bar count (`market.bars.1m.revised` lane).
- Bad-tick screen: returns beyond k·σ that reverse next bar; zero or negative prices; OHLC inconsistency (H < max(O,C), L > min(O,C)).
- Stale-price runs (unchanged close for N bars); volume anomalies (zeros, spikes).
- Timestamp integrity (`available_time` ≥ event time; monotonic; timezone).
- Venue coverage consistency across sources; survivorship flag (§2 T19).
- Output: `qc_grade` A–D with reasons. The Data API attaches it to every Parquet manifest.
  Experiments on grade D data are refused, and grade C requires an explicit waiver
  recorded in the report.

---

## 4. The candidate dossier (what every result must include)

The `final_report` validator refuses a candidate strategy without:
1. Hypothesis card (mechanism, counterparty, persistence, falsifier, pre-registration time, `post_hoc` flag).
2. Data QC grade and the as-of universe/instrument-selection note.
3. Specification count, trial count, effective N (correlation-clustered), exploration-ledger count.
4. Walk-forward result and CPCV path distribution (median, worst-5%, share of paths positive).
5. Primary null p-value, **DSR**, **PBO**, and **theme-posterior alpha** (T2).
6. Random-search comparison at matched budget (09 §2.4).
7. Costs (EDGE-based), net-of-cost metrics, **capacity** (T11), turnover.
8. Sized-vs-unsized comparison at matched risk and left-tail metrics (T8).
9. Crypto market-factor beta and return decomposition (T5, T14).
10. Regime-conditional and sub-period stability; robustness to declared transformations.
11. Protocol checklist (§1) with pass/flag per item.
12. Expected live haircut (T18) and the forward-test plan.

---

## 5. Spec changes this doc makes elsewhere

| Doc | Change |
|---|---|
| 03 toolbox | Add `posterior_alpha`, `collider_check`, `decompose_strategy_returns`, `factor_attribution`, `covariance`, `ofi_features`, `basis_report`, `expected_decay`, `data_qc`, `universe(as_of)`; `evaluate_forecast` gains OOS R²/Clark–West/DM/utility |
| 06 knowledge | Cards T1–T20 added to the L1 library (seed from §2 rows) |
| 08 §8 evaluation | Add theme-posterior alongside DSR; specification counting; `post_hoc` flag; ES backtests for risk outputs; `data_qc` grade gate |
| 10 tools | T7/T10/T11 upgrade the volatility, cost and capacity tools (HARQ, EDGE, √-impact) |
| Agent eval suite (04 §9) | Add cases: collider trap (a filter caused by returns), survivorship trap (universe from today's survivors), post-hoc hypothesis trap, complexity-as-vol-timing trap |

---

## Sources

- Arnott, Harvey, Markowitz — [A Backtesting Protocol in the Era of Machine Learning](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=3275654) ([PDF](https://people.duke.edu/~charvey/Research/Published_Papers/SSRN-id3275654.pdf))
- Jensen, Kelly, Pedersen — [Is There a Replication Crisis in Finance?](https://onlinelibrary.wiley.com/doi/full/10.1111/jofi.13249) ([NBER](https://www.nber.org/papers/w28432))
- López de Prado et al. — [Why Has Factor Investing Failed? The Role of Specification Errors](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4697929) · [Causality and Factor Investing: A Primer](https://rpc.cfainstitute.org/research/foundation/2025/causality-factor-investing) · [ADIA Lab protocol](https://www.adialab.ae/research-series/a-protocol-for-causal-factor-investing)
- Goyal, Welch, Zafirov — [A Comprehensive 2022 Look at the Empirical Performance of Equity Premium Prediction](https://academic.oup.com/rfs/article/37/11/3490/7749383) (RFS 2024)
- Kelly, Malamud, Zhou — [The Virtue of Complexity in Return Prediction](https://onlinelibrary.wiley.com/doi/full/10.1111/jofi.13298) · Nagel — [Seemingly Virtuous Complexity](https://voices.uchicago.edu/stefannagel/files/2025/07/Complexity_2.pdf) · Buncic — [Simplified](https://papers.ssrn.com/sol3/Delivery.cfm/5239006.pdf?abstractid=5239006&mirid=1)
- [The forecast combination puzzle: a simple theoretical explanation](https://www.sciencedirect.com/science/article/abs/pii/S0169207016000327) · [Forecast combinations: a 50-year review](https://arxiv.org/pdf/2205.04216)
- Bollerslev, Patton, Quaedvlieg — [Exploiting the Errors (HARQ)](https://www.sciencedirect.com/science/article/abs/pii/S0304407615002584)
- Harvey et al. — [The Impact of Volatility Targeting](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=3175538)
- Ledoit, Wolf — [Analytical nonlinear shrinkage](https://www.researchgate.net/publication/345182724_Analytical_nonlinear_shrinkage_of_large-dimensional_covariance_matrices) · [The Power of (Non-)Linear Shrinking](https://dx.doi.org/10.2139/ssrn.3384500)
- Ardia, Guidotti, Kroencke — [Efficient Estimation of Bid-Ask Spreads from OHLC Prices](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=3892335) ([bidask](https://github.com/eguidotti/bidask))
- Square-root impact — [Tokyo SE universality](https://arxiv.org/html/2411.13965v3) · [Million-metaorder analysis on Bitcoin](https://www.worldscientific.com/doi/10.1142/S2382626615500082) · [Market impact survey](https://arxiv.org/pdf/2205.07385)
- Cont, Cucuringu, Zhang — [Cross-Impact of Order Flow Imbalance](https://arxiv.org/abs/2112.13213)
- Hurst, Ooi, Pedersen — [A Century of Evidence on Trend-Following Investing](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=2993026) · [A Decade of Evidence of Trend Following in Cryptocurrencies](https://arxiv.org/pdf/2009.12155)
- Liu, Tsyvinski, Wu — [Common Risk Factors in Cryptocurrency](https://onlinelibrary.wiley.com/doi/abs/10.1111/jofi.13119)
- [Funding rate arbitrage on CEX and DEX](https://www.sciencedirect.com/science/article/pii/S2096720925000818) · [Two-tiered funding-rate markets](https://www.mdpi.com/2227-7390/14/2/346) · [Fundamentals of Perpetual Futures](https://arxiv.org/pdf/2212.06888)
- [The Bitcoin VIX and Its Variance Risk Premium](https://www.pm-research.com/content/iijaltinv/23/4/84) · [Deribit: four years of volatility regimes](https://insights.deribit.com/industry/bitcoin-options-finding-edge-in-four-years-of-volatility-regimes/)
- Guijarro-Ordonez, Pelger, Zanotti — [Deep Learning Statistical Arbitrage](https://arxiv.org/abs/2106.04028)
- McLean, Pontiff — [Does Academic Research Destroy Stock Return Predictability?](https://onlinelibrary.wiley.com/doi/abs/10.1111/jofi.12365)
- [Survivorship and Delisting Bias in Cryptocurrency Markets](https://www.alexandria.unisg.ch/bitstreams/2bc8397d-47dd-4f66-8467-9004b2c9d212/download) · [StratBase: dead coins](https://stratbase.ai/en/blog/survivorship-bias-crypto)
- Acerbi, Szekely — [General Properties of Backtestable Statistics](https://doi.org/10.2139/ssrn.2905109) · [Backtesting Expected Shortfall (MSCI)](https://msci.com/www/research-report/research-insight-backtesting/0128184734)
