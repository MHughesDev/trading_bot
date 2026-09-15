# Deep Research Prompt: Uncommon Tools & Workflows for the Quant Research Agent

**Date saved:** 2026-09-14
**Purpose:** Reusable prompt for external deep-research tools (ChatGPT Deep Research, Gemini Deep Research, etc.) or for Claude itself, to surface high-leverage additions to the internal AI research agent's toolbox (`crates/research`, `AGENT-002-agent-toolbox.md`).
**How to reuse:** Paste the prompt block below verbatim into the research tool. Before reuse, refresh the "Tools that ALREADY exist" section against the current `docs/Specs/AGENT-002-agent-toolbox.md` catalogue so the research budget isn't spent re-discovering things already built.
**First run:** 2026-09-14, executed by Claude (Sonnet 5) directly via WebSearch/WebFetch — see [quant-agent-toolbox-uncommon-tools.md](../quant-agent-toolbox-uncommon-tools.md) for the resulting brief.

---

## Prompt block (copy from here down)

# Deep Research Brief: Uncommon, High-Leverage Tools and Programmatic Workflows for an Embedded Quant Research Agent

## Your role
You are a senior quant researcher and agent-systems engineer doing a literature-and-practice survey. Your reader is the architect of a quant trading platform who already knows finance, ML, and agent design. Do not explain basics. Do not pad. Every recommendation must say who uses it (or the closest public evidence), why it is rare, and what it would concretely change in an agent's behaviour.

## The system you are researching for
A self-hosted quant research platform (Rust backend, Python SDK, ClickHouse time-series store, React UI) with an **internal AI research agent** that expert quant traders drive in natural language. The agent runs a tool-use loop against a typed toolbox. Key facts:

- **Markets/data:** crypto spot (Coinbase, Kraken) and US equities (Alpaca), 1-minute bars aggregated from live trades, plus free-tier alt data (funding, open interest, option chains, prediction markets, fundamentals, macro). **Free data sources only.** No Bloomberg, no paid tick data.
- **Strategies** are a JSON definition format (signals, rules, exits, sizing, per-asset instances) compiled and run by a Rust strategy runtime. A separate market simulator does backtesting.
- **Backtesting** is built around honest evaluation: Run → Study → Experiment hierarchy, a **trial ledger** (append-only record of every trial, effective number of trials, sealed trial tickets), a **null library** (block bootstrap, random-entry matched controls, shuffled labels, synthetic paths), and a **gate funnel** (hypothesis → in-sample → null comparison → robustness → holdout → paper) that the agent must advance through with evidence.
- **MLOps suite:** train/evaluate/deploy regression, trees, XGBoost, PyTorch, LLM adapters, embeddings; feature registry; model registry; calibration; meta-labelers; datasets built from ClickHouse.
- **Execution:** paper engine with risk gate; live execution planned. Paper-vs-backtest divergence tracking exists.
- **Agent runtime:** tool budgets, output budgets, ask-user tool, structured reports, proposals that a human approves, skills/memory store, an agent evaluation suite. Can run on a strong hosted LLM or a weak local LLM (degraded tier), so tools must carry the intelligence, not the model.

### Tools that ALREADY exist or are specified (do not re-research these; you may propose upgrades)
Data: catalog/coverage/search, bars, trades/quotes/book, universe, data quality checks, synthetic data, funding/OI/options/prediction-markets/fundamentals/macro.
Analysis: return moments with SEs and JB/Ljung-Box/ARCH-LM/Hill; volatility estimators (PK/GK/RS/YZ, RV/BV/RQ, EWMA/GARCH/HARQ); regime filter with Hurst/VR/ADX; event study vs matched random entries with block-bootstrap bands; liquidity cost (EDGE spread, Roll, CS, Amihud, sqrt-impact); rolling correlation/beta/NLS covariance/Marchenko-Pastur; seasonality with BH and Romano-Wolf; ADF/KPSS/Engle-Granger/Johansen/OU half-life; distribution comparison (KS, Wasserstein); kNN analogs; fit-process/simulate-paths/fit-regime-model.
Strategy: validate/compile/save/diff/explain/translate/reconcile, preview signals, trade forensics.
Backtest/experiment: run, outputs, equity curve, explain trade, hypothesis register, sweeps, sweep status/cancel, carried-forward params, study diagnostics, parameter surface, null picker, funnel/gate advance, robustness, compare, benchmark, alpha chain, family test, capacity, posterior, decompose, factors, collider check, decay, vs-random, dossier builder.
Models: list/train/evaluate/compare/importance/predict-series/calibrate/meta-labeler; datasets build/validate; text signal series and LLM training-data contamination probe.
Agent: memory add/search, skills search/propose/patch/vote, portfolio rollup, paper activity, orders.

