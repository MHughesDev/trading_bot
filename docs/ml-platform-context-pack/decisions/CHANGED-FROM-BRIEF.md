# Evidence-backed reversals

Sixteen places where the spec reverses the obvious or initially-recommended answer. **These are the highest-value pages in the pack.** If you are going to skim anything, skim this — each one is a mistake you would otherwise make, with the evidence for why.

---

**R-01 · Bitemporal is four timestamps, not two.**
*Initially:* `event_time` + `knowledge_time`.
*Corrected:* also `venue_ts` and `ingest_time`. And **Iceberg/Delta snapshots are not a bitemporal model** — compaction creates new snapshots with new commit times while preserving old rows, so the snapshot timeline stops mapping 1:1 to knowledge events.
*Why:* a batch that arrived 09:00 but committed 11:30 was not actionable at 10:00. Snapshot time travel is for artifact pinning, not PIT semantics.
→ `reference/08-market-data-arch.md` · spec §0.1, §1.2

**R-02 · Use EDGE for spread estimation. Retire Roll and Corwin–Schultz.**
*Evidence:* at 0.50% true spread, RMSE **EDGE 0.38% vs CS 0.57%, AR 0.83%, Roll 1.72%**. Against TAQ 1993–2020: EDGE 1.22% vs CS 2.08%; correlation 76.5% vs 66.9%; non-positive estimates 5% vs 20–30%. Validated at minute level; unbiased under infrequent trading.
*Why it matters here:* illiquid crypto and long-dated options are exactly the infrequent-trading case.
→ `reference/05-asset-embeddings.md` · spec §5.2

**R-03 · Reference portfolios, not "landmarkers."**
*Initially:* performance landmarkers as task representation.
*Corrected:* a greedy submodular-selected **complementary portfolio** grown against your own ledger. Auto-sklearn 2.0 *deliberately deleted* meta-features (including landmarkers); TabRepo shows a 3-config portfolio beats most AutoML systems and 15 beats full AutoGluon, saturating ~150.
*Caution carried forward:* a 2025 benchmark found catch22 specifically weak for algorithm selection — which is our task — so the portfolio scores, not the statistics, carry the routing signal.
→ `reference/02-hpo-search.md`, `reference/05-asset-embeddings.md` · spec §5.2

**R-04 · HRP/HERC is a taxonomy, not an allocator.**
*Evidence:* 2011–2025, US + Brazil, 756-day windows — minimum-variance had the lowest realized volatility in both markets; hierarchical methods had higher turnover; authors find "no evidence" of outperformance.
*Use:* define the Outcome Tensor's row space. Do not allocate with it.
→ `reference/05-asset-embeddings.md` · spec §5.5

**R-05 · MNAR joint-likelihood over plain IPW.**
*Evidence:* exponential-family CP tensor completion with `P = logit⁻¹(b₀+b₁X)` has bounds valid for propensities arbitrarily near 0/1, plus a sample-split test of `H₀: b₁=0`.
*Action:* ship `b₁` as a platform health metric — quantitative proof of whether the naive tensor is biased, and when the correction stops mattering.
→ `reference/05-asset-embeddings.md` · spec §5.5

**R-06 · Off-policy config selection is actively dangerous. LCB-rank and shrink, non-bypassably.**
*Evidence:* selecting hyperparameters by maximizing an IPS/DR estimate on logged data can pick policies **~15% worse than the logging policy you started from.**
*Action:* `score(c) = LCB_α(DR_estimate(c))`, shrunk toward default by a significance-tested weight. **No expert mode that returns raw point estimates.**
*This is the single most important safety rule in the knowledge plane.*
→ `reference/05-asset-embeddings.md` · spec §5.5

**R-07 · Regimes: filtered probabilities only, enforced by GRANT.**
*Evidence:* controlled lookahead ladder on a 2-state HMM — **Sharpe 0.78 filtered, 0.77 with full-sample parameters, 1.74 with smoothed.** A 2.2× inflation from non-causal state inference alone; parameter lookahead was nearly harmless.
*Action:* smoothed/Viterbi live in `regime_research`, on which the backtest role has no GRANT. A reviewer misses this once; a missing grant never does.
→ `reference/05-asset-embeddings.md` · spec §5.4

**R-08 · Store implied volatility. Never store greeks.**
*Why:* greeks are deterministic functions of (IV, S, K, T, r, q) — storing them freezes one model choice into your data forever and multiplies width by five. IV is *not* cheaply reproducible later because it depends on rate and dividend curves as known at that moment.
→ `reference/08-market-data-arch.md` · spec §1.6

**R-09 · Forward-adjusted ratio is the default continuous futures method.**
*Why:* back-adjusted series are not prices — their entire history changes at every roll, and Panama/additive adjustment can go negative, breaking log returns. Forward adjustment leaves history immutable, so a 2026 backtest still reproduces bit-for-bit in 2031.
*Also:* roll rules using open interest or volume have publication lag. Rolling on same-day OI is look-ahead — small, common, and extremely profitable-looking.
→ `reference/08-market-data-arch.md` · spec §1.5

