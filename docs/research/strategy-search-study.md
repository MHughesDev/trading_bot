# Strategy Search: how the research agent should find, change, and tune algorithms

Status: study / planning input. Not an ADR yet.
Date: 2026-09-08

This is a literature-grounded study of one question: **how should an agent
search the space of trading strategies efficiently, change a strategy's
*structure* in response to backtest results, tune its parameters, and avoid
fooling itself?** It ends with a proposed architecture mapped onto the crates
we already have, and a short list of decisions to make before writing code.

---

## 0. Framing: "optimal" over what?

The space described — infinite strategies x infinite parameters — is real,
but it is not uniform, and treating it as one search problem is the classic
mistake. It factors into three axes with very different character:

| Axis | Nature | Right tool | Wrong tool |
|---|---|---|---|
| **Structure** — which blocks, which logic, which filters | Discrete, combinatorial, needs *ideas* | LLM as hypothesis generator inside an evolutionary loop | Grid search (there is no grid), gradient (no gradient) |
| **Parameters** — the numbers on the blocks | Continuous/ordinal, numeric | Bayesian optimisation, CMA-ES, random search, multi-fidelity racing | An LLM (expensive, poorly calibrated, non-reproducible) |
| **Context** — *when* a structure works | A property of the market, not the strategy | Unsupervised regime detection; regime-to-strategy map learned from backtests | Supervised "predict the right algo" (no ground truth exists) |

Everything below is organised around keeping those three separate and letting
each use its own machinery. The efficiency win is almost entirely from
**never spending an LLM token on something a numerical optimiser or a lookup
table can do**, and from **killing bad candidates early on cheap evaluations**.

---

## 1. Parameter optimisation (the inner loop)

### 1.1 Search algorithms

- **Grid** is only sane for <=2 parameters. Three parameters at ten values is
  1,000 backtests; five is 100,000. Beyond that the probability of a
  chance "great" result approaches certainty (see 1.4).
- **Random search** beats grid at equal budget for almost all real objective
  surfaces because most parameters barely matter and random samples cover the
  important ones densely. Cheap, parallel, embarrassingly simple. Good default.
- **Bayesian optimisation** (GP or TPE, i.e. Optuna-style) builds a surrogate
  of the objective surface and samples where expected improvement is highest.
  Wins when each evaluation is expensive (a long backtest) and the dimension is
  <= ~20. This is the sweet spot for strategy parameters.
- **CMA-ES** for continuous parameters with a noisy objective — robust, no
  surrogate to fit, handles the ruggedness of backtest surfaces well.
- **DEHB / evolutionary Hyperband** (arXiv 2105.09821) combines differential
  evolution with successive halving — a strong, cheap, parallel default that
  needs no surrogate model.

### 1.2 Multi-fidelity: the biggest compute win available

**Successive halving** samples N configurations, evaluates all at the
*cheapest* fidelity, keeps the top 1/eta, and re-evaluates survivors at eta x
the budget, repeating. **Hyperband** hedges across several starting fidelities
so it is provably at most a constant slower than random search while usually
being far faster.

For backtests, "fidelity" maps naturally onto things we already have:

1. **Timeframe** — 1h bars first, 15m for survivors, 1m for finalists. (The
   current agent prompt already hints at this; it should be a mechanism, not a
   suggestion.)
2. **Window length** — last 90 days first, full history for survivors.
3. **Simulator fidelity** — simplified fills first, full Nautilus engine last.

A candidate that loses money on 90 days of 1h bars does not need an hour-long
1m simulation to confirm it. This is the single largest lever on
"backtests per insight".

### 1.3 The parameter surface matters more than the optimum

A sweep must return the **surface**, not just the argmax. An optimum where
`period=21` scores 2.1 but `20` and `22` score 0.4 is noise. One where 15-30
all score ~1.8 is a real effect. Robustness lives in the neighbourhood.

Practical rule: the objective reported for a parameter set is its **local
average over neighbours** (a smoothed surface), not its point value. This
single change removes most spike-chasing. Set J already encodes this:
`StudyKind::Neighborhood` perturbs a parameter +/-k steps, `is_plateau` tests
dispersion/median < 0.5, and `SelectionRule::MedianStableCentroid` carries
forward the centroid of the stable region rather than the peak (INV-2). The LLM should see a compact
description of the surface ("plateau 15-30, cliff at 35") so it can reason
about robustness rather than fixate on one number.