## What I want you to find
The **uncommon** tools and **multi-tool programmatic workflows** (fixed sequences with decision points, run as one callable unit) that separate a research agent that merely runs backtests from one an elite discretionary-quant would actually trust. "Uncommon" means: not in Zipline/vectorbt/backtrader/Lean tutorials, not a standard tear sheet, but used by serious practitioners, described in strong papers, or logically required by honest research and simply absent from open-source tooling.

Investigate each theme below. For each, report (a) what practitioners actually do, (b) the strongest published methods, (c) open-source implementations if any, (d) what would be novel to build, (e) which items compose into a workflow.

### Theme 1 — Multiple-testing and research-budget accounting
Deflated Sharpe ratio, probability of backtest overfitting (CSCV/PBO), combinatorial purged cross-validation, minimum backtest length, minimum track record length, Sharpe standard errors under autocorrelation and fat tails (Lo 2002, Ledoit-Wolf), Harvey-Liu-Zhu haircuts and multiple-testing t-stat thresholds, White's Reality Check / Hansen SPA, Bayesian trial-count priors, **holdout burn accounting** (how many looks has this holdout absorbed, and how to charge for each look), family-wise vs research-program-wise error, pre-registration mechanics. How do firms operationalise a "research budget" per hypothesis family? What is the best design for a tool that says "given your ledger, this Sharpe is not distinguishable from selection"?

### Theme 2 — Leakage, lookahead, and data-provenance forensics
Point-in-time correctness auditing, feature timestamp lineage, automated lookahead detection (e.g., perturb-future tests, timestamp-shift sensitivity, label-shuffle baselines, "too good to be true" detectors), survivorship and delisting handling in free data, restatement drift in fundamentals, resampling/bar-boundary leakage, target leakage via engineered features, LLM/embedding training-cutoff contamination when text or pretrained models are used as signals. Which of these can be made a single **leakage audit workflow** that runs before any strategy is allowed past the in-sample gate?

### Theme 3 — Adversarial and counterfactual workflows ("break the strategy")
Automated ablation of each rule/feature, parameter-neighbourhood fragility, regime-conditional failure search, alternative-history simulation (bootstrapped, regime-resampled, fitted-process, and GAN/diffusion synthetic markets — assess which are trustworthy), stress paths, "red-team" agent patterns where a second agent tries to falsify the first's claim, trade-reshuffle Monte Carlo, drawdown distribution and time-under-water bootstraps, sensitivity to entry/exit timing jitter, cost-shock curves and break-even turnover. What do practitioners consider the minimum adversarial battery before capital?

### Theme 4 — Alpha attribution, decomposition, and decay
Factor spanning tests and neutralisation, alpha vs beta vs timing decomposition, cross-sectional vs time-series contribution, signal half-life and information decay curves, publication-decay evidence (McLean-Pontiff), crowding proxies from free data, "is this just momentum/carry/vol-selling in disguise" detectors, WorldQuant-Brain-style checks (self-correlation, correlation to existing alpha pool, sub-universe consistency, turnover constraints, sign-flip tests). What makes a good **"what is this really"** workflow?

### Theme 5 — Execution realism and capacity
Fill models beyond fixed slippage: queue position, latency-aware fills, partial fills, impact models (sqrt, Almgren-Chriss, Kyle lambda), capacity curves, market-impact-adjusted Sharpe, funding/borrow costs in crypto, exchange-specific microstructure quirks (Coinbase/Kraken/Alpaca), paper-vs-backtest divergence attribution (which component: fills, timing, data?), and **shadow trading** as a research instrument. Which realism upgrades most change go/no-go decisions for 1-minute-bar strategies?