**R-10 · The transfer metric is (instrument family, venue), not asset similarity.**
*Evidence:* 3.4M minute observations, 6 crypto pairs, Binance spot + perp — models transfer well between spot and futures of the *same* asset, **not across assets**.
*Action:* venue is a stored first-class coordinate, not something an embedding is expected to rediscover.
*Confidence:* partially supported; one study. Flag and measure.
→ `reference/05-asset-embeddings.md` · spec §1.1, §5.3

**R-11 · Do not trigger retraining on distribution drift. Use the learning-debt rule.**
*Evidence:* drift-triggered methods "recommend retraining far too often" when retraining is costly (ADWIN-5% cost 3.27 vs oracle 2.68). PSI's 0.1/0.25 thresholds are sample-size-dependent folklore.
*Rule:* retrain iff `ρ_t > c_churn/(c_churn + c_wait)`. Beat calendar retraining in 24/24 gradual-drift cells; **0.36× the excess loss** of semi-annual retraining over a 104-week backtest; triggered 16 weeks early after a policy shock.
*Caveat from the same work:* monitoring-dashboard proxies performed poorly. Dashboards are not a retraining signal.
→ `reference/07-finetuning-drift.md` · spec §14.1

**R-12 · CBPE cannot gate alpha-adjacent models.**
*Why:* label-free performance estimation explicitly **assumes no concept drift**. It answers "the market looks different but your model should still work." It cannot answer "your edge is gone" — which in trading is the entire question.
*Use:* M1, M2, M3, M11 only.
→ `reference/07-finetuning-drift.md` · spec §14.2

**R-13 · Cross-tenant meta-learning on strategy content: no. And DP/FL do not fix it.**
*Why:* a global strategy recommender is a mechanism for broadcasting one tenant's edge to every other tenant. Modelled harm includes compressing signal half-life from ~58 to ~18 months. **FL protects raw data while your risk is that the learned function transfers competitive information** — it defends the wrong thing. DP costs a measured 5–10 accuracy points at ε=6.
*Instead:* `info_class` schema whitelist + a build-blocking CI test. Also makes per-tenant deletion architecturally true rather than aspirational.
→ `reference/07-finetuning-drift.md`, `reference/08-market-data-arch.md` · spec §7.2–7.4

**R-14 · Exact kNN in pgvector with no ANN index.**
*Why:* 10⁵–10⁷ vectors at 64-d is 25 MB–2.5 GB; a filtered exact scan is ~1 ms in BLAS. Faster, exact, and it closes a cross-tenant approximate-neighbor side channel. Add HNSW only when measured p99 fails.
*Related:* retrieve at 48 whitened dims. Distance concentration is survivable; **hubness is the real problem** — apply mutual-proximity reduction.
→ `reference/05-asset-embeddings.md` · spec §5.3, §6.1

**R-15 · Public tool-use benchmarks cannot gate releases. Build a private harness.**
*Evidence:* expert audit of BFCL v4 / τ²-Bench / LiveMCPBench / MCP-Atlas found **18.5% of official labels disagree with expert judgment**; one LLM-judge harness scored the identical setup between **57.9% and 76.8% across 23 repeats**.
*Related:* LLM-judge raw agreement overstates chance-corrected κ by 33.8–41.2 points — an "85% agreement" judge is κ ≈ 0.48. Judges triage; gates decide.
→ `reference/04-agentic-ml.md`, `reference/07-finetuning-drift.md` · spec §14.5

**R-16 · Fine-tune later, but instrument now — and the trigger is measurable.**
*Break-even:* cached frontier executor call ≈ $0.019; self-hosted 8B+LoRA ≈ $0.0012, but the H100 is $1,825/month regardless. GPU alone: ~3,400 calls/day. GPU + the 0.5–1.0 FTE a fine-tuning program consumes: **~100k executor-calls/day sustained.**
*Structural risk:* a fine-tuned executor is a cache of your API surface. **Measure trajectory-corpus half-life from `tool_schema_hash` churn.** Shorter than your training cadence ⇒ structurally unprofitable, and no GPU budget fixes it.
*Override conditions:* determinism/reproducibility requirement, or a tenant contractually forbidding payloads leaving your VPC.
*When it happens:* on-policy distillation, not SFT or RL — 1,800 GPU-hours → 74.4% AIME'24 vs RL's 17,920 → 67.6%.
→ `reference/07-finetuning-drift.md` · spec §14.6

---

## Two findings that reorganize priorities rather than reversing a decision

**P-01 · The bottleneck is selection, not search.**
With 10,000 trials over 5 years, the Sharpe **expected from pure noise is 1.92**. Separately, Meta's AIRA study measured that selecting the final candidate by test score rather than validation score was worth **9–13 percentage points** — larger than any search-algorithm improvement they found, and top-3 instead of top-1 recovered ~10%.
*Implication:* engineering spent on trial accounting, deflation and the gate stack outperforms engineering spent on search algorithms.

**P-02 · Operators dominate search policy.**
AIRA's ablation: MCTS and evolutionary search gave **zero gain** over greedy iteration when paired with a generic operator set; tuning the exploration constant moved nothing. Only after operators improved did search policy begin to pay.
*Implication:* invest in *what changes an agent can make and how surgically* (ablation-guided single-block edits), not in a cleverer tree search.