### 1.4 Validation: this is where strategies die, and where they should

- **Walk-forward** (rolling or anchored): optimise on window k, test on k+1,
  roll. Industry standard for realistic simulation; produces one path.
- **Combinatorial Purged Cross-Validation (CPCV)** (Lopez de Prado): split
  history into N groups, form all train/test combinations, purge overlapping
  samples and add an embargo. Produces a **distribution** of out-of-sample
  outcomes rather than a single number. A 2024 controlled-synthetic comparison
  found CPCV markedly better than walk-forward and k-fold at *detecting*
  overfitting — lower Probability of Backtest Overfitting, stronger Deflated
  Sharpe statistics. Recommendation from that literature: walk-forward for the
  realistic simulation; CPCV for the go/no-go decision.
- **Probability of Backtest Overfitting (PBO)** via CSCV: what fraction of
  train-optimal configurations rank below median out-of-sample. If PBO is high
  the *search process* is overfitting regardless of the winner's score.
- **Deflated Sharpe Ratio (DSR)**: corrects the best Sharpe for the number of
  trials, their variance, non-normality and sample length. **Every search run
  must log the number of trials** — DSR is meaningless without it. This is a
  first-class field on an Experiment, not a note.
- **Minimum trade count** as a hard constraint. Four trades at 100% win rate
  is a coin that landed heads twice.
- **Objective choice.** Win rate is trivially gamed (tiny take-profit, huge
  stop-loss -> 90% wins, negative expectancy). Prefer Sortino, Calmar, profit
  factor or expectancy as the primary, with drawdown and trade-count as
  constraints. Objectives should be *composable data*, not prose.

**Structural rule for the agent:** it optimises on the train fidelity/window
and *cannot read* the held-out score until it commits a candidate. Set J's
Funnel is exactly this gate; the agent must go through it, not around it.

---

## 2. Structural search (the outer loop): how an agent *changes* an algorithm

### 2.1 The cautionary history

Genetic programming over trading rules is old. Allen & Karjalainen (1999)
evolved technical rules on S&P 500 data and found in-sample profits that
**did not survive transaction costs out of sample**. Neely, Weller & Dittmar
had similar FX results. The later "Failure of GP-induced trading strategies"
literature asks whether that reflects efficient markets or inefficient
algorithms; the honest answer is "mostly the former, and the search made it
worse". Two lessons carry forward:

1. Pure blind mutation over a rule grammar finds noise as readily as signal.
2. Costs and multiple-testing corrections must be inside the fitness function,
   not applied afterwards.

### 2.2 The current state of the art: LLM as hypothesis-driven mutation operator

**FunSearch -> AlphaEvolve** (DeepMind) re-established evolutionary program
search by replacing random mutation with an LLM that reads high-performing
programs and proposes edits, inside an island model with a MAP-Elites-style
archive for diversity. The LLM supplies *direction*; evolution supplies
*selection*.

**QuantEvolve** (arXiv 2510.18569, Oct 2025) applies this to trading strategy
discovery and is the closest published analogue to what we want:

- A strategy is a tuple *(hypothesis, code, metrics, analysis)*.
- **Islands** seeded by strategy category plus buy-and-hold; top 10% migrate
  every M generations.
- A **feature map** (MAP-Elites) over investor-relevant dimensions — category,
  trading frequency, Sharpe, Sortino, max drawdown, return — keeps one elite
  per cell. This is what prevents collapse onto a single idea. Their ablation:
  16 bins sustained improvement; 1-4 bins converged prematurely; adding
  *category* as a dimension cut single-category dominance from 46%.
- Per generation: sample a parent (exploit/explore balance), sample
  "cousins" (best, feature-space neighbours, random), a **Research Agent**
  writes a hypothesis from that context plus curated insights, a coding team
  implements and backtests, an evaluation team scores hypothesis quality and
  code fidelity, the child replaces the cell incumbent if better.
- Every K generations **insights are curated and de-duplicated** — an explicit
  memory step.
- Fitness: equal-weight Sharpe + information ratio + drawdown on a
  train/val/test split.
- Stated limitations they themselves flag: robustness testing is thin,
  data-snooping bias is not controlled, and it is unclear whether hypotheses
  are genuine theories or post-hoc rationalisation. 5-10 LLM inferences per
  cycle bound throughput.

