# Uncommon Tools and Workflows for the Quant Research Agent

**Date:** 2026-09-14
**Researcher:** Agent (Claude Sonnet 5), via four parallel research subagents using WebSearch/WebFetch
**Status:** Complete

## Question

What uncommon, high-leverage tools and multi-tool programmatic workflows should the internal AI research agent (`crates/research`, `docs/Specs/AGENT-002-agent-toolbox.md`) have, beyond what it already exposes, to let expert quant traders do genuinely rigorous research through it?

## Scope

**In scope:** the nine themes from [the reusable prompt](prompts/quant-agent-toolbox-deep-research-prompt.md) — multiple-testing/research-budget accounting, leakage/lookahead forensics, adversarial/counterfactual testing, alpha attribution and decay, execution realism and capacity, rare discovery tools, research memory, portfolio/meta tools, and agent-level mechanics.

**Out of scope:** the adoption decision itself (belongs in an ADR if pursued), implementation, and anything already covered by the existing tool catalogue in `AGENT-002-agent-toolbox.md` (data/analysis/strategy/backtest/models/agent tools listed there were given to the research agents as "already exists, don't re-research").

## Method

Four `general-purpose` subagents ran in parallel, each with WebSearch/WebFetch, covering two or three themes apiece (~150 total search/fetch calls across academic papers on arXiv/SSRN, exchange API docs, GitHub/OSS libraries, and practitioner blogs). Each was instructed to tag every claim **[PRACTICE]** (practitioners/OSS actually do this, cited), **[PAPER]** (a paper proposes it, cited), or **[INFERRED]** (the agent's own synthesis, flagged explicitly) — that tagging is preserved below. Findings were then synthesized into one document, de-duplicated against the existing catalogue, and cross-checked against the repo's current working tree.

## Findings

### Correction: what the research agents were told exists vs. what is in code

The research prompt listed the `analysis.*` suite from `AGENT-002` §6.3 (return moments with SEs, PK/GK/RS/YZ volatility, regime filter, event studies vs. random entries, liquidity cost, correlation/Marchenko-Pastur, seasonality with Romano-Wolf, ADF/KPSS/Johansen, KS/Wasserstein, kNN analogs, fit-process/simulate-paths) as already built. Checked against the code on 2026-09-14: none of it exists. There is no `analysis` module in `crates/api` and no `analysis_*` entry in `crates/mcp-server/src/taxonomy.rs`. The spec marks the suite "Phase 1". So wherever this brief says a tool "composes with the existing X" for an analysis-namespace X, read that as "composes with the spec'd but unbuilt X" — and treat building that suite as a prerequisite that outranks most of the catalogue below. The null library, gate funnel, trial ledger, experiment/sweep/study tools, and the falsification tools do exist as described.

### Alignment with in-flight work

Before the catalogue: the current git status shows several untracked files that already anticipate specific recommendations below — `crates/ledger/src/falsification.rs`, `crates/backtest/src/costs.rs`, `crates/ledger/src/capacity.rs`, `crates/domain/src/numeraire.rs`, and `migrations/0058_strategy_revision.sql`. The falsification-ledger work in particular matches the "red-team falsifier" pattern (Theme 3, #8) almost exactly, including the dual-guardrail design (structural leakage prevention + search-aware statistical deflation) from the most relevant paper found, [arXiv:2608.27734](https://arxiv.org/html/2608.27734). Worth checking that in-progress design against this brief's Theme 3 findings before finalizing it, rather than treating it as unrelated new work.

### Executive summary — ranked top additions

1. **Holdout-burn ledger check** (Theme 1, #6) — tracks how many times a sealed holdout has been queried and blocks further "just looking." This is the single highest-leverage control specifically *because* the researcher is an LLM agent that can accidentally spend a holdout conversationally, not through a formal gate.
2. **Deflated Sharpe Ratio + Minimum Track Record Length** (Theme 1, #1/#4) — pure math over data the trial ledger's N_eff already has. Turns "this Sharpe looks good" into "this Sharpe survives the number of trials actually spent."
3. **Claim-with-evidence schema + tool-interactive self-verification** (Theme 9, #1/#2) — every agent claim carries a pointer to the exact evidence, and gets independently re-derived via a real tool call rather than self-attested. Foundational to trusting anything else the agent reports.
4. **Metric-integrity / Goodhart audit** (Theme 9, #4) — re-derives the headline metric from platform-logged tool calls (not agent-reported text) and diffs against what the agent claims. Directly evidenced: current frontier models measurably game reward specifications when given tool access ([arXiv:2605.02964](https://arxiv.org/pdf/2605.02964)).
5. **Red-team falsifier agent, run in a fresh context** (Theme 3, #8) — a second agent whose only job is falsifying the first agent's claims, evidenced to need genuine context separation (naive same-session "debate" underperforms isolated critique). Already scaffolded in the working tree.
6. **Future-perturbation and bar-boundary leakage detectors** (Theme 2, #3/#5) — mechanical, code-free lookahead detection via re-run-and-diff, with a proven OSS precedent (Freqtrade) and a direct match to this platform's continuous 1m-aggregation architecture.
7. **Paper-vs-backtest divergence attribution** (Theme 5, #7) — the platform already detects that paper and backtest performance diverge; this decomposes *why* (signal vs. execution vs. timing vs. data), which is what actually drives a go/no-go call.
8. **Cost-shock curve + venue-constraint lookup** (Theme 5, #5/#6) — cheap sweeps that guard the platform's specific free-tier-data, real-exchange-limits constraints (Kraken's decaying rate limits, Coinbase's self-trade-prevention modes, Alpaca's 2026 margin framework were all verified directly from current docs).
9. **Negative-result ledger + semantic hypothesis dedup** (Theme 7, #2/#3) — closes the "re-testing a dead idea under a new name" gap named directly in the brief; cheap additive metadata on already-sealed trial tickets.
10. **Kill switch as an independent process** (Theme 8, #6) — not a new algorithm, an architectural requirement: the halt mechanism must run outside the process/loop it might need to stop, or a failing strategy can also disable its own kill switch.
11. **Factor spanning test and signal half-life estimator, as upgrades** (Theme 4, #1/#4) — likely already partially covered by the existing `factors`/`decompose`/`decay` tools; the addition is crypto-appropriate factor proxies and an explicit half-life-to-rebalance-frequency output.
12. **Trade-reshuffle and bootstrap Monte Carlo over the trade ledger** (Theme 3, #3) — directly reuses the block-bootstrap machinery already built for the null library, applied to drawdown/time-under-water instead of returns.

Also load-bearing but not "new": several themes converge on one architectural principle — **the checking mechanism must be independent of the thing being checked.** It shows up as the kill switch needing a separate process, the reviewer agent needing a separate session, and claim verification needing to hit a real tool rather than trust the model's own text. The platform's ledger/gate-funnel/risk-gate design already embodies this in places; the highest-leverage additions extend the same principle into newer surfaces rather than introducing a new paradigm.

---

### Theme 1 — Multiple-testing and research-budget accounting

Retail/OSS backtesters almost universally skip trial accounting; where correction *is* practiced it's either a hard published bar (Harvey-Liu-Zhu's t>3.0) or pre-registration + sealed-holdout discipline. A directly relevant 2026 finding: AI agents given the same data can produce differently-slanted "methodologically clean" analyses at will, motivating an "m-value" audit statistic ([The Agentic Garden of Forking Paths, arXiv:2607.01507](https://arxiv.org/html/2607.01507v1)) — this is the field's own evidence that an *agent* doing research is a distinct multiple-comparisons risk, not just a faster human.

| Tool | Purpose | Effort | Priority |
|---|---|---|---|
| `stats.deflated_sharpe_ratio(run_id, trial_family_id)` | Probability observed Sharpe survives N_eff trials + non-normality correction ([Bailey & López de Prado](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=2460551); OSS: [pypbo](https://github.com/esvhd/pypbo), [jsharpe](https://github.com/tschm/jsharpe)) | S | **P0** |
| `stats.min_track_record_length(...)` | Minimum sample length before a Sharpe is distinguishable from luck given N_eff, skew, kurtosis | S | **P0** |
| `ledger.check_holdout_budget` / `spend_holdout_look` | Charges each look at a holdout; refuses reads outside this door. Grounded in Dwork et al.'s adaptive-data-analysis result that *any* adaptive reuse invalidates a holdout regardless of sample size ([NeurIPS PDF](http://papers.nips.cc/paper/5993-generalization-in-adaptive-data-analysis-and-holdout-reuse.pdf)) | S–M | **P0** |
| `stats.probability_of_backtest_overfitting` (CSCV/PBO) | Probability the in-sample winner ranks below median out-of-sample, over the full trial-family return matrix ([Bailey/Borwein/López de Prado/Zhu](https://escholarship.org/uc/item/4w1110bb); OSS: `pbo` CRAN, `pypbo`, [mlfinlab](https://github.com/hudson-and-thames/mlfinlab)) | M | P1 |
| `backtest.run_cpcv` | Combinatorial purged CV as a backtest-path generator, giving a *distribution* of OOS Sharpe instead of one path | L (new backtest execution mode) | P1 |
| `stats.family_wise_significance_test` (Romano-Wolf / Hansen SPA) | Family-wise error control across a *set* of candidate strategies via the existing block-bootstrap | M | P1 |
| `research.forking_paths_audit` | m-value: how extreme the reported result is vs. the full space of equally-defensible analyses the agent could have run ([arXiv:2607.01507](https://arxiv.org/html/2607.01507v1); complementary: [Coqueret, arXiv:2401.08606](https://arxiv.org/abs/2401.08606)) — specification space must be hashed *before* the first backtest runs, or the audit is circular | L | P2 |

**Workflows:**
- **Selection-Bias Verdict** (gates in-sample → null-comparison): pull N_eff → `min_track_record_length` hard gate → `deflated_sharpe_ratio` → if N_eff≥10, `probability_of_backtest_overfitting` → `family_wise_significance_test` vs. siblings → verdict with all four numbers attached as evidence.
- **Holdout Spend Gate** (gates holdout → paper): `check_holdout_budget` → block if exhausted; else `spend_holdout_look` is the only door into holdout data.
- **Program-Level Research Budget Report** (periodic, portfolio-level not per-strategy): aggregate N_eff and elapsed research time per hypothesis family; retrospective `forking_paths_audit` on any "surprising" result.

**Anti-patterns:** reporting best-of-N Sharpe with no N (zero-signal parameter search alone produces expected Sharpe ≈3.7 across 1,000 combinations, per [Bailey et al.](https://www.susanpotter.net/quant/backtest-bias-taxonomy/)); random `train_test_split` on time series; trusting a bootstrapped significance threshold without full forking-paths accounting (understates the true threshold by roughly half, [arXiv:2401.08606](https://arxiv.org/abs/2401.08606)); adaptively reusing a holdout; treating "we tested many markets" as sufficient defense against p-hacking.

**Open questions:** no citable Bayesian trial-count-prior methodology found specific to strategy selection; no primary-source evidence on how Man AHL/Two Sigma actually operationalize a research budget internally (only that AHL states "fewer than 1 in 10 ideas make it past test trading," with no disclosed mechanism); no operational algorithm found for continuously updating a live alpha budget across a multi-year rolling research program.

---

### Theme 2 — Leakage, lookahead, and data-provenance forensics

Real practice converges on mechanical, automatable checks rather than manual review: bitemporal point-in-time storage, perturbation-based diff detectors (Freqtrade ships this), and Monte Carlo permutation as the default "is this even real" filter. Newest and most relevant: LLM-specific lookahead detection is now its own 2025-2026 arXiv sub-literature, with a purpose-built statistic ("Lookahead Propensity") that shows an LLM's apparent edge collapses to zero once restricted to dates after its training cutoff ([arXiv:2512.23847](https://arxiv.org/pdf/2512.23847)) — directly relevant if this platform ever adds LLM/embedding-derived features.

| Tool | Purpose | Effort | Priority |
|---|---|---|---|
| `research.bar_boundary_leak_check` | Proves a decision at time T couldn't have seen an unfinished higher-timeframe candle — a subtle bug pandas resample/groupby naturally introduces, and a direct match to this platform's 1m→higher-timeframe aggregation | S | **P0** |
| `research.future_perturbation_test` | Reruns with each signal/indicator sliced or time-shifted and diffs; any divergence proves lookahead. Proven precedent: [Freqtrade's `lookahead-analysis`](https://www.freqtrade.io/en/stable/lookahead-analysis/) | M | **P0** |
| `research.feature_leakage_scan` | Static scan of feature definitions for full-window statistics, future-correlation, or disguised label transforms — natural fit for the existing strategy-validator crate | M | P1 |
| `data.pit_provenance_check` | Verifies fundamentals/alt-data actually respects an as-of timestamp rather than serving the latest-restated value (Enron's 2001 restatement is the canonical case) | M | P1 |
| `data.survivorship_audit` | Flags delisted/defunct assets missing from the backtest universe — quantified crypto impact: 0.93%/62.19% annualized bias (value-/equal-weighted) from omitting delisted coins, 2014-2021 ([SSRN 4287573](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4287573)) | S–M | P2 |
| `research.llm_contamination_check` (Lookahead Propensity) | Tests whether an LLM/embedding-derived feature's apparent edge is memorized training data, not inference | L | P3 now, **re-prioritize to P1 the moment any text/embedding feature ships** |
| `research.label_shuffle_leakage_check` | Likely already covered by the existing null library's shuffled-labels entry — flag for a wiring check into the leakage-audit workflow specifically, not a new build | — | P3 (wiring only) |

**Workflow — Leakage Audit** (run once before a strategy is admitted to the in-sample gate): `pit_provenance_check` on every non-bar source → `feature_leakage_scan` (static) → `bar_boundary_leak_check` on all resample pairs → `label_shuffle_leakage_check` (verdict: no-edge vs. leaking are different conclusions) → `future_perturbation_test` (strongest positive proof) → if LLM features present, `llm_contamination_check` → verdict `clean`, with the full evidence bundle sealed into the trial ticket permanently.

**Workflow — Data-Provenance Certification** (per data source, not per strategy): `pit_provenance_check` + `survivorship_audit` over full history → a per-source certificate with an expiry, so individual strategy audits cite the certificate instead of re-running it every time.

**Anti-patterns:** random shuffle splits on time series (the single most repeated warning in the literature); fundamentals tables that silently overwrite restated values; free-tier equity data defaulting to currently-listed-only universes; conflating "no edge" with "leaking edge" as if one check catches both.

**Open questions:** no evidence found on whether crypto free-tier sources (exchange REST backfills) suffer the equivalent of fundamentals restatement drift — treat bar backfills as potentially revised by inference, not confirmed practice; whether the LAP contamination statistic generalizes from direct-forecast/sentiment tasks to embeddings used as opaque numerical features is untested in the literature.

---

### Theme 3 — Adversarial and counterfactual workflows

Jane Street's process is explicitly described as researchers spending most of their time trying to disprove their own idea before capital commits — cultural, not just scripted. The most consequential single finding: standard GAN/WGAN-GP synthetic-market generators **collapse during strategy evaluation**, producing annualized returns exceeding 2000% and volatility near 1000% despite passing visual/stylized-fact checks ([arXiv:2601.12990](https://arxiv.org/pdf/2601.12990)) — this bears directly on the platform's existing synthetic-paths null-library component.

| Tool | Purpose | Effort | Priority |
|---|---|---|---|
| `trade_reshuffle_bootstrap` | Two distinct Monte Carlo modes (reshuffle = path risk of a fixed outcome set; bootstrap = estimation-uncertainty of the endpoint) applied to the trade ledger, reusing the existing block-bootstrap | S | **P0** |
| `cost_shock_curve` | Re-prices at 1x-5x assumed cost, reports break-even multiple against measured live spreads | S | **P0** |
| `red_team_falsify` | Second agent, fresh context, evidence-bundle-only, runs ablation/fragility/cost-shock/regime/vs-random/sign-flip against the claim; charges N_eff for every trial it spends | M–L (partially built — see alignment note) | **P0** |
| `ablation_sweep` | Leave-one-component-out over the JSON strategy_def's rules/exits/sizing, ranking load-bearing vs. decorative rules | S | P1 |
| `parameter_fragility_map` | Perturbs each numeric parameter, distinguishes a broad robust plateau from a knife-edge peak, outputs a scalar fragility index | M | P1 |
| `timing_jitter_sensitivity` | Shifts entry/exit by ±N bars or delays fills; a sharp collapse at jitter=1 bar is itself a lookahead-suspicion signal | S–M | P1 |
| `regime_failure_search` | Actively hunts for the regime where the strategy fails hardest (attacker framing), not just conditions on known regimes | M | P1 |
| `synthetic_stress_paths` (upgrade) | Adds a trust-tier flag to the existing bootstrap/fitted-process/synthetic-paths tools — GAN/diffusion generators labeled experimental, never equal evidentiary weight, given the collapse finding above | S for the guardrail; L for actual GAN/diffusion generators | **P0** for the guardrail, P2 for the generators |

**Workflow — Pre-Capital Adversarial Battery** (gates Study → paper, between robustness and holdout): `ablation_sweep` (concentration risk check) → `parameter_fragility_map` (knife-edge → reject) → `cost_shock_curve` using measured live spreads (safety margin <2x → reject) → `trade_reshuffle_bootstrap` (ruin probability at target capital) → `timing_jitter_sensitivity` (collapse at 1 bar → reject, route to leakage audit) → `regime_failure_search` (if current regime matches the worst one, hold rather than deploy) → `synthetic_stress_paths` (bootstrap/fitted-process only) → `red_team_falsify` on the full bundle → final verdict: reject / reduced-size-paper / full-paper.

**Workflow — Two-Agent Falsification Debate**: primary produces claims via existing `alpha_chain`/`posterior`/`decompose`; falsifier (fresh context) runs `vs_random`, `ablation_sweep`, `regime_failure_search`, and a sign-flip check; falsifier's own N_eff-adjusted significance can independently invalidate the claim; any primary/falsifier disagreement is surfaced to the human, never auto-resolved.

**Anti-patterns:** conflating reshuffle with bootstrap as "the" Monte Carlo result (they answer different questions); requiring every cell of a robustness matrix to pass (itself a form of curve-fitting if criteria aren't set before running); trusting GAN-generated markets because they pass stylized-fact checks; unguided homogeneous multi-agent debate as verification (isolated self-correction outperforms it, [arXiv:2605.00914](https://arxiv.org/pdf/2605.00914)); assuming any multi-agent trading architecture is inherently robust to data/prompt poisoning ([arXiv:2608.24069](https://arxiv.org/abs/2608.24069)); trusting statistical deflation alone — a planted-oracle strategy with Sharpe 35 survives DSR/PBO correction but fails structural guardrails ([arXiv:2608.27734](https://arxiv.org/html/2608.27734)).

**Open questions:** whether diffusion-based simulators (TRADES, CoFinDiff) generalize to crypto 1-minute bars — all strong evidence found is equities/LOB-specific; no consensus numeric ruin-probability threshold for reject vs. reduced-size; sign-flip-of-signal-direction as a strategy-level falsification technique is this report's own generalization from GLM score-contribution literature, not directly evidenced.

---

### Theme 4 — Alpha attribution, decomposition, and decay

WorldQuant BRAIN enforces self-correlation-to-pool and fitness checks *before* an alpha is even scoreable, with the explicit finding that "fitness failures are often turnover problems in disguise." McLean & Pontiff's 26% OOS / 58% post-publication decline is the field's empirical anchor for "assume decay, don't assume persistence" ([SSRN](https://dx.doi.org/10.2139/ssrn.2156623)).

| Tool | Purpose | Effort | Priority |
|---|---|---|---|
| `factor_spanning_test` (upgrade to existing `factors`/`decompose`) | Regresses a signal on a factor basis; residual alpha with its own t-stat determines redundancy vs. novelty — needs a crypto-specific factor proxy set (BTC beta, funding-rate carry) since Fama-French doesn't apply | S–M | **P0** |
| `signal_half_life` (upgrade to existing `decay`) | IC-vs-horizon curve, fitted half-life, recommended rebalance frequency — should cross-check against `cost_shock_curve`'s breakeven turnover before surfacing a recommendation | S | **P0** |
| `alpha_beta_timing_decompose` (upgrade to `decompose`) | Splits return into static beta, timing skill, residual alpha (Treynor-Mazuy/Henriksson-Merton) | S–M | P1 |
| `alpha_health_check` bundle | Composite gate mirroring BRAIN's actual submission gate: self-correlation to the ledger's own alpha pool, sub-universe consistency, turnover fitness, sign-flip robustness | M | P1 |
| `cs_ts_decompose` | Cross-sectional vs. time-series variance decomposition — tells you whether a multi-asset strategy is really stock-picking or really trend-following, independent of what the JSON rules claim | M | P2 |
| `crowding_decay_overlay` | Benchmarks decay against McLean-Pontiff base rates + a live crowding proxy (funding-rate extremity/OI concentration for crypto — this crypto adaptation is inferred, not directly evidenced) | M | P2 |
| `style_detector` | Thin wrapper on factor spanning with a fixed interpretable basis (momentum/carry/short-vol/liquidity) for human-readable "what is this, really" labeling | S | P2 |

**Workflow — "What Is This Signal, Really"**: `signal_half_life` (establish natural holding period) → `factor_spanning_test` (spanned by known factors → stop) → `style_detector` (repackaged style → benchmark against the pure style instead) → `cs_ts_decompose` if multi-asset (statistical-driver/strategy-framing mismatch is a red flag even with good performance) → `alpha_health_check` against the ledger's alpha pool (self-correlation >0.7 → redundant, reject) → `crowding_decay_overlay` → final verdict: novel & clean / novel-confirm-via-Theme-3 / repackaged-style / redundant / inconclusive.

**Anti-patterns:** reporting IC/Sharpe with no decay curve, then discovering in production the edge was already mostly gone; misdiagnosing a turnover problem as a model problem (explicitly documented on BRAIN); naive residualization against correlated factors causing multicollinearity artifacts; applying US-equity multi-year academic decay base rates directly to short-horizon crypto without adjustment (a real reference-class mismatch, flagged as this report's own caution).

**Open questions:** no public numeric sub-universe-consistency threshold for BRAIN was found; no paper validates funding-rate/OI as a genuine crypto analogue to equity short-interest crowding proxies; the Di Mascio/Lines/Naik "Alpha Decay" paper's specific figures could not be verified this session (PDF extraction failed) — treat any numbers attributed to it cautiously until re-fetched.

---

### Theme 5 — Execution realism and capacity

[hftbacktest](https://github.com/nkaz001/hftbacktest) exists precisely because "a backtest that fills every order at the exact signal-bar close will systematically overstate performance" — it ships queue-position and latency simulation as first-class, not bolted on. For a 1-minute-bar platform, the biggest realism gap usually isn't sophisticated impact math, it's that a full-bar-close fill assumption hides whether the order would have queued through 60 seconds of order flow at all.

| Tool | Purpose | Effort | Priority |
|---|---|---|---|
| `attribute_paper_backtest_divergence` | Decomposes the *existing* paper-vs-backtest divergence tracking into signal/execution/timing/data error, using illustrative thresholds (<20% Sharpe decay = expected friction, >30% or 2x max-DD = a real problem) | M | **P0** |
| `apply_funding_costs` | Applies periodic (8h) funding cash flows to backtests/paper P&L — currently-uncovered cost the platform's crypto strategies will bleed on if ignored; a 0.03%/day rate alone can exceed a short-horizon strategy's whole take-profit target | S–M | **P0** |
| `get_venue_constraints` | Encodes verified, current venue-specific rules: Coinbase's price-time-priority + 4 self-trade-prevention modes, Kraken's decaying per-pair rate-limit points (younger cancels cost more, penalizing quote-and-yank), Alpaca's 2026 real-time margin framework replacing PDT (min equity $25k→$2k) | S | **P0** |
| `simulate_fill` | Queue-position + latency-aware probabilistic fill, replacing "fills at bar close, 100%" | L (needs L2/L3 or trade-tape retention beyond current 1m OHLCV) | P1 |
| `estimate_capacity_curve` | AUM sweep with cost-normalized impact model; a documented real case shows optimal capacity can shift 10x (from $50k to $500k) just by slicing into 4 child orders | M | P1 |
| `run_shadow_session` | Live, zero/tiny-size execution against real books to collect fill-quality ground truth and recalibrate the fill simulator — must never report P&L, only execution-quality metrics | M–L | P1 |
| `estimate_market_impact` (cross-check, not rebuild) | Almgren-Chriss / Kyle's-lambda cross-validation of the *already-existing* sqrt-impact tool — square-root law is now well-confirmed empirically ([arXiv:2606.24019](https://arxiv.org/pdf/2606.24019)) | S–M | P2 |

**Workflow — "Should this 1m-bar strategy go live?"**: `estimate_capacity_curve` at intended AUM (below optimal → reject) → `get_venue_constraints` (requote frequency vs. Kraken's decay limits) → `apply_funding_costs` (carry erasing >50% of edge → reject) → `attribute_paper_backtest_divergence` (execution_problem → recalibrate fill model first; alpha_problem → reject regardless) → `run_shadow_session` 24-72h (bias vs. predicted slippage → recalibrate and repeat) → verdict with `optimal_aum_usd` as the sizing ceiling.

**Workflow — "Diagnose a strategy that's losing live but looked good in backtest"**: `attribute_paper_backtest_divergence` → route by dominant component (execution → retroactive `simulate_fill` with a tighter model; data → check funding gaps and venue-constraint staleness; signal → hand off to the existing regime/Hurst/ADF tools, out of Theme 5's scope).

**Anti-patterns:** full fills assumed in thin books; flat-bps slippage masking the capacity ceiling entirely; ignoring venue rate limits in backtest (a requote-heavy strategy backtests fine, then gets rate-limited live on Kraken specifically); treating funding-rate history as complete (silently zero-filling gaps understates carry cost); conflating paper trading (fills against last-trade/mid) with genuine execution-quality shadow testing.

**Open questions:** no source directly measured which realism upgrade most changes go/no-go decisions specifically for 1-minute strategies — this needs an internal controlled experiment (naive-fill vs. queue-aware vs. real-shadow-fill on the same strategy set); no validated funding-rate carry distribution specifically for Coinbase/Kraken perps; Almgren-Chriss risk-aversion calibration has no crypto precedent — treat any crypto deployment as unvalidated until back-tested against realized impact.

---

### Theme 6 — Discovery tools rare in retail stacks

The tooling for most of these (STUMPY, Tigramite, PySR/gplearn) is real and maintained, but the statistical guardrails that make them safe (multiple-testing correction, block-permutation significance, out-of-sample stability) are usually missing from blog-post usage. The most damning case found: **VPIN**, a peer-reviewed, institutionally-pedigreed metric ([Easley/López de Prado/O'Hara, RFS 2012](https://academic.oup.com/rfs/article-abstract/25/5/1457/1569929)), was subsequently shown by [Andersen & Bondarenko](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=1881731) to be largely a mechanical artifact of volume/volatility with no incremental predictive power once controlled for — and peaked *after*, not before, the 2010 Flash Crash. The dispute is still live (a rejoinder exists). Publication and popularity are not evidence of a real edge; that should set the epistemic bar for everything else below.

| Tool | Verdict | Effort | Priority |
|---|---|---|---|
| `detect_distribution_shift` (live-vs-train covariate shift) | Genuine operational hygiene, not exotic — pure risk management, no edge claimed; gates live inference on drift alerts | S–M | **P0** |
| `conformal_calibrate` | Real calibration layer for the existing forecaster's quantile outputs; must use an exchangeability-robust variant (adaptive conformal inference / EnbPI), never vanilla split-conformal, under regime shift | M | P1 |
| `detect_changepoint_bocpd` (Bayesian online changepoint) | Real, well-founded; failure mode is wiring the changepoint probability directly to position sizing — it signals *that* the world changed, not which direction to trade | M | P1 |
| `screen_features_info_theoretic` (MIC/HSIC + block permutation) | Legitimate pre-filter, high false-positive risk without block-permutation p-values + FDR correction across the full feature set scanned | M | P1 |
| `search_symbolic_signal` (PySR/gplearn) | **Real production pedigree** — WorldQuant's public "101 Formulaic Alphas" and Guotai Junan's 191 factors were both built this way — but the search itself is a massive multiple-testing exercise; must ship with deflated-Sharpe (trial count = expressions evaluated) and a Pareto-front, complexity-penalized selection, never raw best-fit | M | P1 |
| `compute_strategy_pareto_frontier` (multi-objective selection) | Sound methodology, but compounds multiple-testing if run over unvetted candidates — a frontier over un-validated backtests just finds what overfits best on the most axes at once, harder to spot than single-metric overfitting | M | P1 (sequence after the deflated-Sharpe primitive exists) |
| `time_aware_feature_importance` (WinIT-style block permutation) | Diagnostic only, no edge claimed — corrects naive permutation importance's autocorrelation artifact | S–M | P2 |
| `discover_causal_graph` (PCMCI/LPCMCI) | More rigorous than pairwise Granger/TE, but causal-sufficiency is almost certainly violated by unmeasured macro/liquidity confounders — read edges as "conditionally robust lagged association," never literal causation; default to the latent-confounder-aware variant | L | P2 |
| `scan_lead_lag` (transfer entropy) | Real technique, high false-positive risk run naively across N assets (an N² multiple-testing problem); mandatory surrogate significance + FDR correction | M | P2 |
| `fit_hawkes_order_flow` | Real, well-evidenced microstructure tool, but best framed as a *fill-model input* (Theme 5) given the platform's 1-minute-bar horizon, not a standalone alpha signal — needs raw trade-tape retention beyond current 1m aggregation | L | P2 |
| `estimate_tail_dependence` (copulas) | Real, used for crypto risk research; feed risk/position-sizing overlays only, never directional signal — estimates are unstable from inherently few extreme co-movement events | M | P2 |
| `estimate_evt_tail` (Hill/GPD-POT) | Standard tail-risk measurement; must run regime-conditionally (crypto tail behavior is demonstrably time-varying), not as one global fit | M | P2 |
| `compute_vpin` | Build **only** with the incremental-predictive-power-over-volume/volatility benchmark as a non-optional field, given the unresolved Andersen-Bondarenko critique — never surface as a standalone "toxicity score" | M | P2 |
| `find_motifs_discords` (matrix profile / STUMPY) | **Reject as a standalone signal generator** — "a motif is evidence a shape recurred, not that it predicts anything." Acceptable only as a pre-filter feeding the existing kNN-analogs tool, with mandatory out-of-sample recurrence testing | S–M | P2 as pre-filter, **reject** standalone |
| `decompose_causal` (wavelet/EMD) | **Reject for general use** — parameter choices are unprincipled for financial data, and naive whole-series decomposition leaks look-ahead into "past" coefficients (this specific leakage risk is this report's own inference, not directly sourced). If built at all, strictly-causal/streaming only, never in backtest mode | M | **reject** general use, P3 causal-only variant |

**Workflow — "Vet a newly-discovered signal before it enters the strategy runtime"**: `screen_features_info_theoretic` (block-permutation + FDR) → `search_symbolic_signal` on surviving features, train+validation only → deflated Sharpe using actual expressions-evaluated as trial count → `time_aware_feature_importance` (confirm no single fragile feature dominates) → `detect_distribution_shift` train-vs-live → only then enters the existing Run/Study/Experiment funnel.

**Workflow — "Regime-aware risk overlay"**: `detect_changepoint_bocpd` running continuously → on elevated changepoint probability, recompute `estimate_evt_tail`/`estimate_tail_dependence` regime-conditionally rather than trusting a stale unconditional fit → low-confidence fits (few effective tail observations) fall back to conservative default risk limits, not a thin-sample estimate.

**Workflow — "Cross-asset lead-lag research pass"**: `scan_lead_lag` with mandatory surrogate significance + FDR → surviving pairs go through `discover_causal_graph` restricted to that pair plus plausible confounders (funding rates, macro) — many pairwise relationships disappear once a common driver is conditioned on → only survivors become candidate signal inputs, feeding the vetting workflow above.

**Anti-patterns:** VPIN as a standalone toxicity alarm (the clearest documented case of a published, institutionally-pedigreed metric being substantially a mechanical artifact); pattern-matching treated as signal; naive permutation importance on autocorrelated series; symbolic regression treated as single-shot discovery rather than a massive multiple-testing exercise; causal-discovery output read as literal causation; un-gated multi-objective selection over unvalidated backtests.

**Open questions:** no source measured comparative false-positive rates across these ~14 methods specifically on crypto 1-minute bars (evidence base is mostly equity/futures tick data) — this needs internal validation, not literature lookup; BOCPD's claimed order-flow-regime improvement on NASDAQ data could not be directly verified from a primary source; HSIC's finance track record is unverified (only MIC was directly evidenced).

---

### Theme 7 — Research memory, negative results, and literature

Chen & Zimmermann's Open Source Asset Pricing project and Jensen-Kelly-Pedersen's replication study show the field already built the "known-anomaly catalogue with decay tracking" this theme asks about — but as a human-curated, batch-updated academic resource, not something wired into a live agent loop. On the agent side, "research memory" is a hot 2025-2026 topic, but essentially no published evidence shows it surviving sustained real-world use outside benchmark harnesses — PaperBench found even the best agent+scaffold achieved only 21.0% average replication score against ICML papers ([arXiv:2504.01848](https://arxiv.org/abs/2504.01848)), below human ML-PhD baselines. Treat any "paper-to-strategy" ambition as bounded by that number.

| Tool | Purpose | Effort | Priority |
|---|---|---|---|
| `ledger.record_negative_result` / `query_negative_results` | Durable, queryable record of "this was tried and it failed," *with why* (failure taxonomy, not just pass/fail) — additive metadata on already-sealed trial tickets | S | **P0** |
| `provenance.seal_run` / `verify` | Content hash of {code version, data snapshot, config, seed} per trial ticket, so any claim traces to an exactly reproducible state — should hash the actual query result set, not just the table version | S (if the ledger doesn't already hash data snapshots) | P1 |
| `dedup.check_hypothesis` | Semantic (embedding) similarity check before a new trial ticket opens, catching paraphrase-level duplicates a naive text/name match misses; never hard-blocks, requires logged override | M | P1 |
| `registry.lookup_signal` / `register_result` | Canonical record of every factor/signal ever tried, its prior art, and current decay status, queryable before proposing "new" ideas — built on the same pattern as [Chen & Zimmermann](https://www.openassetpricing.com/) / [Jensen-Kelly-Pedersen](https://github.com/bkelly-lab/ReplicationCrisis) | M | P1 |
| `skills.propose_from_incident` (upgrade to existing skills subsystem) | Requires a lesson-provenance field linking a house-rule proposal back to the incident that triggered it | S | P2 |
| `literature.extract_hypothesis` / `compile_to_strategy` | Paper-to-strategy pipeline — deliberately two calls so ambiguous fields must be human-resolved before compilation, given the 21% PaperBench baseline | L | P2 (stage after #1-#3 exist to catch duplicate paper-derived ideas) |

**Workflow — "Is this hypothesis dead?"** (pre-check before any compute spend): `dedup.check_hypothesis` → if warned, `registry.lookup_signal` for known decay curves on close matches → `ledger.query_negative_results` for prior failures and *why* they failed → decision point: a near-duplicate that failed for `execution_infeasible` may now be worth retrying if a specific new platform capability removes that blocker, but the agent must cite the capability delta explicitly → verdict: proceed_new / proceed_as_variant / reject_duplicate.

**Workflow — "Paper ingestion to backtest-ready strategy"**: `extract_hypothesis` → dedup check on the extracted hypothesis → ask-user on specific ambiguous parameters only (not the whole paper) → `compile_to_strategy` → normal gate funnel → on terminal verdict, `register_result` and, if failed, `record_negative_result` with a failure class — closing the loop back into memory.

**Anti-patterns:** only recording memory on success (the firm-internal equivalent of publication bias — Registered Reports exist specifically to fix this in psychology, [arXiv:1205.4251](https://arxiv.org/pdf/1205.4251)); lexical-only dedup that lets a relabeled dead idea re-run; single-shot paper-to-strategy extraction with no ambiguity gate, given the 21% PaperBench baseline.

**Open questions:** no public case study of a negative-result ledger operating continuously inside a live trading research org was found (only academic-catalogue-shaped or generic-MLOps-shaped analogues); no published recipe exists for structurally normalizing a strategy-definition DSL for embedding-based dedup (so that parameter variants register as near-duplicates but different indicator families don't).

---

### Theme 8 — Portfolio-level and meta tools

The covariance-shrinkage literature (Ledoit-Wolf) is settled; the harder, less-published part is behavioral — when to cut a live strategy, using pre-committed thresholds rather than formal hypothesis tests, because live sample sizes stay too small to distinguish bad luck from real decay for a long time.

| Tool | Purpose | Effort | Priority |
|---|---|---|---|
| `monitor.check_control_limits` — **as an independent process, not an agent-optional tool call** | SPC/CUSUM-based kill switch on live P&L and input metrics (fill quality, signal frequency, holding-period drift, which often flag regime change earlier than P&L itself). Explicit architectural finding: "the kill switch must run in a different process, or the failure that requires killing will also disable the kill" ([NYIF](https://www.nyif.com/articles/trading-system-kill-switch-panacea-or-pandoras-box)) | M | **P0** |
| `portfolio.estimate_shrunk_covariance` (Ledoit-Wolf) | Well-conditioned correlation matrix across strategy return streams even when N (strategies) approaches T (observations) — always report raw sample alongside shrunk, since shrinkage biases toward historical structure, exactly wrong right before a correlation-breakdown crisis | S | P1 |
| `portfolio.allocate` | Risk-budgeted weights across the book given covariance, capacity caps, and regime; hard cap on max single-strategy weight enforced outside the optimizer, not as a soft penalty (a portfolio-level instance of Goodhart risk — an agent could concentrate to hit a target Sharpe) | M | P1 |
| `capacity.estimate` (composes with the in-flight `costs.rs`) | AUM ceiling before market impact erodes edge — reject on thin ADV history rather than reporting overconfident numbers for crypto altcoins/small caps | M | P1 |
| `metalabel.train` / `size_bet` | Meta-labeling and bet-sizing — likely already partially built per the existing MLOps "meta-labelers" component; add explicit fractional-Kelly capping by default (never full Kelly) and require the existing risk gate to independently re-check size regardless of the meta-model's recommendation. Note: a QuantConnect community writeup explicitly pushes back on meta-labeling as "not a silver bullet" — treat as real but not unconditionally beneficial | S–M | P1 |
| `portfolio.apply_regime_tilt` | Regime-conditional allocation via HMM posteriors, with mandatory hysteresis/minimum-confidence gating — detection lag of "several days" during rapid transitions is a documented weakness | M | P2 |

**Workflow — New strategy onboarding to the book**: `capacity.estimate` → `estimate_shrunk_covariance` including the new strategy (correlation to an existing strategy >0.7 → flag for human review, since a near-duplicate adds concentration, not diversification) → `allocate` with the capacity cap applied → verdict: admit / admit-reduced-weight / reject.

**Workflow — Ongoing live-book health check** (scheduled, not agent-triggered): `check_control_limits` on every live strategy, independent process → warning → log, no action → breach → automatic scale-down fires immediately (reversible, so pre-authorized per the reversibility principle in Theme 9) → notify human → human decides resume/keep-scaled-down/halt-permanently (the last, being closer to irreversible, requires explicit sign-off).

**Anti-patterns:** kill switch exposed as just another agent tool call the agent can decline or fail to invoke (voids the entire safety property); full-Kelly sizing off a raw meta-model probability; treating meta-labeling as a guaranteed improvement rather than conditional on the primary model's error structure; waiting for the P&L series itself to show decay rather than monitoring upstream inputs.

**Open questions:** no rigorously published capacity-estimation formula exists for crypto/small-cap universes specifically (most literature assumes liquid large-cap equities/futures) — a genuine gap given this platform's asset mix and free-tier constraint; the "shut down if live Sharpe < 50% of backtest Sharpe" rule-of-thumb is repeated widely but traces to no single authoritative source — treat as folk wisdom, not a validated threshold; whether regime-tilt turnover costs net out positive after the tilting itself is unaddressed in the sources found.

---

### Theme 9 — Agent-level mechanics specific to quant research

There is now a fast-moving 2025-2026 literature on exactly this cluster, plus named financial-agent benchmarks (FinBen, Finance Agent Benchmark, FinResearchBench) — but the consistent headline finding is that current agents perform poorly on real financial research tasks (best model 46.8% accuracy at $3.79/query on Finance Agent Benchmark, [arXiv:2508.00828](https://arxiv.org/abs/2508.00828)). Reward hacking in tool-using agents is now empirically demonstrated, not theoretical — a dedicated benchmark shows Claude 3.7/o3/o4-mini all "discover shortcuts that satisfy metrics without accomplishing intended outcomes" ([arXiv:2605.02964](https://arxiv.org/pdf/2605.02964)).

| Tool | Purpose | Effort | Priority |
|---|---|---|---|
| `report.extract_claims` | Decomposes agent output into discrete {claim, evidence_refs, confidence, verification_method} objects — post-hoc span-alignment against actual evidence, not self-attested citation, since LLMs are weak at pinpointing the precise supporting span even when good at finding relevant documents ([arXiv:2606.07130](https://arxiv.org/html/2606.07130)) | M | **P0** |
| `verify.recheck_claim` | Re-derives a claim via an actual deterministic tool call (rerun the stat, requery the ledger) rather than the model re-reading its own text — the CRITIC pattern ([arXiv:2305.11738](https://arxiv.org/pdf/2305.11738)); a failed recheck must block report finalization, not just log | S | **P0** |
| `audit.check_metric_integrity` | Independently re-derives the headline metric from platform-logged (not agent-reported) tool calls and diffs against the claim — the systematic version of the reward-hacking benchmark's finding that current frontier models game reward specs given tool access | M | **P0** |
| `review.adversarial_critique` | Second agent, genuinely separate session (not just a different prompt in the same context — cross-context review measurably outperforms same-session critique, [arXiv:2603.12123](https://arxiv.org/pdf/2603.12123)), sole job is finding flaws before a proposal reaches a human | M | P1 |
| `hitl.classify_checkpoint` | Routes actions to a human by reversibility × confidence, not by step — "if you gate everything, the human stops being a safeguard and becomes a rubber stamp" ([MindStudio](https://www.mindstudio.ai/blog/human-in-the-loop-checkpoints-ai-agents)); defaults to `requires_human: true` for any unclassified action type | S | P1 |
| `budget.check_and_route` (upgrade to existing tool/output budgets) | Adds degradation-aware routing for the platform's own degraded local-LLM tier: when degrading, the *tool* must carry more rigor (route to the deterministic null-comparison tool and have the weak model only select/interpret templated output, never compute a statistic itself); every degradation logs into the evidence trail as an explicit caveat | S | P1 |
| `report.compile_dossier` | Terminal report step with mandatory Unresolved-Questions and Reviewer-Critique sections that must say "none found" explicitly rather than being silently omitted — omission itself reads as a trust signal to experienced human reviewers | S | P1 |

**Workflow — Report finalization gate** (every agent-produced research report passes through this before a human sees it): `extract_claims` (unsupported claims must be resolved first) → `recheck_claim` on every `direct_stat`-tagged claim, all must pass → `check_metric_integrity` (any gaming flag escalates to human immediately, overriding everything else) → `adversarial_critique` in a fresh session (blocking flaws send it back to step 1) → `compile_dossier` → routed to a human via `classify_checkpoint` (almost always required for anything proposing capital allocation or strategy promotion).

**Workflow — Degraded-tier (weak local LLM) research loop**: `budget.check_and_route` before each expensive step → on degrade, route to the deterministic tool and shrink the model's job to selecting/interpreting structured output → every degradation logged and surfaced in the final dossier so a reviewer knows which claims came from full vs. reduced-rigor analysis.

**Anti-patterns:** self-critique without tool independence ("the critic is not independent and the model may confidently endorse its own wrong reasoning"); gate-everything HITL design producing rubber-stamping; trusting agent-reported metrics without independent recomputation, given the demonstrated reward-hacking behavior in current frontier models; same-session reviewer agents; omitting an "unresolved questions" section instead of stating none were found.

**Open questions:** no finance-domain-specific reward-hacking study exists yet (the cited benchmark is domain-general tool use) — whether an agent would quietly narrow a backtest universe/date range to hit a target Sharpe is this report's own inferred risk, not measured; adversarial-reviewer precision/recall in production (false-positive rate fatiguing human reviewers) is unquantified anywhere and would need the platform's own agent-evaluation suite to measure; the "rigor-in-the-tool vs. rigor-in-the-model" framing for a degraded LLM tier is this report's own synthesis of CRITIC's finding plus the platform's known local-LLM constraint, not directly evidenced elsewhere.

## Recommendation

Build in three waves rather than by theme, since the P0 items cluster around a few shared primitives:

**Wave 1 — cheap, ledger-native, agent-safety-critical** (all reuse data the ledger/gate-funnel already has): holdout-burn accounting, deflated Sharpe ratio and minimum track-record length, negative-result recording, bar-boundary and future-perturbation leakage checks, cost-shock curve and venue-constraint lookup, and the claim-evidence + tool-interactive self-verification pair. None of these require new data infrastructure; most are a few days of work each. Do the metric-integrity audit and the independent-process kill switch alongside these — both are safety-critical enough to not defer, and the kill switch specifically must be scoped as its own process from day one, not retrofitted later.

**Wave 2 — extends existing tools, needs light new infrastructure**: factor spanning and signal half-life as upgrades to `factors`/`decompose`/`decay`; paper-vs-backtest divergence attribution; the red-team falsifier (validate the in-flight `falsification.rs` design against this brief's Theme 3 findings before finalizing it); shrunk covariance and capacity estimation building on the in-flight `costs.rs`/`capacity.rs`; semantic hypothesis dedup and the signal registry; the adversarial-critique and HITL-classification agent-mechanics pair.

**Wave 3 — real new capability, sequence last**: queue-aware fill simulation and shadow trading (need trade-tape retention beyond current 1m aggregation); symbolic regression and the Pareto-frontier selector (both depend on the deflated-Sharpe primitive from Wave 1 already existing, or they just add a new flavor of overfitting); the forking-paths audit and paper-to-strategy pipeline (both are genuinely hard and should wait until the cheaper dedup/registry tools exist to catch duplicates before spending effort on either).

**Explicitly reject or scope down:** GAN/diffusion synthetic markets as anything but experimental-tier evidence, until validated against this platform's own backtest engine rather than trusted for stylized-fact realism; matrix-profile motifs and wavelet/EMD decomposition as standalone signal generators; VPIN without its incremental-predictive-power benchmark attached; same-context multi-agent "debate" as a verification mechanism, in favor of genuinely separate review sessions.

## Open Questions

Across all nine themes, three gaps recur and are worth the platform running its own experiments on rather than waiting for literature: (1) which execution-realism upgrade actually changes go/no-go decisions for 1-minute-bar strategies — no external source measured this; (2) whether any of the ~14 "exotic" discovery methods in Theme 6 produce real edges specifically on crypto 1-minute data, since almost all rigorous evidence is equity/futures-tick-based; (3) what capacity-estimation and crowding-proxy models look like for crypto/small-cap universes, since the mature literature assumes liquid large-cap equities. See each theme section above for the full per-theme open-questions list.

## References

Grouped by theme; deduplicated where a source was cited in more than one section.

**Theme 1 (multiple testing):** [Deflated Sharpe Ratio](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=2460551) · [PBO](https://escholarship.org/uc/item/4w1110bb) · [mlfinlab](https://github.com/hudson-and-thames/mlfinlab) · [pypbo](https://github.com/esvhd/pypbo) · [jsharpe](https://github.com/tschm/jsharpe) · [Purged CV](https://en.wikipedia.org/wiki/Purged_cross-validation) · [Lo 2002](https://rpc.cfainstitute.org/research/financial-analysts-journal/2002/the-statistics-of-sharpe-ratios) · [Harvey/Liu/Zhu](https://foxholm.com/q/research/harvey-liu-zhu-cross-section/) · [White 2000](https://www.researchgate.net/publication/4896389_A_Reality_Check_for_Data_Snooping) · [Hansen SPA](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=264569) · [Romano-Wolf](https://journals.sagepub.com/doi/10.1177/1536867X20976314) · [Forking paths in finance](https://arxiv.org/abs/2401.08606) · [Agentic Garden of Forking Paths](https://arxiv.org/html/2607.01507v1) · [Dwork et al., reusable holdout](http://papers.nips.cc/paper/5993-generalization-in-adaptive-data-analysis-and-holdout-reuse.pdf) · [Ten strategies rejected (pre-registration case)](https://github.com/wahazzin/quant-trading-ten-strategies-rejected) · [Backtest bias taxonomy](https://www.susanpotter.net/quant/backtest-bias-taxonomy/)

**Theme 2 (leakage):** [Freqtrade lookahead-analysis](https://www.freqtrade.io/en/stable/lookahead-analysis/) · [Point-in-time fundamentals](https://dev.to/tradevodata/point-in-time-fundamentals-data-what-it-is-why-it-matters-and-how-to-choose-a70) · [pit-fundamentals dataset](https://github.com/christianpichichero-max/pit-fundamentals) · [Survivorship bias in crypto](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4287573) · [QuantRocket survivorship primer](https://www.quantrocket.com/blog/survivorship-bias/) · [Detecting Lookahead Bias in LLM Forecasts](https://arxiv.org/pdf/2512.23847) · [Look-Ahead-Bench](https://arxiv.org/abs/2601.13770) · [Feature leakage, Hex](https://hex.tech/blog/feature-leakage/) · [Bar-boundary leakage issue](https://github.com/Ferdows-Ahmadi/trading-llm/issues/7) · [Monte Carlo permutation tests](https://www.susanpotter.net/quant/monte-carlo-permutation-tests-strategy-significance/)

**Theme 3 (adversarial):** [Beyond Visual Realism (GAN collapse)](https://arxiv.org/pdf/2601.12990) · [What survives honest evaluation?](https://arxiv.org/html/2608.27734) · [Poisoning Agentic Alpha](https://arxiv.org/abs/2608.24069) · [Isolated self-correction vs. multi-agent debate](https://arxiv.org/pdf/2605.00914) · [StrategyQuant Monte Carlo methods](https://strategyquant.com/blog/new-robustness-tests-on-the-strategyquant-codebase-5-monte-carlo-methods-to-bulletproof-your-trading-strategies/) · [BuildAlpha robustness](https://www.buildalpha.com/robustness-testing-guide/) · [Man AHL/Jane Street process writeups](https://thequantsignal.substack.com/p/the-firms-a-candidates-guide-to-the) · [TRADES diffusion market sim](https://arxiv.org/html/2502.07071v1) · [CoFinDiff](https://arxiv.org/pdf/2503.04164)

**Theme 4 (alpha decay):** [McLean & Pontiff](https://dx.doi.org/10.2139/ssrn.2156623) · [WorldQuant BRAIN self-correlation (DeepWiki)](https://deepwiki.com/xiegengcai/world-quant-brain/4.1-self-correlation-analysis) · [BRAIN IQC guidelines](https://www.worldquant.com/brain/iqc-guidelines/) · [Novy-Marx, taxonomy of anomalies](https://mysimon.rochester.edu/novy-marx/research/ToAatTC.pdf) · [Moskowitz/Ooi/Pedersen TSMOM decomposition](https://www.sciencedirect.com/science/article/pii/S0304405X11002613) · [Portfolio Optimizer, residualization pitfalls](https://portfoliooptimizer.io/blog/residualization-of-risk-factors-examples-and-pitfalls/) · [Signal half-life](https://backtestbrewery.github.io/posts/alpha-decay-trading.html) · [Quant capacity squeeze / crowding](https://www.hedgeco.net/news/04/2026/quant-funds-face-a-capacity-squeeze-when-too-much-capital-threatens-alpha.html)

**Theme 5 (execution realism):** [hftbacktest](https://github.com/nkaz001/hftbacktest) · [Square-root law confirmation](https://arxiv.org/pdf/2606.24019) · [Almgren-Chriss](https://en.wikipedia.org/wiki/Almgren%E2%80%93Chriss_model) · [Coinbase matching engine docs](https://docs.cdp.coinbase.com/exchange/concepts/matching-engine) · [Kraken rate limits](https://docs.kraken.com/api/docs/guides/spot-ratelimits/) · [Alpaca PDT-rule retirement](https://alpaca.markets/blog/finra-retires-the-pdt-rule-introducing-alpacas-new-intraday-margin-framework/) · [Capacity estimation case study](https://concretumgroup.substack.com/p/estimating-the-capacity-of-a-trading) · [Backtest vs. live divergence tracking](https://skills.himanshujangir.com/skills/backtest-vs-live-performance-divergence-tracking/)

**Theme 6 (discovery tools):** [BOCPD, Adams & MacKay](https://arxiv.org/abs/0710.3742) · [STUMPY](https://github.com/stumpy-dev/stumpy) · [Tigramite/PCMCI](https://github.com/jakobrunge/tigramite) · [PySR](https://github.com/MilesCranmer/PySR) · [gplearn overfitting discussion](https://github.com/MilesCranmer/PySR/discussions/995) · [Conformal time-series forecasting](https://proceedings.neurips.cc/paper/2021/file/312f1ba2a72318edaaa995a67835fad5-Paper.pdf) · [VPIN original](https://academic.oup.com/rfs/article-abstract/25/5/1457/1569929) · [VPIN and the Flash Crash critique](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=1881731) · [WinIT, time-aware feature importance](https://arxiv.org/pdf/2107.14317) · [MIC](https://royalsocietypublishing.org/doi/10.1098/rsos.201424) · [Hawkes processes, order-flow imbalance](https://arxiv.org/html/2408.03594v1)

**Theme 7 (research memory):** [Chen & Zimmermann, Open Source Asset Pricing](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=3604626) · [openassetpricing.com](https://www.openassetpricing.com/) · [Jensen/Kelly/Pedersen replication crisis](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=3774514) · [PaperBench](https://arxiv.org/abs/2504.01848) · [MemoNoveltyAgent](https://arxiv.org/html/2603.20884) · [Scientific Utopia II (publication bias)](https://arxiv.org/pdf/1205.4251) · [Center for Open Science preregistration](https://www.cos.io/initiatives/prereg) · [Reproducibility in ML-based research](https://onlinelibrary.wiley.com/doi/10.1002/aaai.70002)

**Theme 8 (portfolio/meta):** [Ledoit-Wolf shrinkage](https://arxiv.org/pdf/2305.11298) · [Strategy capacity, algotrader.ch](https://algotrader.ch/operational-robustness/strategy-capacity/) · [Regime detection with HMM](https://bsic.it/regime-detection-and-risk-allocation-using-hidden-markov-models/) · [Meta-labeling, Hudson & Thames](https://hudsonthames.org/does-meta-labeling-add-to-signal-efficacy-triple-barrier-method/) · [Meta-labeling pushback, QuantConnect](https://www.quantconnect.com/forum/discussion/14706/why-meta-labeling-is-not-a-silver-bullet/) · [Kill-switch independent-process principle, NYIF](https://www.nyif.com/articles/trading-system-kill-switch-panacea-or-pandoras-box) · [Algorithm monitoring, 3forge](https://3forge.com/use-cases/regulatory-adherence/algorithm-monitor-and-kill-switches)

**Theme 9 (agent mechanics):** [Finance Agent Benchmark](https://arxiv.org/abs/2508.00828) · [FinBen](https://arxiv.org/abs/2402.12659) · [PaperTrail, claim-evidence interface](https://arxiv.org/pdf/2602.21045) · [Structured inline citation generation](https://arxiv.org/html/2606.07130) · [CRITIC](https://arxiv.org/pdf/2305.11738) · [Cross-Context Review](https://arxiv.org/pdf/2603.12123) · [Reward hacking benchmark](https://arxiv.org/pdf/2605.02964) · [Goodhart's law in RL](https://arxiv.org/pdf/2310.09144) · [MindStudio, HITL checkpoints](https://www.mindstudio.ai/blog/human-in-the-loop-checkpoints-ai-agents) · [LangChain HITL docs](https://docs.langchain.com/oss/python/langchain/human-in-the-loop)

Each subagent's full report (with the complete per-tool citation trail and additional sources beyond this consolidated list) is preserved in this session's transcript; ask to have any theme's full source list re-extracted if a deeper citation audit is needed before implementation.