### Theme 6 — Discovery tools that are rare in retail stacks
Assess usefulness and false-positive risk of: Bayesian online change-point detection, matrix-profile motif/discord discovery, transfer entropy / Granger lead-lag scans across assets, causal discovery on lagged panels (PCMCI etc.), symbolic regression for signal search, conformal prediction for signal uncertainty, distribution-shift detectors for live-vs-train, wavelet/EMD decomposition, information-theoretic feature screens (MIC, HSIC, mutual information with block permutation), time-aware permutation importance, Hawkes processes on trade flow, VPIN/order-flow toxicity from free trade data, tail-dependence copulas, extreme-value tail estimators, Pareto-frontier multi-objective strategy selection. For each: does it produce edges or hallucinations in practice, and what guardrail tool must accompany it?

### Theme 7 — Research memory, negative results, and literature
Hypothesis registries, negative-result databases, "known anomaly" catalogues with decay tracking, paper-to-strategy replication pipelines (SSRN/arXiv q-fin ingestion → structured hypothesis → JSON strategy → replication verdict), skill libraries that encode house rules, reproducibility hashes and research diaries, and how to prevent the agent from re-testing a dead idea under a new name (semantic dedup of hypotheses). What has been published on agentic research memory that survives contact with real use?

### Theme 8 — Portfolio-level and meta tools
Combining strategies with correlated returns, capacity-aware allocation, regime-conditional allocation, meta-labeling and bet sizing, ensemble-of-strategies vs ensemble-of-models, strategy-return correlation matrices with shrinkage, kill-switch and decay-monitoring rules for live strategies, "when to stop trusting a backtest" monitors.

### Theme 9 — Agent-level mechanics specific to quant research
Claim-with-evidence output schemas, self-verification loops, tool-cost budgets, reviewer/critic agent patterns, evaluation benchmarks for quant agents (what exists, what is missing), guarding against the agent optimising the metric instead of the edge, how to make tools carry rigor so a weak local model still produces honest work, human-in-the-loop checkpoints that experts tolerate, and structured "research dossier" formats reviewers actually read.

## Sources to mine (go beyond these)
Lopez de Prado (AFML, Machine Learning for Asset Managers), Harvey/Liu/Zhu, Bailey/Borwein/LdP, Lo, Hansen, White, Romano-Wolf, McLean-Pontiff, Novy-Marx, Chen-Zimmermann open-source asset pricing, Jensen-Kelly-Pedersen replication, Man AHL and Two Sigma engineering blogs, Jane Street public writing, WorldQuant Brain docs, Numerai docs and forums, QuantConnect/Lean research features, Qlib, mlfinlab/hudson-thames, skfolio, riskfolio-lib, alphalens/pyfolio, nautilus_trader, vectorbt PRO, Arctic, Kaggle competition post-mortems for time series, recent arXiv work on LLM agents for finance and agent evaluation (2024–2026), and practitioner threads (Quantopian archive, EliteTrader/Nuclear Phynance, Twitter/X quant community) where they contain concrete methods.

## Constraints on recommendations
- Must be implementable from free data at 1-minute and daily granularity.
- Must be exposable as a typed tool (clear inputs, outputs, and an output budget) callable by an LLM, or as a workflow composed of such tools.
- Prefer methods with statistical guarantees or strong empirical track record; flag anything fashionable but unproven.
- Note anything that is a bad idea for this system and why.

## Required output format
1. **Executive summary** (≤300 words): the ten highest-leverage additions, ranked.
2. **Catalogue**: one entry per tool or workflow, grouped by theme. Each entry: name; one-line purpose; why it is uncommon; who uses it / evidence (with citations); inputs → outputs; proposed tool signature (function name, parameters, return schema); which existing tools it composes with; implementation effort (S/M/L); risk of misuse and required guardrail; priority (P0–P3).
3. **Workflows**: 8–15 named programmatic workflows, each as an ordered tool sequence with decision points, the gate it serves, and what verdict it emits. Example shape: "Leakage Audit: data.qc → feature timestamp lineage → label-shuffle baseline → future-perturbation test → verdict {clean, suspect, leaking} with evidence."
4. **Anti-patterns**: things commonly built into quant agents that harm research quality, with evidence.
5. **Open questions**: where the literature is thin and the platform would have to run its own experiments.
6. **Bibliography** with links.

Be concrete, cite everything, and distinguish clearly between "practitioners do this," "the paper proposes this," and "I am inferring this."