**RD-Agent(Q)** (Microsoft, NeurIPS 2025) adds three ideas worth stealing:

- A **knowledge forest** of every prior experiment feeds hypothesis
  generation, with *adaptive complexity* — simpler proposals after a run of
  failures, bolder ones after successes.
- **De-duplication by information coefficient**: a new factor correlated
  >= 0.99 with an existing one is discarded before it costs a backtest. Our
  analogue: de-duplicate candidate strategies by signal-series correlation.
- A **multi-armed bandit scheduler** (Thompson sampling over a Bayesian linear
  model on an 8-dim performance vector) decides *what kind of work to do next*
  (factor vs model). Reported cost: under $10 per optimisation cycle.

**Compute allocation as a bandit** (arXiv 2605.29268, 2026) formalises the
depth-vs-breadth question — polish one candidate further, or spawn new ones —
as a bandit over candidates, allocating LLM calls by observed improvement
rate. Practical rules: never allocate uniformly; track return-per-call;
reallocate toward improving candidates; reserve a fixed exploration budget.

### 2.3 What the agent needs to *see* in order to plan a change

A summary Sharpe tells the model nothing about *what to change*. Human quants
change strategies in response to **diagnostics**. The agent should get, per
backtest, a compact diagnostic bundle:

- Trade-level: distribution of P&L per trade, average win/loss, holding time,
  MAE/MFE (how far trades went against/for before exit).
- Time-sliced: performance by month, by regime (see section 3), by
  hour-of-day if intraday.
- Failure attribution: the ten worst trades and what the signals looked like
  at entry; longest drawdown and what the market was doing.
- Exposure: time in market, turnover, cost drag as a fraction of gross.
- Parameter surface summary from the inner loop (1.3).

With that, "the strategy loses in high-vol chop; add a volatility filter" is a
*reasoned* edit. Without it, the LLM is guessing. This diagnostic bundle is
the most important new tool output in the whole design.

### 2.4 Structural moves, made explicit

Giving the LLM a vocabulary of legal edits both constrains it and makes its
proposals machine-checkable. A reasonable initial move set over our
strategy-definition graph:

`add_filter`, `remove_filter`, `swap_indicator`, `change_exit_logic`,
`add_regime_gate`, `change_position_sizing`, `combine(strategy_a, strategy_b)`
(crossover), `simplify` (drop the least-contributing node — measured by
ablation backtest).

`simplify` deserves emphasis: complexity is overfitting's friend. A move that
removes a node and *keeps* the score is a strict improvement.

---

## 3. "Categorisation of curves": regimes, and whether ML is worth it

What was described — classify price behaviour, then match behaviour to the
algorithms most likely to work — is **regime detection followed by
regime-conditional strategy selection**. Both halves have mature literature.
The key design insight is that the two halves are learned *differently*.

### 3.1 Detecting regimes (unsupervised)

- **Hidden Markov Models** (Hamilton 1989 onward): latent states (e.g. low-vol
  trend, high-vol chop, crash) emit observed returns; transitions form a
  Markov chain. Standard, interpretable, cheap.
- **Statistical jump models** (Nystrup et al.): cluster temporal features with
  an explicit penalty on state changes, yielding persistent regimes rather
  than the flickering HMMs can produce. Well suited to trading, where a regime
  that changes every bar is useless.
- **Clustering on hand-crafted windows**: k-means / GMM / spectral clustering
  over windowed features. Features with real content:
  - **Hurst exponent** — H < 0.5 mean-reverting, ~0.5 random walk, > 0.5
    trending. Directly tells you which *family* of strategy has a chance.
  - **Variance-ratio test** (Lo-MacKinlay) — same question, different lens.
  - Realised volatility and vol-of-vol; ADX/trend strength; return
    autocorrelation at several lags; skew/kurtosis of returns; volume regime.
- **Change-point detection** (BOCPD, PELT) to find *when* regimes shift.
- **Learned representations** — TS2Vec (contrastive, multi-scale) or ROCKET
  (random convolutional kernels) turn a window into a vector for clustering.
  Useful when hand features miss something; but on noisy financial data,
  hand-crafted regime features frequently match or beat learned ones out of
  sample, at a fraction of the complexity. Start with hand features.

### 3.2 Mapping regime -> strategy (learned from backtests, not supervised)

There is **no ground-truth label** for "the right algorithm for this curve".
The only signal is backtest performance itself. So the mapping is not a
supervised classifier; it is a **contextual bandit / meta-learning** problem:
context = regime features, arms = strategy families (or specific vaulted
strategies), reward = out-of-sample performance in that regime. Every
backtest the platform runs, sliced by regime (2.3), is a training example for
this table for free.

What this buys, in order of value:

1. **A prior for structural proposals.** "This instrument spends 60% of its
   time in H~0.4 chop; mean-reversion families have historically scored best
   there" is exactly the kind of context that turns the LLM's first hypothesis
   from generic into targeted. It cuts wasted generations.
2. **Regime-conditional evaluation.** A strategy that wins only in trends is a
   different object from one that wins everywhere. Reporting score-by-regime
   makes that visible instead of averaging it away.
3. **Live gating later.** The same detector, run online, can arm/disarm
   strategies by regime. Out of scope for the research agent but it is the
   same component.

### 3.3 Verdict on "is ML worth it?"

- **Yes** for regime *features* and *detection* (unsupervised, cheap,
  interpretable — start with HMM/jump model on Hurst, VR, vol, ADX).
- **Yes** for the regime-to-strategy *prior*, learned from our own backtest
  corpus as a bandit table.
- **No** for replacing the search loop with a model that "predicts the best
  strategy" end-to-end, and **no** (for now) for deep nets predicting price
  direction from OHLCV — the signal-to-noise ratio is too low and the
  literature's honest results are poor. The model-training suite we already
  have is for *forecast features that strategies consume*, which is a
  different and sounder use.

---

## 4. Proposed architecture for this platform

Five components, most of which map onto crates that already exist.

```
              +----------------------------------------------+
              |  Knowledge Base                               |
              |  hypothesis ledger . feature-map archive .    |
              |  regime->strategy table . trial counts (DSR)  |
              +------------^---------------------^-----------+
                           | reads                | writes
+--------------+   +-------+--------+    +--------+---------+   +----------------+
| Regime Engine|-->| Research       |--->| Sweep Engine     |-->| Honest         |
| HMM/jump on  |   | Campaign       |    | inner numerical  |   | Evaluation     |
| Hurst,VR,vol |   | outer loop:    |    | loop: BO/CMA-ES  |   | (Set J): CPCV, |
| per instr.   |   | LLM proposes   |<---| + multi-fidelity |<--| nulls, DSR,    |
+--------------+   | structural     |    | racing; returns  |   | funnel, vault  |
                   | moves; islands |    | surface not point|   +----------------+
                   | + MAP-Elites   |    +------------------+
                   +----------------+
```

| Component | Lives in / builds on | New work |
|---|---|---|
| Regime Engine | new module in `crates/features` or `crates/backtest/stats` | feature extraction, HMM/jump fit, per-bar regime label stored alongside bars |
| Research Campaign | `crates/api/src/agent/` (driver stays) | campaign = many runs; island/feature-map state; move vocabulary; bandit compute allocation |
| Sweep Engine | `crates/backtest` manager (fan-out already exists) | typed parameter spaces on strategy nodes; BO/random/CMA-ES; successive halving over timeframe/window; surface summariser |
| Honest Evaluation | **Set J as-is** (`experiment/`, `nulls/`, `gates/`, `stats/`) | expose to the agent as tools — nothing else. Verified in code: `StudyKind::{Cpcv, WalkForward, Neighborhood, RegimeConditional}`, `deflated_sharpe_ratio`, `probability_of_backtest_overfitting`, an automatic irreversible trial counter on `Experiment`, and `SelectionRule::{MedianStableCentroid, WorstCaseRobust}` (never argmax) all already exist |
| Knowledge Base | Postgres (`agent_runs` pattern) | hypothesis ledger, elite archive, regime->strategy table, insight curation |
| Diagnostics | `crates/backtest/stats` | the per-backtest diagnostic bundle (2.3) as a single tool response |

The existing driver's mechanics — loopback tools, zero-token waits, budget
gates, persisted transcript — are all reused unchanged. What changes is the
*protocol* the LLM plays: it proposes structural moves and parameter ranges,
reads diagnostics and surfaces, and never sets a number itself.

### 4.1 Efficiency, defined

"Most efficiently optimised" should be measured, not asserted. Proposed
metrics per campaign:

- **Backtests per accepted improvement** (lower is better).
- **LLM tokens per accepted improvement**.
- **Fraction of backtests killed at low fidelity** (higher is better — it
  means racing is working).
- **PBO of the campaign** (must stay low — otherwise the search is efficient at
  finding noise).
- **Feature-map coverage** (diversity; guards against collapse).

---

## 5. Decisions to make before code

1. **Parameter rule.** May the LLM ever set a numeric parameter directly?
   Recommendation: **no** — it proposes ranges; the sweep engine chooses.
2. **Objective as data.** Primary metric + constraints (drawdown, trade count,
   turnover) as a JSON object attached to the Experiment. Which primary
   metrics do we support at launch? Recommendation: Sortino, Calmar, profit
   factor, expectancy. Win rate available but never primary.
3. **Validation gate.** Walk-forward for the realistic path, CPCV for the
   go/no-go. Both exist in Set J; Gate 3 already enforces DSR >= 0.95 and
   PBO <= 0.5. Nothing to add — only to route the agent through it.
4. **Fidelity ladder.** Concrete tiers (e.g. 1h/90d -> 15m/1y -> 1m/full) and
   the halving ratio eta.
5. **Move vocabulary** for structural edits (2.4) — the initial list, and
   whether `combine` (crossover) is in v1.
6. **Feature-map dimensions** for the elite archive. QuantEvolve's set
   (category, frequency, Sharpe, Sortino, drawdown, return) is a fine start;
   add *regime-of-best-performance*.
7. **Regime features v1.** Hurst, variance ratio, realised vol, ADX,
   autocorr(1,5,20). HMM vs jump model: start with jump model for persistence.
8. **Budget denomination.** Backtests + wall clock, not LLM turns.

### Suggested phasing

- **Phase 1 — inner loop + honesty.** Typed parameter spaces; sweep engine
  with random/BO + successive halving; surface summariser; expose Set J to
  the agent; trial counting -> DSR; diagnostic bundle tool.
- **Phase 2 — outer loop.** Move vocabulary; hypothesis ledger; feature-map
  archive with islands; bandit compute allocation; insight curation.
- **Phase 3 — context.** Regime engine; regime-sliced diagnostics;
  regime->strategy table as prior; campaign-level efficiency metrics.

---

## Sources

- QuantEvolve: multi-agent evolutionary strategy discovery — https://arxiv.org/html/2510.18569
- RD-Agent(Q) walkthrough — https://saulius.io/blog/automated-quant-research-ai-agents-rd-agent
- Automate Strategy Finding with LLM in Quant Investment — https://arxiv.org/html/2409.06289v4
- Compute Allocation in Evolutionary Search: Depth-Breadth to Bandits — https://arxiv.org/pdf/2605.29268
- LLM-Driven Evolutionary Program Search: FunSearch to Scientific Discovery — https://www.researchgate.net/publication/407540701
- Failure of GP-Induced Trading Strategies — https://link.springer.com/chapter/10.1007/978-3-540-72821-4_11
- Maximizing the Sharpe Ratio: A GP Approach — https://www.acem.sjtu.edu.cn/sffs/2020/pdf/paper3.pdf
- Deflated Sharpe Ratio — https://en.wikipedia.org/wiki/Deflated_Sharpe_ratio
- Purged / Combinatorial Purged CV — https://en.wikipedia.org/wiki/Purged_cross-validation , https://towardsai.com/p/l/the-combinatorial-purged-cross-validation-method
- Backtest overfitting: comparison of OOS methods in a synthetic environment — https://www.sciencedirect.com/science/article/abs/pii/S0950705124011110
- Walk-forward: anchored vs rolling — https://www.susanpotter.net/quant/walk-forward-optimization/
- DEHB: Evolutionary Hyperband — https://arxiv.org/pdf/2105.09821
- Market regime detection with HMMs — https://questdb.com/glossary/market-regime-detection-using-hidden-markov-models/ , https://www.quantstart.com/articles/market-regime-detection-using-hidden-markov-models-in-qstrader/
- Dynamic factor allocation with regime-switching signals — https://arxiv.org/pdf/2410.14841
- Hurst exponent for time-series classification — https://iopscience.iop.org/article/10.1088/1742-6596/1328/1/012056
- TS2Vec — https://liner.com/review/ts2vec-towards-universal-representation-time-series
- Meta-learning the optimal mixture of strategies — https://arxiv.org/html/2505.03659v1
