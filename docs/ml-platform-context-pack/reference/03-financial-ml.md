# Rigorous Financial ML & Backtest Validity

**Scope:** the safeguards that stop an automated research platform from fooling itself.
**Audience:** platform engineers building gates that AI agents cannot argue their way past.
**Date:** September 2026. Literature emphasis 2024–2026.

---

## 0. Executive summary — the gate stack

The central fact: **an AI research agent is a multiple-testing machine.** A human quant runs maybe
10²–10³ configurations a year; an agent loop runs that before lunch. Every classical safeguard
(Sharpe > 1, positive out-of-sample, "it survived 2008") was calibrated for the human trial rate and
is worthless at agent trial rates. The platform's job is therefore **not** to find good strategies —
it is to *count trials honestly* and *deflate accordingly*, and to make the deflation
non-negotiable by putting it in the infrastructure rather than the researcher's discretion.

Design principle: **the number of trials must be measured by the platform, not self-reported by the
agent.** Every backtest execution — including "quick checks", failed runs, and parameter sweeps —
increments a counter attached to the strategy lineage. This single piece of bookkeeping does more
for validity than every statistical test in this document combined.

### Recommended promotion gate stack (defaults; tune per asset class)

Candidate must pass **every** stage. Stages are ordered cheapest-first so most candidates die early.

| # | Gate | Default threshold | Rationale |
|---|---|---|---|
| G0 | **Pre-registration** | Hypothesis + feature set + horizon + universe + cost model registered *before* first backtest; hash-locked | Kills post-hoc narrative; makes trial counting possible |
| G1 | **Leakage suite** | 100% of automated leakage tests pass (§3.9) | Non-statistical; a single failure invalidates everything downstream |
| G2 | **Cost sensitivity** | Sharpe at 3× modelled costs ≥ 0.5 × Sharpe at 1× costs; break-even cost ≥ 3× modelled cost | Cost-fragile strategies are the #1 live-failure mode |
| G3 | **Capacity** | Target AUM ≤ 20% of capacity-at-half-Sharpe; daily trading ≤ 5% ADV per name (10% hard cap) | Impact is convex; capacity gates prevent "paper alpha" |
| G4 | **CPCV / purged CV** | Median path Sharpe > 0; **5th-percentile path Sharpe > 0**; IQR of path Sharpe < 1.0 | Distribution of outcomes, not point estimate |
| G5 | **PBO (CSCV)** | **PBO < 0.20** (hard reject ≥ 0.50) | Bailey et al.; direct overfit probability |
| G6 | **Deflated Sharpe** | **DSR ≥ 0.95** using platform-counted `N_eff` trials | The multiple-testing haircut |
| G7 | **Min backtest length** | `T ≥ MinBTL(N)`; and `T ≥ 5 yr` **and** `≥ 300 independent events` | Prevents "short backtest + many trials" |
| G8 | **Factor attribution** | Annualized alpha t-stat ≥ **3.0** vs. FF5+MOM+STR+BAB+QMJ; |β_mkt| ≤ 0.3 for market-neutral claims; R² of factor regression < 0.7 | Ensures it isn't repackaged beta |
| G9 | **Regime coverage** | ≥ 3 distinct volatility regimes and ≥ 2 crisis windows in OOS; **no single regime contributes > 50% of total PnL**; worst-regime Sharpe > −0.5 | Prevents one-regime wonders |
| G10 | **Perturbation robustness** | Median Sharpe over ±20% parameter perturbations ≥ 0.7 × center Sharpe; **no parameter cliff** (∂Sharpe/∂θ bounded) | Flat optima generalize; spikes don't |
| G11 | **Synthetic / bootstrap** | Stationary-bootstrap 5th-pct Sharpe > 0 over ≥ 1000 resamples | Path-dependence check |
| G12 | **Family-wise control** | Romano–Wolf stepdown adjusted p < 0.05 against the full candidate family from this research program | The honest multiple-testing test |
| G13 | **Paper/shadow trading** | ≥ 3 months (or ≥ 60 trading days **and** ≥ 100 fills); realized Sharpe ≥ 0.5 × deflated expectation; slippage within 1.5× modelled | Reality check on execution assumptions |
| G14 | **Capital ramp** | Start ≤ 10% target; double only on meeting a pre-registered PnL/tracking gate | Limits cost of a false positive |

**Expectation deflation for sizing:** Even after all gates, size the strategy to
`SR_expected = min(DSR-implied SR, 0.5 × backtest SR)`. Empirically, live Sharpe lands at roughly
**one-half to one-third** of a well-constructed backtest Sharpe; Harvey–Liu's haircut analysis puts
the multiple-testing haircut alone at ~60% for a 0.75 Sharpe with 200 trials, and >50% for any
Sharpe < 0.4.

---

## 1. The López de Prado framework and its critiques

### 1.1 Structured data: bars

Before labeling, LdP argues time bars are statistically the worst sampling scheme: they
over-sample quiet periods, under-sample active ones, and produce returns with worse
normality/heteroskedasticity properties. Alternatives: **tick bars**, **volume bars**, **dollar
bars** (preferred — invariant to price level and splits), and **information-driven bars**
(imbalance/run bars: sample when order-flow imbalance exceeds an EWMA expectation).

*Implementable rule:* default to **dollar bars** sized so that the average number of bars/day is
stable over the sample (recalibrate the dollar threshold annually to a rolling median of daily
dollar volume, otherwise bar counts explode with market growth — a subtle non-stationarity trap in
naive implementations).

*Critique:* dollar bars help normality but the evidence that they improve *predictive* performance
is thin and mostly anecdotal. They also complicate joining with calendar-indexed data
(fundamentals, macro), which is itself a leakage surface. Treat as a default, not a dogma.

### 1.2 Triple-barrier labeling

Label each event `t0` by which of three barriers is touched first within `[t0, t0+h]`:
- upper (profit-take) at `+pt · σ_{t0}`,
- lower (stop-loss) at `−sl · σ_{t0}`,
- vertical (time) at `t0 + h`.

Label ∈ {+1, −1, 0}. Barriers are **volatility-scaled** (`σ` = EWMA of returns at `t0`, computed
strictly from data ≤ `t0`) so labels are comparable across regimes.

Key derived object: `t1[t0]` = the **actual** label end time (first touch). This is the object that
drives purging, uniqueness, and sample weighting. Getting `t1` wrong silently breaks all three.

**Critiques, honestly stated:**
- **It bakes the exit policy into the label.** The model learns "was a (pt, sl, h) trade profitable",
  not "what is the expected return". If you later change exits, labels are stale. `pt/sl/h` become
  three extra hyperparameters — three extra dimensions of the search space that must be counted
  as trials.
- **Class imbalance and 0-label ambiguity.** With tight barriers, few vertical touches; with wide
  barriers, mostly 0s. Many implementations map vertical touches to `sign(return)`, which
  quietly changes the estimand.
- **Path-dependence within the bar.** If your bars are daily and both barriers are touched
  intrabar, first-touch is undefined without intraday data. Naive OHLC resolution (check high
  before low) introduces a systematic bias. *Rule: require intraday data for barrier resolution, or
  mark such labels as ambiguous and drop them (and record the drop rate).*
- Independent evaluations (Hudson & Thames) find triple-barrier + meta-labeling gives real but
  **modest** improvements — better precision/F1, better risk-adjusted return via sizing — not the
  step-change the book's framing implies.

### 1.3 Meta-labeling

Two-stage: a **primary model** (or rule, or human PM) produces side (long/short); a **secondary
binary model** predicts whether that specific bet will be profitable, and its predicted probability
drives **position size** (not side).

Why it works when it works:
- It converts a *side* problem into a *precision* problem — the secondary model can only filter,
  never flip, so it cannot destroy a primary edge, only fail to improve it.
- It decouples "what to trade" (often economically motivated, low-capacity-for-overfit) from
  "how much" (statistically learned, high-dimensional features OK).
- It improves the F1 of a high-recall/low-precision primary model.

*Implementable sizing:* `size = max(0, (2·Φ⁻¹-based bet size) )` — LdP's bet sizing maps predicted
probability `p` for `n` classes to `m = 2·Φ(z) − 1` where `z = (p − 1/n)/√(p(1−p))`. Then discretize
to avoid churn (e.g., round to nearest 0.05) and apply averaging over concurrent bets.

**Critiques:**
- Meta-labeling **cannot create alpha**; it only reallocates it. If the primary has no edge, the
  secondary learns noise, and the two-stage structure makes the overfitting harder to detect
  because in-sample precision improvements look impressive.
- Training the secondary on the *same* data that produced the primary is a leakage vector. *Rule:
  the primary must be fit (or specified) on a strictly earlier window than the secondary's training
  window, or fit inside the same CV fold with the same purge/embargo.*
- The labels for the secondary are `{0,1}` = was the primary's bet profitable — which is a
  *function of the primary's own errors*, so the secondary's sample is non-i.i.d. and regime-coupled
  in ways sample weighting doesn't fully fix.

### 1.4 Fractional differentiation

Integer differencing (returns) achieves stationarity but destroys memory; raw prices have memory but
are non-stationary. Fractional differencing applies `(1−B)^d` with real `d ∈ [0,1]`:

```
w_0 = 1,  w_k = −w_{k−1} · (d − k + 1)/k
X_t^{(d)} = Σ_{k≥0} w_k X_{t−k}
```

Use **fixed-width window** FFD (truncate weights at |w_k| < τ, e.g. τ = 1e-5) rather than expanding
window, so the transform is time-invariant. Choose the **minimum `d`** such that ADF rejects a unit
root at 95%. Typically `d ≈ 0.3–0.5` for equity prices, often much lower — meaning most of the
memory is retained.

**Critiques:**
- Empirically the *forecasting* benefit is contested. Comparative studies (e.g., LSTM comparisons of
  differentiation techniques) find FFD sometimes helps, sometimes is indistinguishable from log
  returns plus explicit lags/moving averages, which are simpler and more interpretable.
- The stated motivation ("ML needs stationary features") is weaker than claimed: tree ensembles and
  properly regularized nets handle non-stationarity through the *label* distribution shift, not the
  feature scale, and the real killer is regime shift, which FFD does nothing about.
- `d` is another searched hyperparameter. Selecting `d` by downstream Sharpe is straightforwardly
  overfitting; selecting by minimum-`d`-passing-ADF is defensible and cheap. *Rule: select `d` by
  the ADF criterion only, on the training fold only, and never by downstream performance.*
- FFD weights must be computed **inside the fold** (expanding from fold start) or with a fixed
  window that never spans the train/test boundary — otherwise it is a leakage vector (§3.6).

### 1.5 Sample uniqueness, average uniqueness, sequential bootstrap

Overlapping labels mean observations are **not i.i.d.** Define concurrency `c_t` = number of labels
live at time `t`. Then:

```
uniqueness of obs i at time t:  u_{i,t} = 1/c_t
average uniqueness:             ū_i = (Σ_{t∈[t0_i, t1_i]} u_{i,t}) / (t1_i − t0_i)
sample weight (return-attrib.): w_i ∝ |Σ_{t∈[t0_i,t1_i]} r_t / c_t|
time decay:                     w_i ← w_i · d(cumulative uniqueness)
```

**Sequential bootstrap** draws samples with probability inversely proportional to their overlap with
already-drawn samples, raising the average uniqueness of each bootstrap bag toward i.i.d. Used for
bagged classifiers (`max_samples = ū` in sklearn's BaggingClassifier is the cheap approximation).

**Critiques:**
- Sequential bootstrap is **O(N²)** naively (recompute the overlap matrix each draw); on large
  samples it dominates training cost. Practical implementations approximate or subsample; the
  approximation's effect on results is rarely studied.
- The whole apparatus is a patch for an avoidable problem: **you can just not overlap labels.**
  Non-overlapping sampling (one label per horizon) loses data but is unambiguous. Overlap-plus-
  weighting is a bias/variance trade with a poorly characterized bias.
- `ū` conflates two things: label overlap (fixable) and genuine serial correlation of returns
  (not fixable by weighting). Reporting `ū` as if it measures information content overstates it.

*Implementable rule:* compute `ū` always and **emit it as a first-class diagnostic**. If
`ū < 0.05`, the effective sample size is < 5% of `N` — flag the run: t-statistics and DSR computed
on the naive `N` are badly overstated. Use `N_eff ≈ N · ū` everywhere a sample size enters a
statistic.

### 1.6 Purged K-Fold CV and the embargo

**Purging:** remove from the training set any observation `i` whose label span `[t0_i, t1_i]`
overlaps the test set's time range. This is the *only* correct treatment of overlapping labels
across a CV boundary.

**Embargo:** additionally remove training observations in a window *immediately after* the test set,
to kill leakage through serial correlation of features and slow information diffusion.

*Implementable rule (the one worth hard-coding):*

```
purge:   drop train obs i if [t0_i, t1_i] ∩ [test_start, test_end] ≠ ∅
embargo: drop train obs with t0 ∈ (test_end, test_end + E]
E = h + max_feature_lookback + settlement_lag
```

where `h` = label horizon. LdP's percentage embargo (e.g. 1% of T) is a crude proxy; the
**horizon-plus-lookback** rule is strictly better because it is derived from the pipeline rather
than guessed. Minimum: `E ≥ h + 1 bar`. If any feature uses a 252-day lookback, `E ≥ h + 252`.

*Common implementation bugs to test for:*
- Purging only forward (test → train) and not backward. Both directions are needed for K-fold
  (unlike walk-forward, where only one direction exists).
- Using `t0` instead of `t1` for overlap detection (silently under-purges by the full horizon).
- Not embargoing when features are EWMAs — an EWMA has infinite formal lookback; use the effective
  half-life × 5 as `max_feature_lookback`.

### 1.7 Combinatorial Purged Cross-Validation (CPCV)

Split T into `N` contiguous groups; choose `k` groups as test in every combination. Then:

```
combinations:      C(N, k)
each group tested: C(N−1, k−1) times
backtest paths:    φ = k · C(N,k) / N  =  C(N−1, k−1)
```

For `N=6, k=2`: 15 splits, each group appears in 5 tests → **5 distinct backtest paths**.
For `N=10, k=2`: 45 splits → 9 paths. For `N=12, k=3`: 220 splits → 55 paths.

Each path is a full-length OOS equity curve reconstructed from non-overlapping test blocks. This
turns a *point estimate* of Sharpe into a **distribution**, which is the whole point: you can now
compute a confidence interval and feed the distribution to DSR/PBO.

**Evidence for:** Arian, Norouzi & Seco (2024, *Knowledge-Based Systems*) build a synthetic
controlled environment with regime shifts and non-stationarity and report CPCV markedly superior to
K-Fold, Purged K-Fold and especially Walk-Forward on both PBO and DSR test statistic, with
Walk-Forward showing "notable shortcomings in false discovery prevention, characterized by
increased temporal variability and weaker stationarity."

**Critiques — take these seriously:**
1. **It is not a backtest.** CPCV paths train on data from *after* some test blocks. A CPCV path is
   a statement about the *model class's* generalization, not about what a trader could have earned.
   Conflating the two is the most common misuse. *Rule: report CPCV metrics and a strictly causal
   walk-forward, and require both to pass. Never promote on CPCV alone.*
2. **It assumes a degree of stationarity it doesn't test.** Training on the future to predict the
   past is only legitimate if the data-generating process is time-invariant, which is exactly the
   assumption that fails in markets. Where the DGP drifts, CPCV is *optimistically* biased for
   deployment and *pessimistically* biased for model comparison — it flatters models that exploit
   stable structure and penalizes adaptive ones.
3. **The Arian et al. result is on synthetic data.** Their generator's non-stationarity is
   parametric and mild relative to real regime breaks (2008, 2020-03, 2022). The result should be
   read as "CPCV is better at detecting overfitting *given a stationary-ish DGP*", not "CPCV is the
   right deployment simulator."
4. **Paths are not independent.** The φ paths share training data heavily; their Sharpe dispersion
   understates true uncertainty. Do **not** treat path count as an effective sample size, and do
   **not** compute a t-stat on the path mean with `df = φ − 1`.
5. **Cost:** `C(N,k)` model fits. At N=12, k=3 that's 220 fits per candidate. For an agent loop this
   is the dominant compute cost. Sampling a random subset of combinations is standard and loses
   little, but record the sampling seed.
6. **Purging removes a lot of data.** With `k` non-contiguous test groups and `E = h + lookback`,
   the effective training set can shrink dramatically; with long horizons CPCV can become
   infeasible (this is the real reason long-horizon strategies get tested by walk-forward).

*Implementable defaults:* `N = 10–12`, `k = 2–3`, giving 9–55 paths. Require `N ≥ 6`. Compute
Sharpe on every path; gate on the **5th percentile**, not the mean (G4).

### 1.8 Probability of Backtest Overfitting (PBO) via CSCV

Combinatorially Symmetric Cross-Validation (Bailey, Borwein, López de Prado, Zhu):

1. Build `M` (T × N): rows = time, columns = the `N` strategy configurations you tried.
2. Partition rows into `S` disjoint submatrices (S even; S=16 typical).
3. For each of the `C(S, S/2)` combinations `c`: take S/2 submatrices as IS (`J`), complement as OOS (`J̄`).
4. `n*` = argmax of IS performance. Find its OOS rank `r̄_c`; relative rank `ω̄_c = r̄_c/(N+1)`.
5. Logit `λ_c = ln(ω̄_c / (1 − ω̄_c))`.
6. **`PBO = P[λ_c < 0]`** = fraction of combinations where the IS-best config lands below the OOS
   median. Estimated as `∫_{−∞}^{0} f(λ) dλ`.

Companion diagnostics from the same object:
- **Performance degradation:** regress OOS `R̄_{n*}` on IS `R_{n*}`; slope `β` is typically
  *negative* under overfitting (better IS ⇒ worse OOS). *Gate: require β ≥ 0.*
- **Probability of loss:** `P[R^c_{n*} < 0]`. *Gate: < 0.2.*
- **Stochastic dominance:** OOS distribution of the selected config vs. the pooled OOS distribution.

**Critiques:**
- PBO requires the full `N`-column matrix of *all* trials. Agents that discard failed runs cannot
  compute it honestly. **This is the strongest architectural argument for a mandatory experiment
  registry that persists every run's return series.**
- CSCV's IS/OOS splits are combinatorial in *time blocks*, so like CPCV it trains on the future.
  For strongly non-stationary series PBO under-detects overfitting to *regime*, only catching
  overfitting to *noise*.
- PBO is a property of the *selection procedure over a family*, not of a strategy. Reporting "this
  strategy's PBO" for a single candidate is meaningless; you need ≥ 20 configurations (ideally
  ≥ 100) in the family for the rank statistic to have resolution.
- With small `N`, `ω̄` is coarse and PBO is quantized; with correlated configurations (a parameter
  sweep over one strategy) PBO is optimistically low because all columns behave alike. *Rule: PBO
  over a parameter sweep of one idea is weak evidence; PBO over a diverse candidate family is
  strong evidence.*

### 1.9 Probabilistic and Deflated Sharpe Ratio

**PSR** — probability that true SR exceeds a benchmark `SR*`, adjusting for non-normality:

```
PSR(SR*) = Φ( (SR − SR*) · √(T−1) / √(1 − γ₃·SR + ((γ₄−1)/4)·SR²) )
```
with `γ₃` = skewness, `γ₄` = kurtosis of returns (both of the *strategy's* returns, not the market's).
Negative skew and fat tails **reduce** PSR at fixed SR — this correctly penalizes
premium-selling/short-gamma profiles that dominate naive Sharpe rankings.

**Minimum Track Record Length** — the `T` needed for `PSR(SR*) > α`:

```
MinTRL = 1 + (1 − γ₃·SR + ((γ₄−1)/4)·SR²) · ( Φ⁻¹(α) / (SR − SR*) )²
```
Rule of thumb: SR = 1.0 annualized needs ~**3 years** of daily returns to reject SR ≤ 0 at 95%
under normality — and *more* with negative skew.

**Deflated Sharpe Ratio** — PSR with `SR*` set to the expected maximum Sharpe under the null that
all `N` trials are worthless (the **False Strategy Theorem**):

```
E[max SR*] ≈ √(V[SR]) · [ (1−γ)·Φ⁻¹(1 − 1/N) + γ·Φ⁻¹(1 − 1/(N·e)) ]
```
`γ` = Euler–Mascheroni ≈ 0.5772; `V[SR]` = **cross-sectional variance of the Sharpe ratios across
the N trials**; `N` = number of independent trials. Then `DSR = PSR(E[max SR*])`.

`E[max SR*]` grows like `√(2 ln N)` — slowly, but relentlessly. For `V[SR] = 1` (annualized),
`N = 100` gives `E[max SR*] ≈ 2.7`; `N = 1000` gives ≈ 3.3; `N = 10⁶` gives ≈ 5.0. An agent loop
reaches N = 10⁶ in weeks.

**Critiques — the important ones:**
1. **`N` is the whole ballgame and nobody knows it.** DSR is exquisitely sensitive to the trial
   count, which is (a) usually unrecorded, (b) not the raw count but the count of *independent*
   trials. A 200-point grid search over two correlated parameters is nowhere near 200 independent
   trials. **Under-counting makes DSR a rubber stamp; over-counting makes it impossible to pass.**
2. **Estimating `N_eff`.** Practical approaches: (i) cluster trial return series by correlation
   (e.g., ONC / hierarchical clustering at ρ = 0.5) and set `N_eff` = number of clusters;
   (ii) `N_eff = N / (1 + (N−1)·ρ̄)` using average pairwise correlation; (iii) eigenvalue-based:
   `N_eff = (Σλ_i)² / Σλ_i²` on the trial-returns correlation matrix (participation ratio). Pick one,
   **write it down, and never change it to make a candidate pass.**
3. **The False Strategy Theorem cuts both ways.** With enough trials no Sharpe is large enough —
   which is mathematically true and operationally useless if the platform's trial rate is unbounded.
   The correct response is to *bound the trial rate per hypothesis family*, not to discount DSR.
4. **`V[SR]` is itself estimated** from the trial population, and is unstable when trials are few or
   heterogeneous. A single wild trial inflates `V[SR]`, raises `E[max SR*]`, and can reject a genuine
   strategy.
5. **Distributional assumptions.** DSR's variance term uses a Gaussian-kernel approximation of the
   SR estimator's distribution; under strong autocorrelation (illiquid assets, monthly-marked
   portfolios) the denominator is wrong and DSR is optimistic. Lo's non-i.i.d. correction and
   Mertens' skew/kurtosis-aware standard error should be used when `|ρ₁| > 0.1`; Mertens notes that
   wrongly assuming normality can make asymptotic variance estimates **off by up to 70%**.
6. **DSR is not a decision rule for deployment sizing.** DSR ≥ 0.95 says "probably not pure luck",
   not "expect this Sharpe". Size on the deflated point estimate.

*Implementable rule:* DSR must be computed by the **platform**, from the registry's trial count,
using the registry's clustering-based `N_eff`, with `V[SR]` from the registry's trial population.
The agent never supplies `N`.

### 1.10 Minimum Backtest Length (MinBTL)

Bailey et al.: to keep `E[max SR*] ≤ 1` (annualized) when selecting the best of `N` trials on `y`
years of data,

```
MinBTL ≈ 2·ln(N) / E[max SR*]²      (years, for target E[max SR*])
```
Equivalently, with `N` trials, an IS Sharpe of `√(2 ln N / y)` is *expected from noise alone*.

Practical table (Sharpe expected from pure noise):

| Trials N | y = 2 yr | y = 5 yr | y = 10 yr | y = 20 yr |
|---|---|---|---|---|
| 10 | 1.52 | 0.96 | 0.68 | 0.48 |
| 100 | 2.15 | 1.36 | 0.96 | 0.68 |
| 1,000 | 2.63 | 1.66 | 1.18 | 0.83 |
| 10,000 | 3.03 | 1.92 | 1.36 | 0.96 |
| 1,000,000 | 3.72 | 2.35 | 1.66 | 1.18 |

Read this as the platform's most important table. **With 10,000 trials on 5 years of data, a
backtest Sharpe of 1.9 is literally the expected value of noise.** An agent loop that reports
Sharpe 2.0 on 5 years after a broad search has demonstrated nothing.

*Implementable gate:* reject if `backtest_SR < 1.5 × √(2·ln(N_eff)/y)`. This is a crude but
extremely effective first-pass filter that costs nothing to compute.

---

## 2. Walk-forward analysis

### 2.1 Anchored vs. rolling

- **Anchored (expanding):** train on `[0, t]`, test `(t, t+H]`, advance. Training set grows.
  Pros: more data late in the sample, stable estimates, matches how you'd actually retrain.
  Cons: early regimes dominate late models; slow to adapt; the model at the end is fit on 20 years
  of a process that changed.
- **Rolling (sliding):** train on `(t−W, t]`, test `(t, t+H]`. Fixed window `W`.
  Pros: adapts to regime; forces the model to be re-learnable from a realistic data budget.
  Cons: `W` is a hyperparameter (another trial dimension); high variance; late-sample folds may miss
  crises entirely.

*Rule:* run **both**. Anchored is the better generalization test; rolling is the better deployment
simulation. If conclusions differ materially, the strategy is regime-dependent — route it to §7.

### 2.2 Fold design

```
W = training window      (default: max(3 yr, 10 × h))
H = test window          (default: 3–6 months; must contain ≥ 30 independent events)
Δ = step                 (default: Δ = H, i.e. non-overlapping test blocks)
E = embargo = h + max_feature_lookback + settlement_lag
folds = floor((T − W − E) / Δ)
```

*Gate:* require **≥ 10 folds** for any promotion decision; ≥ 20 preferred. Fewer than 10 and the
distribution of fold Sharpes has no resolution. Note the tension: more folds ⇒ shorter `H` ⇒ noisier
per-fold estimates. Resolve in favour of more folds and judge the *distribution*.

A recent concrete instantiation (Interpretable Hypothesis-Driven Trading, arXiv 2512.12924) uses
`W = 252d, H = 63d, Δ = 63d` → 34 OOS periods over 2015–2024, with hypothesis registration and
explicit cost modelling (\$1 commission + 5bp slippage). Their honest reporting — aggregate
p = 0.34, i.e. *not significant* — is a model of how to report.

### 2.3 Nested walk-forward

The only correct structure when you tune anything:

```
for each outer fold (t_train_end):
    inner = purged CV (or short walk-forward) INSIDE [0, t_train_end]
    select hyperparameters θ* on inner folds only
    fit model with θ* on full [0, t_train_end]
    evaluate once on (t_train_end, t_train_end + H]   ← touched exactly once
```

**The outer test block must be touched exactly once per candidate, ever.** The platform should
enforce this with a ledger: a `(strategy_id, fold_id)` pair may be evaluated once; a second
evaluation increments the global trial counter and is flagged.

*Relation to purged CV:* the inner loop of nested walk-forward **is** purged K-fold (or CPCV). The
difference is only the outer loop's causality constraint. A useful framing: **CPCV answers "does
this model class generalize?"; walk-forward answers "would this have worked?"** Both are needed;
neither substitutes.

### 2.4 Regime coverage

Fold count is not the same as information. Ten folds all inside 2013–2019 is one regime.

*Implementable rule:* label every OOS block with a regime tag (§7.1) and require:
- ≥ 3 distinct volatility regimes represented across OOS blocks,
- ≥ 2 designated crisis windows in OOS (from: 2008-09, 2010-05 flash crash, 2011-08, 2015-08,
  2018-02 volmageddon, 2018-Q4, 2020-02/03, 2022 (rates/inflation bear), 2023-03 banking,
  2024-08 yen-carry unwind, 2025 tariff/vol episodes),
- **no single regime contributes > 50% of cumulative OOS PnL**,
- worst-regime Sharpe > −0.5.

The PnL-concentration test is the one that catches the most self-deception: many "robust" strategies
turn out to have made all their money in one 3-month window.

### 2.5 Walk-forward over-tuning — the danger

Walk-forward *feels* safe because each test block is causally clean. It isn't, for three reasons:

1. **Reuse across candidates.** The same OOS blocks are used for candidate 1, 2, ... 10,000. After
   enough candidates, the OOS is in-sample for the *selection procedure*. Walk-forward provides no
   protection against multiple testing — that's what §6 is for.
2. **WFO-efficiency chasing.** Practitioners tune `W`, `H`, `Δ`, and the re-optimization objective
   until "walk-forward efficiency" (OOS/IS performance ratio) looks good. Each of those is a trial.
3. **Anchoring on the last fold.** Humans and agents alike over-weight recent folds; a strategy that
   works in the last two folds and fails in the first eight gets promoted with a "regime has
   changed" narrative. *Rule: require the fold-Sharpe series to have no significant trend
   (Mann–Kendall p > 0.05) — a *rising* trend is as suspicious as a falling one, because it usually
   means the recent folds were leaked into via feature engineering choices made after seeing them.*

Arian et al.'s finding that walk-forward has the *worst* false-discovery properties of the methods
compared is best interpreted as: **a single walk-forward path is a single draw**, and single draws
are terrible estimators. The fix is to combine walk-forward's causality with CPCV's
distribution-over-paths — e.g., run walk-forward over multiple universes, multiple start dates
(shift `t=0` by `Δ/k` for `k` offsets), and multiple bootstrap resamples of the asset universe, and
gate on the resulting distribution's lower tail.

---

## 3. Leakage detection in financial pipelines

Leakage is the only failure mode on this list that is a **bug**, not a statistical subtlety. It
should be caught by tests, not by judgment. Every item below is written as a testable assertion.

### 3.1 Look-ahead bias

Using information not available at decision time. Canonical sources:
- Using the close to decide a trade executed at the same close. *Rule: decision at bar `t` executes
  at `t+1` open (or VWAP of `t+1`), unless you can prove sub-bar timing.*
- Signals from data timestamped by *event* date rather than *availability* date (earnings, macro
  releases, index changes, analyst revisions).
- Using a full-sample volatility/beta/correlation estimate to normalize features.

*Automated test — the timestamp invariant:* every row in every feature table carries
`(event_time, knowledge_time)`. Assert `knowledge_time ≤ decision_time` for every feature used at
`decision_time`. This single invariant, enforced at the data-access layer, eliminates most
look-ahead. Make the feature store refuse to serve rows violating it.

### 3.2 Survivorship bias

Universe built from currently-listed securities. Inflates returns by roughly **1–4% annually** in
equities, more in high-turnover universes, catastrophically in crypto and small-cap.

*Automated tests:*
- Assert the universe at date `t` contains ≥ 1 security that is delisted by `T_end` for any `t` more
  than 2 years before `T_end`. If zero delistings appear anywhere in the sample, the dataset is
  survivorship-biased — fail hard.
- Assert delisting returns are populated (CRSP `DLRET`); a missing delisting return silently assumes
  you exited at the last price, which for bankruptcies is a large positive bias.
- Compare universe size at `t` against a known point-in-time index membership count.

### 3.3 Point-in-time fundamentals and restatements

Reported financials are restated; most vendor databases show the *latest* restated value on the
*original* fiscal date. Using it means trading on numbers nobody had.

*Rules:*
- Require a vendor PIT dataset (Compustat Point-in-Time snapshots, S&P Capital IQ PIT, Refinitiv
  PIT) or apply a conservative reporting lag: **≥ 45 days after fiscal quarter end for 10-Q,
  ≥ 90 days after fiscal year end for 10-K**, and prefer the *actual* filing date (SEC EDGAR
  `acceptedDate`) when available.
- Track `first_reported` vs `latest_restated` and assert the pipeline uses `first_reported`.
- *Automated test:* for a sample of (ticker, quarter), assert the value the pipeline serves at
  `t = fiscal_end + 1 day` is NULL, and becomes non-NULL only at/after the filing date.

### 3.4 Index reconstitution

Backtesting "S&P 500 constituents" using today's membership is survivorship bias with extra steps.
Additions/deletions are announced before they are effective, and the announcement itself moves
prices (index effect).

*Rules:* use point-in-time constituent snapshots; trade on the **effective** date, not the
announcement date, unless the strategy is explicitly an index-effect strategy (in which case model
the announcement-to-effective price run-up explicitly). Assert membership is queried as
`membership_as_of(t)`, never as a static list.

### 3.5 Corporate actions and the adjusted-price trap

The classic: back-adjusted prices change *retroactively* whenever a new dividend or split occurs.
A backtest run today on back-adjusted data uses a price series that did not exist historically.
Consequences:
- Cross-sectional ranks based on price level (e.g., "stocks under \$5") are wrong.
- Volume/dollar-volume computed from adjusted prices is wrong.
- Any strategy with a nominal price threshold (tick size, lot size, penny-stock filters) is wrong.
- Futures continuous contracts: back-adjusted (Panama) series can go **negative**, breaking log
  returns and percentage-based stops. Ratio-adjusted breaks absolute-dollar stops.

*Rules:*
- Store **unadjusted** prices plus an adjustment factor table; adjust *forward from a fixed anchor*
  inside the backtest, or compute returns from unadjusted prices + explicit cash dividends.
- Assert that re-running a backtest with a data snapshot taken 6 months ago produces the same
  historical positions for dates before the snapshot. Any divergence = retroactive adjustment leak.
  (This "snapshot reproducibility test" is the single best automated corporate-action check.)
- For futures, run the strategy on *both* ratio- and difference-adjusted series; large divergence is
  a red flag.

### 3.6 Feature computation leakage

The quiet killer. All of these are trivially introduced and hard to see:

| Leak | Wrong | Right |
|---|---|---|
| Global scaling | `StandardScaler().fit(X_all)` | Fit scaler on train fold only; or use expanding-window z-scores |
| Target/mean encoding | Encode category by full-sample target mean | Encode with out-of-fold means inside each CV fold |
| Imputation | `fillna(X.mean())` on full data | Fit imputer per fold; or forward-fill only |
| Winsorization/clipping | Clip at full-sample 1%/99% | Expanding-window quantiles |
| Feature selection | Select top-k features on full data, then CV | Selection inside the fold, counted as a hyperparameter |
| Resampling/SMOTE | Oversample then split | Split then oversample train only |
| PCA / factor models | Fit PCA on full panel | Fit on train fold; roll the loadings |
| Denoising / detrending | HP filter, wavelet denoise, Kalman smoother on full series | Only *filtered* (causal) versions, never *smoothed* |
| Fractional differencing | Expanding weights across the split | Fixed-width FFD, window ≤ embargo |
| Volatility targeting | Scale by realized vol computed with centered window | Trailing-only vol |
| Cross-sectional rank | Rank using full-day data at same timestamp as execution | Rank at `t`, execute `t+1` |

*Automated test — the future-shuffle test:* take the full pipeline, randomly permute all data
strictly after `decision_time` for each decision, re-run, and assert every feature value at
`decision_time` is bit-identical. Any feature that changes is reading the future. This is the single
most powerful leakage test you can implement and it catches all of the above mechanically.
Implementation: wrap the dataframe in a proxy that raises on any read of rows with index > current
decision time (a "causal access guard"), and run the pipeline under it in CI.

*Second test — the shifted-label test:* re-run with labels shifted forward by `2h`. Performance
should collapse to ~0. If it doesn't, the model is picking up structure that isn't the label, i.e.
a leak or an artifact.

*Third test — the random-label test:* replace labels with a random permutation preserving the
class balance and the `t1` structure. The full pipeline (including CV, selection, sizing) must
produce Sharpe ≈ 0 with a confidence interval containing 0. If it produces Sharpe 0.5, your
*evaluation harness* is broken — this catches leakage in the backtester itself, not just the
features. Run this as a CI gate on the platform, not per-strategy.

### 3.7 Leakage from overlapping labels

Covered in §1.5–1.6. The specific failure: with horizon `h` and daily sampling, two adjacent
observations share `h−1` days of outcome. Random K-fold puts one in train and one in test →
near-duplicate leakage → wildly optimistic CV scores. Symptom: **CV score far above walk-forward
score**. *Automated test: compute both; if `CV_Sharpe − WF_Sharpe > 1.0`, flag for overlap
leakage.*

### 3.8 Other financial-specific leaks

- **Intraday bar timestamp convention.** Bar labeled `09:30` — is that the start or end of the
  interval? Off-by-one-bar leakage is endemic. *Test: assert bar `t`'s high/low/close are unknowable
  at the decision timestamp attached to bar `t`.*
- **Time zones and DST.** Mixing exchange-local and UTC timestamps produces 1-hour look-ahead twice
  a year. *Test: all timestamps tz-aware, stored UTC, exchange calendar applied explicitly.*
- **Vendor backfill.** New data fields are backfilled to inception; the field did not exist
  historically. *Test: check the vendor's field-introduction date against the backtest start.*
- **Borrow/short availability backfill.** Borrow-cost datasets often only exist post-2010; assuming
  shortability before then is leakage.
- **Options/derived data.** Implied vol surfaces are often computed with same-day settlement prices
  published after close.
- **Universe filters using future data.** "Stocks with ≥ \$10M ADV over the full sample" is a leak;
  use trailing ADV.
- **Optimizer warm starts / random seeds shared across folds** — a subtle path for information to
  cross the boundary in iterative model fitting.

### 3.9 The implementable leakage suite

Ship these as a `pytest` suite that every candidate must pass (G1):

```
test_causal_access_guard()        # future-shuffle / proxy raises on future reads
test_random_label_sharpe_zero()   # harness-level; |Sharpe| < 0.2, CI contains 0
test_shifted_label_collapse()     # 2h label shift ⇒ Sharpe drops ≥ 80%
test_snapshot_reproducibility()   # old data snapshot ⇒ identical historical positions
test_delistings_present()         # universe contains securities that later delist
test_delisting_returns_populated()
test_pit_fundamentals_null_before_filing()
test_membership_is_point_in_time()
test_knowledge_time_le_decision_time()   # over all features, all rows
test_scaler_fit_only_on_train()   # introspect sklearn pipeline for fit-on-full
test_embargo_ge_horizon_plus_lookback()
test_purge_uses_t1_not_t0()
test_cv_vs_walkforward_gap()      # gap > 1.0 Sharpe ⇒ fail
test_no_nan_forward_fill_across_split()
test_timezone_awareness()
test_bar_timestamp_convention()
test_execution_lag_at_least_one_bar()
```

---

## 4. Realistic execution modeling

The single largest source of live-vs-backtest divergence after overfitting. A strategy whose edge is
smaller than its true cost is not a strategy.

### 4.1 Cost decomposition

Total cost per trade = **commission + fees/taxes + half-spread + temporary impact + permanent
impact + delay/slippage + opportunity cost (unfilled)**.

```
C_total = C_fixed + C_spread + C_temp + C_perm + C_delay
C_fixed  = commission + exchange fees + SEC/FINRA fees + stamp duty (UK 0.5%, etc.)
C_spread = ε · |x| · P                       ε = half-spread (bps)
C_temp   = β · σ · (x/V) · |x| · P           (Almgren–Chriss linear form)
C_perm   = ½ · α · σ · (x/V) · |x| · P
ΔP_perm  = α · σ · (x/V) · P                 (lasting price shift, decays)
```

**Square-root law (the empirically robust one):**

```
I(Q) = Y · σ_daily · √(Q / ADV)
```
`Y` ≈ 0.5–1.0 (commonly ~0.5–0.6 for liquid equities; higher for less liquid). This is the most
replicated result in market microstructure and holds across asset classes and decades. For
`Q = 1% ADV` and `σ = 2%/day`, impact ≈ `0.6 × 2% × 0.1 = 12 bps`. For `Q = 10% ADV`: ≈ 38 bps.

**Permanent impact decay:** `ΔP_t = ΔP_{t−1}·(1−λ)`, `λ = 1 − 2^(−1/τ½)`, half-life `τ½ ≈ 5 days`
for large-cap equities. Matters for strategies that re-trade the same names.

*Implementable default cost model (US large-cap equities, 2026):*

| Component | Value |
|---|---|
| Commission | 0.1–0.5 bps (institutional) / \$0.005 per share (retail-ish) |
| Half-spread | max(0.5 bps, 0.5 × trailing median quoted spread at trade time) |
| Temporary impact | `0.6 · σ_d · √(Q/ADV)` |
| Permanent impact | `0.3 · σ_d · √(Q/ADV)`, half-life 5d |
| Delay/slippage | 1–3 bps for next-open execution; 5 bps in high-vol regimes |
| Short borrow | GC: 25–50 bps/yr; HTB: 100–10,000 bps/yr — **must use PIT borrow data** |

*Rule:* costs must be **time-varying and regime-dependent**. A constant 5 bps assumption
systematically flatters strategies that trade most in crises (when spreads are 3–10× wider) — which
is most mean-reversion strategies. Use realized spread history; where unavailable, scale spread with
realized volatility (`spread_t ∝ σ_t` is a decent proxy).

### 4.2 Participation-rate / ADV limits

```
daily_participation = |shares_traded| / ADV_20
```
*Gates:* soft cap 5% of ADV per name per day, hard cap 10%. Above ~10% the square-root law
under-estimates and execution becomes signalling. Enforce by **clipping the trade and carrying the
residual**, not by scaling down the whole portfolio — clipping-with-carry reproduces the real
implementation shortfall (you don't get the price you wanted for the unfilled part).

*Automated test:* assert `max(daily_participation) ≤ hard_cap` over the whole backtest; report the
distribution. If the 99th percentile is at the cap, the backtest is capacity-constrained and the
reported Sharpe is not achievable at the stated AUM.

### 4.3 Capacity analysis

Run the backtest at AUM ∈ {0.1×, 0.3×, 1×, 3×, 10×, 30×} target and plot Sharpe vs. AUM.
Define **capacity = AUM at which Sharpe falls to half its small-AUM value** (or to a chosen
hurdle). Because impact is `√Q`, net return per dollar declines smoothly; the curve is informative.

*Gate (G3):* deploy at ≤ 20% of capacity-at-half-Sharpe. Report capacity in the strategy card.
Note capacity is regime-dependent — recompute using crisis-period ADV, not full-sample.

### 4.4 Shorting: borrow, locates, costs

- **Availability:** not all names are shortable at all times. Use PIT borrow data (IHS Markit
  Securities Finance / S&P Global, Hazeltree). Missing data ⇒ assume **not shortable** for
  microcaps and recently-IPO'd names.
- **Cost:** borrow fee accrues daily on market value; hard-to-borrow names can cost 10–100%+
  annualized and the fee **rises exactly when the short is working** (crowded shorts).
- **Recalls / buy-ins:** model a forced-close probability for HTB names; in stress, recall risk
  spikes (2021 meme squeeze, 2008 short-sale bans).
- **Short-sale bans:** hard-code the 2008-09 and 2011 EU bans as periods where financials could not
  be shorted. Any backtest that shorts financials in Sep–Oct 2008 is fictional.
- **Regulatory:** uptick/alternative uptick rule (SSR) triggers at −10% intraday — affects fill
  assumptions for short entries on down days, which is precisely when mean-reversion strategies want
  to short.

*Gate:* long-short strategies must report PnL decomposition long vs. short, and net-of-borrow-cost
performance. If > 60% of gross PnL comes from the short book, require HTB-name exclusion robustness
(re-run excluding names with borrow > 200 bps; Sharpe must retain ≥ 60%).

### 4.5 Latency

- **Decision-to-order latency:** feature computation + inference + risk checks. Measure it; it is
  often 100ms–10s for ML pipelines, which matters for anything intraday.
- **Order-to-exchange latency:** colocation ~µs; cloud-to-exchange 1–50 ms.
- **Market data latency:** consolidated (SIP) feeds lag direct feeds by ~500µs–5ms.

*Rule:* the backtest must apply a **latency budget** as a hard delay between signal timestamp and
the earliest fillable market data. Default: 1 full bar for daily/hourly; measured p99 latency + 2×
for intraday. For anything below 1-minute bars, a naive backtest is worthless without an LOB
simulator.

### 4.6 Fill models

**Market orders:** fill at (next available price) + half-spread + impact. Add a slippage draw:
`slip ~ Lognormal` calibrated to realized TCA, not a constant.

**Limit orders — where backtests lie most:**
- Naive "filled if price touched my limit" is **badly optimistic**: you're at the back of the queue,
  and the trades that reach you are exactly the ones where the price is about to keep going
  (adverse selection).
- Minimum realistic model: fill only if price **trades through** your limit by ≥ 1 tick, and even
  then fill only a fraction. Better: queue-position model — track queue ahead `Q_ahead`, decrement
  by observed trades and cancellations (probabilistic cancel model), fill when `Q_ahead ≤ 0`.
- Recent work (Rosenbaum et al., arXiv 2603.24137) identifies two systematic gaps in LOB simulators:
  (i) **inter-event times are not exponential** — they cluster at exchange round-trip latency
  (~29µs), so "latency races" determine fills, and Markovian simulators over-estimate passive fill
  rates; (ii) **no market impact feedback** — after your execution "the book evolves as if nothing
  happened", systematically overstating profitability. Fixes proposed: empirical inter-event time
  distributions, conditional fill probability when inter-event time < exchange latency, and a
  power-law-decay signed-flow feedback kernel reproducing concave impact and partial reversion.

*Gate:* any strategy relying on passive fills must be validated against a queue-aware simulator, and
must show its edge survives assuming **50% of passive fills do not occur** (the worst-case adverse
selection scenario).

### 4.7 Tick size, lot size, and other discreteness

- Round prices to tick (US equities \$0.01 above \$1, \$0.0001 below; SEC tick-size pilot / 2024
  half-penny rules for certain tick-constrained names — check the effective date).
- Round shares to lot / whole shares (fractional shares available at some brokers — state the
  assumption).
- Futures: contract multipliers and minimum ticks make small accounts un-tradeable — a backtest that
  takes 0.3 contracts is fiction.
- **Rounding bias:** always round *against* the strategy (round position size down, round price to
  the worse side). Systematic favorable rounding is a common silent optimism.
- Minimum commission floors make small trades uneconomic; model them, and check what fraction of
  trades are below the economic minimum.

### 4.8 Cost sensitivity gate (G2)

The most decision-relevant single test in this section:

```
for multiplier in [0.5, 1, 2, 3, 5, 10]:
    run backtest with costs × multiplier
break_even = multiplier at which net Sharpe = 0
```
*Gate:* `break_even ≥ 3`. A strategy whose edge vanishes at 2× modelled costs will not survive
contact with reality, because cost models are systematically optimistic (they omit opportunity cost,
tail spreads, and the fact that your own flow is correlated with everyone else's).

---

## 5. Evaluation metrics

### 5.1 Separate the two metric families — this is a common conflation

| Predictive-model metrics | Portfolio metrics |
|---|---|
| Accuracy, precision, recall, F1, AUC | Sharpe, Sortino, Calmar |
| Log loss, Brier score | Max drawdown, time-under-water |
| **Information Coefficient** (IC = Spearman ρ(forecast, realized)) | Turnover, capacity |
| Rank IC, IC decay curve, IC-IR | Tail metrics (VaR, CVaR, skew, kurtosis) |
| Calibration (reliability diagram) | Factor alpha and t-stat |

They answer different questions and **a model can win on one and lose on the other**. High accuracy
with wrong-sized bets loses money; a 51%-accuracy model with good calibration and correct sizing
makes money. The bridge is the **Fundamental Law of Active Management**:

```
IR ≈ IC · √(Breadth) · TC
```
`TC` = transfer coefficient (correlation between the ideal and the constrained/implemented
portfolio; typically 0.3–0.6 with real constraints, which halves the theoretical IR). Constraints,
costs, and turnover limits show up *only* in `TC` — which is why a great IC can produce a mediocre
live IR.

*Rules:*
- Report IC and IC-IR (`mean(IC)/std(IC)`) per period, plus the **IC decay curve** (IC at horizons
  1, 2, 5, 10, 20, 60 days). A signal whose IC peaks at horizon 1 and dies by 5 has a turnover
  problem; one that peaks at 20 has a capacity opportunity.
- Realistic equity cross-sectional IC: **0.02–0.05** monthly is a genuinely good signal. Report IC
  > 0.15 and you should assume leakage until proven otherwise.
- Always report **both** families; gate on portfolio metrics, diagnose with model metrics.

### 5.2 Risk-adjusted performance

- **Sharpe** `= (μ − r_f)/σ`, annualized by `√252` *only if returns are i.i.d.* With autocorrelation
  `ρ_k`, the correct scaling is `SR_T = √(q / (q + 2Σ(q−k)ρ_k)) · SR_t`. Positive autocorrelation
  (illiquid assets, smoothed marks) makes naive annualization **overstate** Sharpe substantially.
  *Rule: test `ρ₁` of strategy returns; if `|ρ₁| > 0.1`, use the corrected annualization and say so.*
- **Standard error:** `se(SR) ≈ √((1 + SR²/2)/n)` under normal i.i.d.; with skew `γ₃` and excess
  kurtosis `γ₄`, use Mertens: `se(SR) = √((1 + SR²/2 − γ₃·SR + (γ₄/4)·SR²)/n)`. Mis-assuming
  normality can be off by up to **70%**. Prefer studentized bootstrap CIs; for comparing two
  Sharpes use Ledoit–Wolf's bootstrap test (robust to non-i.i.d.).
- **Sortino** `= (μ − MAR)/σ_downside`. Rewards positive skew. Susceptible to gaming by strategies
  with rare large losses (short vol) — always pair with skew/kurtosis and CVaR.
- **Calmar** `= annualized return / |max drawdown|`. Intuitive but **unstable**: MDD is an extreme
  order statistic with enormous sampling variance and grows mechanically with sample length. Prefer
  **average of N largest drawdowns** or the **Ulcer index** (RMS drawdown) for stability. Gate on
  Calmar only with ≥ 10 years.
- **Omega** `= E[(r−θ)⁺]/E[(θ−r)⁺]`. Uses the whole distribution; good complement, rarely decisive.
- **Drawdown statistics to report:** MDD, MDD duration, time-under-water (fraction of days below
  prior peak), recovery time, drawdown at 95th percentile, Ulcer index, and the **conditional
  drawdown at risk (CDaR)**. Time-under-water is the metric that actually determines whether a
  strategy survives organizationally.

### 5.3 Turnover and tail risk

- **Turnover** `= Σ|Δw| / 2` per period, annualized. Report it prominently — it converts directly
  into cost sensitivity and capacity. Also report **holding-period distribution** and the
  **cost-to-gross-alpha ratio** (`total costs / gross PnL`); above 50% the strategy is really an
  execution business.
- **Tail risk:** skew, excess kurtosis, VaR₉₅/₉₉, CVaR₉₅/₉₉, worst day/week/month, and the ratio
  `CVaR₉₉ / σ` (a fat-tail detector). Report the **largest 5 losing days** and check whether they
  cluster (they usually do — that's the real risk).

### 5.4 PSR / DSR in the metric stack

See §1.9. Report PSR(0) (is it better than nothing?), PSR(benchmark), and DSR with the platform's
`N_eff`. Put all three on every strategy card.

### 5.5 Alpha t-stat and factor attribution (G8)

Run the strategy's excess returns against a factor battery:

```
r_t − r_f = α + β_MKT·MKT + β_SMB·SMB + β_HML·HML + β_RMW·RMW + β_CMA·CMA
          + β_MOM·MOM + β_STR·STR + β_BAB·BAB + β_QMJ·QMJ + ε_t
```
Use Fama–French 5 + momentum (Carhart), plus AQR's **BAB** (betting-against-beta) and **QMJ**
(quality-minus-junk) — these two catch a large share of "novel" strategies that are really leverage
or quality tilts. For multi-asset, add a trend/time-series-momentum factor and a carry factor. Use
**Newey–West HAC** standard errors (lag = `⌈4(T/100)^{2/9}⌉`).

*Gates:*
- `t(α) ≥ 3.0` — Harvey–Liu–Zhu's threshold for the factor zoo, and the right default given the
  platform's own multiple testing. (2.0 is the naive threshold and is wrong here.)
- For market-neutral claims: `|β_MKT| ≤ 0.3` and the *time series* of rolling 6-month β must not
  drift monotonically.
- Factor-model `R² < 0.7` — above that, the strategy is mostly a factor combination and should be
  benchmarked against a cheap factor ETF blend, not against cash.
- **Alpha must survive costs**: run the regression on *net* returns. Many strategies have `t(α) = 4`
  gross and `t(α) = 0.5` net.
- Report **rolling 2-year alpha**; a strategy whose alpha is concentrated in the first third of the
  sample is a decayed anomaly (§8.4).

### 5.6 What to put on a strategy card

`SR_net`, `SR_net` 95% CI (bootstrap), PSR(0), DSR(`N_eff`), PBO, CPCV path-Sharpe distribution
(5th/50th/95th), walk-forward fold-Sharpe distribution, MDD, time-under-water, turnover,
cost/gross ratio, capacity, break-even cost multiple, `t(α)` net vs. factor battery, β vector,
IC and IC decay, regime PnL decomposition, `N_eff` trials consumed, and the pre-registration hash.

---

## 6. Multiple testing and data-snooping controls

### 6.1 The problem, quantified

Harvey, Liu & Zhu documented 300+ published "factors" and argued that with that many tests, a
t-stat of 2.0 is meaningless; the appropriate threshold is **≈ 3.0**. Harvey's broader claim: "most
of the claimed research findings in financial economics are likely false."

For an automated platform the numbers are far worse. `E[max SR] ~ √(2 ln N / y)` means the
noise-implied Sharpe scales with `√ln N` — cheap to fight, but you must *know* `N`.

### 6.2 Harvey–Liu haircut Sharpe

Procedure:
1. `t = SR · √T`.
2. Single-test p-value `p = 2(1 − Φ(t))` (or use the t-distribution).
3. Adjust for `M` tests by one of:
   - **Bonferroni:** `p_adj = min(M·p, 1)` — controls FWER, most conservative.
   - **Holm:** `p_Holm(i) = min{ max_{j≤i} [(M−j+1)·p(j)], 1 }` — controls FWER, uniformly more
     powerful than Bonferroni.
   - **BHY (Benjamini–Hochberg–Yekutieli):** controls **FDR** under arbitrary dependence; least
     conservative. Harvey & Liu **recommend BHY for finance** because we care about the proportion of
     false discoveries among promoted strategies, not about never making one error.
4. Convert `p_adj` back to `t_adj` and `SR_adj = t_adj/√T`. Haircut `= (SR − SR_adj)/SR`.

Key empirical results: annualized SR 0.75, T = 240 months, N = 200 trials → SR haircut to **0.32
(~60%)**. The haircut is **non-linear**: SR < 0.4 typically haircut > 50%; SR > 1.0 haircut ≤ 25%.
The common "halve the backtest Sharpe" rule of thumb is therefore wrong in both directions — too
lenient for marginal strategies, too harsh for strong ones.

*Rule:* use **BHY** as the platform's default FDR control across the candidate family, target
`FDR ≤ 0.10`. Report Bonferroni as a conservative bound.

### 6.3 White's Reality Check (RC)

Tests `H₀: max_k E[f_k] ≤ 0` where `f_k` is the performance of strategy `k` relative to a benchmark.
Uses the **stationary bootstrap** (Politis–Romano) on the `T × N` matrix of relative performances:

```
V̄ = max_k √T · f̄_k
for b in 1..B:
    resample time indices via stationary bootstrap (geometric block length, mean 1/q)
    V*_b = max_k √T · (f̄*_{k,b} − f̄_k)
p_RC = #{V*_b > V̄} / B
```
*Critique:* RC is conservative — it is dominated by poor strategies in the family (the "null is least
favourable" construction), so adding bad candidates makes it harder for a good one to pass. In an
agent system that generates thousands of junk candidates, RC becomes nearly unusable.

### 6.4 Hansen's SPA test

Fixes RC's conservatism by (a) studentizing (`f̄_k / ω̂_k`) and (b) recentering only strategies that
are not "too bad" (a threshold that discards obviously-dominated candidates from the null). Hansen's
SPA has materially better power and is the **preferred default** over RC.

*Rule:* use **SPA** as the family-level test of "does the best candidate in this family beat the
benchmark?" with stationary bootstrap, `B ≥ 5000`, block length from Politis–White automatic
selection.

### 6.5 Romano–Wolf stepdown (the one to actually implement)

RC/SPA answer "is *any* strategy good?" Romano–Wolf answers "**which** strategies are good?" while
controlling FWER — which is what a promotion pipeline needs.

```
1. compute studentized statistics t_k for all k; sort descending
2. bootstrap (stationary) the joint null distribution of max_k |t*_k|
3. reject the top strategy if t_(1) > q_{1−α}(max of bootstrap)
4. remove rejected strategies; recompute max over the REMAINING set; repeat
5. stop when no further rejection
```
Properties: controls FWER at `α` under arbitrary dependence, accounts for the correlation structure
across candidates (so a sweep of 200 correlated variants is *not* penalized like 200 independent
ones — this matters enormously for agent-generated families), and is uniformly more powerful than
Holm.

*Gate (G12):* a candidate is promotable only if it survives Romano–Wolf stepdown at `α = 0.05`
against the **full family of candidates generated in the same research program** (not just the ones
the agent liked). `B ≥ 5000` bootstrap replications.

This is the most defensible single statistical gate in the document, because it uses the actual
correlation structure of your trials rather than a guessed `N_eff`.

### 6.6 False Strategy Theorem

See §1.9. Operational reading: `E[max SR]` under the null grows without bound in `N`. Therefore
**cap the trial budget per hypothesis family** rather than letting the agent search freely. A
concrete scheme:

```
budget(family) = 100 trials for the first pass
                 + 100 more only if the family's best DSR at 100 trials ≥ 0.90
hard cap        = 1000 trials per family per quarter
```
Exhausting the budget without a DSR pass **retires the family** — it cannot be resurrected with a
new name. Enforce by hashing the hypothesis description + feature set into a family ID.

### 6.7 The experiment registry — the architectural centerpiece

Nothing above works without it. Requirements:

**Schema (one row per backtest execution, immutable, append-only):**
```
run_id, timestamp, agent_id, session_id
family_id            # hash of (hypothesis text, feature set, universe, horizon)
lineage_parent       # run_id this was derived from
strategy_spec_hash   # full config hash
code_commit          # git SHA of the strategy + platform
data_snapshot_id     # exact dataset version
params               # full hyperparameter dict
cv_scheme            # walk-forward / CPCV config incl. purge & embargo
cost_model_id
oos_returns          # FULL return series, stored — required for PBO/SPA/Romano-Wolf
oos_sharpe, pbo, dsr, alpha_t, turnover, capacity, break_even_cost_mult
gates_passed[], gates_failed[]
disposition          # promoted / rejected / retired / abandoned
```

**Non-negotiable rules:**
1. **Every** execution is logged — including crashed, abandoned, and "just exploring" runs.
   The backtester writes the registry row *before* returning results to the caller; results are
   unavailable if the write fails. The agent cannot opt out.
2. **Trial counting is automatic.** `N_family` = count of runs with the same `family_id`.
   `N_program` = count across the whole research program in the lookback window.
3. **`N_eff` is computed from stored return series** by correlation clustering, not guessed.
4. **Pre-registration:** a candidate must have a registry row with `disposition = 'registered'`
   containing the hypothesis, the feature list, the universe, the horizon, the cost model, and the
   gate thresholds **before** its first backtest. Post-hoc gate-threshold changes are recorded as
   new registrations and reset the trial counter for that family (i.e., they are expensive).
5. **The holdout ledger.** Maintain a final holdout period (most recent 20% of data, or a separate
   asset universe) that is **evaluated at most once per strategy, ever**. Access is logged and
   rate-limited at the platform level. A strategy that touches the holdout twice is rejected
   automatically.
6. **Immutability + audit.** Append-only; no deletes. Periodic audit: sample runs and verify
   reproducibility from `code_commit` + `data_snapshot_id`.
7. **Family retirement.** Once a family exhausts its trial budget, the platform refuses new runs
   under that `family_id`.

Anecdotally, systems like QuantEvolve (arXiv 2510.18569) — a multi-agent evolutionary strategy
discovery framework with island populations and migration — use only a **standard three-period
temporal split** with no CPCV, no multiple-testing correction, and no trial accounting. Evolutionary
search over thousands of candidates against a fixed validation set is a textbook overfitting engine;
the absence of trial accounting in current agentic quant systems is the field's biggest open gap and
the clearest place for a platform to differentiate.

---

## 7. Regime analysis and robustness

### 7.1 Regime detection

Use **several** methods; agreement is the signal, disagreement is a warning.

- **Gaussian HMM** on (return, realized vol, maybe term structure): 2–4 states, EM fitting.
  Interpretable as "calm bull / choppy / crisis". Must be fit **causally** (filtered state
  probabilities, never smoothed) when used as a feature — smoothed Viterbi paths are a look-ahead
  leak. When used only for *post-hoc analysis* of a backtest, smoothed is fine and preferable.
  2026 work extends to **adaptive hierarchical HMMs** for structural change and **ensemble-HMM
  voting** across model specifications for regime-shift detection.
- **Change-point detection:** PELT / binary segmentation (offline, for analysis), CUSUM or Bayesian
  online change-point detection (online, for features). LdP's CUSUM filter is a cheap event sampler.
- **Volatility regimes (the simplest and most robust):** tertiles/quartiles of trailing 21-day
  realized vol, or VIX terciles. Use this as the default labelling for regime attribution — it
  requires no fitting and can't overfit.
- **Clustering:** k-means / HDBSCAN on a feature vector (vol, correlation dispersion, credit spread,
  term slope, breadth). Useful for cross-sectional regime tags.
- **Correlation regimes:** average pairwise correlation of the universe; crisis = correlation spike
  (the "diversification disappears when you need it" regime).

*Rule:* regime tags used for **reporting/attribution** should be the simple, unfittable ones
(vol terciles, crisis calendar). Fitted regime models (HMM) used as **features** must go through the
full leakage suite and count as trials.

### 7.2 Scenario replay and stress testing

Maintain a fixed library of stress windows and **run every candidate through all of them**:

| Window | Character |
|---|---|
| 1998-08/10 | LTCM / Russia — convergence trade blowup |
| 2000-03 → 2002-10 | Dot-com unwind — momentum reversal |
| 2007-08 | **Quant quake** — the canonical equity-stat-arb stress; mandatory |
| 2008-09 → 2009-03 | GFC — liquidity, short bans, correlation → 1 |
| 2010-05-06 | Flash crash — intraday liquidity vacuum |
| 2011-08 | US downgrade / EU crisis |
| 2015-08-24 | ETF/NBBO dislocation |
| 2018-02-05 | Volmageddon — short-vol wipeout |
| 2018-Q4 | Rapid de-risking |
| 2020-02/03 | COVID — fastest drawdown + Treasury basis dislocation |
| 2021-01 | Meme squeeze — short-book catastrophe, borrow recalls |
| 2022 full year | Inflation/rates bear — factor rotation, 60/40 failure |
| 2023-03 | Regional banking / SVB — sector-specific gap |
| 2024-08-05 | Yen-carry unwind — cross-asset deleveraging |
| 2025 tariff/vol episodes | Policy-shock gap risk |

*Gates:* worst-window return ≥ −2× the backtest's MDD; no window in which the strategy loses more
than 6 months of expected PnL; and explicitly test the **2007 quant quake** for any equity
cross-sectional strategy (it is the single most diagnostic window for crowding).

For periods outside the data sample, apply **scenario shocks**: instantaneous ±3σ moves in the
strategy's dominant factor exposures, correlation → 0.9 across the book, spreads × 5, borrow
unavailable, and 50% of passive orders unfilled. Report the P&L under each.

### 7.3 Synthetic data and market simulators

Purpose: generate additional *plausible* histories so the strategy is evaluated against a
distribution of paths rather than the one path history happened to take.

- **IID bootstrap** — wrong for financial series (destroys volatility clustering and
  autocorrelation). Don't.
- **Block bootstrap (moving/circular)** — preserves short-range dependence; block length `b` is a
  nuisance parameter, and the resampled series is non-stationary at block boundaries.
- **Stationary bootstrap (Politis–Romano)** — geometric block lengths with mean `1/q`; the resampled
  series is stationary. **This is the default.** Choose `q` via **Politis–White automatic
  block-length selection** (data-driven, based on the correlogram) rather than by hand — hand-tuning
  the block length to make results look good is itself a trial.
- **Model-based simulation:** GARCH / EGARCH / GJR with Student-t innovations; multivariate:
  DCC-GARCH or a factor model + copula. Reproduces vol clustering and fat tails; misses regime
  breaks and cross-asset contagion structure.
- **Generative models (2024–2026):** GAN-based (QuantGAN, TimeGAN, SigCWGAN), **diffusion models**
  (the current frontier — Quantitative Finance 2025 and arXiv 2410.18897 / 2507.19003 show diffusion
  models reproducing stylized facts better than GANs), and hybrid **GAN–diffusion** frameworks
  (arXiv 2605.27113). Evaluate them on **stylized facts**, not on visual similarity: heavy tails
  (tail exponent ≈ 3–5), volatility clustering (slow decay of `|r|` autocorrelation), absence of
  linear return autocorrelation, leverage effect, aggregational Gaussianity, and volume-volatility
  correlation.
- **Agent-based market simulators:** ABIDES, JAX-LOB, and order-level LLM-agent simulators
  (e.g., StockSim). Useful for impact/execution studies and for testing strategy interaction
  effects; not useful for alpha validation.

**Critique — the fundamental limit:** generative models are trained on history and therefore encode
history's regimes. They *cannot* generate a genuinely novel crisis; they interpolate. Using them to
claim robustness to unseen regimes is circular. **Use synthetic data to test for path-dependence and
parameter fragility, never as evidence of out-of-sample edge.** Also: any generative model is itself
a fitted object and can leak (train it only on the training fold).

*Gate (G11):* 1000 stationary-bootstrap resamples of the strategy's return series (and, separately,
of the underlying price data with the full pipeline re-run); require 5th-percentile Sharpe > 0 and
5th-percentile MDD within 1.5× the backtest MDD.

### 7.4 Sensitivity and perturbation testing (G10)

Three distinct perturbation axes:

1. **Parameter perturbation:** for each hyperparameter `θ`, evaluate at `θ·(1±10%)` and `θ·(1±20%)`
   (or ± 1–2 discrete steps). Require median Sharpe ≥ 0.7 × center Sharpe and **no cliff** — define a
   cliff as any single-step change causing > 40% Sharpe drop. *A parameter optimum sitting on a
   spike is overfit by construction; a broad plateau is the signature of real structure.* Visualize
   as a heatmap for 2-D parameter pairs.
2. **Data perturbation:** add small noise to prices (e.g., ±0.5 tick uniform), jitter timestamps by
   one bar, and randomly drop 5% of observations. Sharpe should be stable. Large sensitivity to tick
   noise means the strategy is trading microstructure it can't actually capture.
3. **Universe perturbation:** re-run on random 80% subsets of the universe (100 draws), on each
   sector excluded in turn, and on a different-but-related universe (e.g., Russell 1000 vs. S&P 500;
   or a different country/exchange). *Gate: median subset Sharpe ≥ 0.7 × full; no single name
   contributing > 20% of PnL; no single sector > 40%.* The single-name concentration test catches an
   enormous fraction of spurious backtests.
4. **Start-date perturbation:** shift the backtest start by `k` months for `k ∈ {1..12}`. A strategy
   whose Sharpe swings by > 0.5 across start dates is path-lucky.

---

## 8. Live-vs-backtest degradation

### 8.1 Expected degradation — set expectations numerically

Sources of the gap, roughly ordered by size:
1. Multiple testing / selection bias (Harvey–Liu haircut: 25–60%+)
2. Unmodelled costs and impact (10–40%)
3. Regime change / alpha decay (varies; see §8.4)
4. Implementation slippage (TC < 1: constraints, risk limits, rounding) (10–30%)
5. Capacity effects at real AUM
6. Residual leakage that survived the tests

*Rule:* the platform's deployed expectation is
`SR_live_expected = min(SR_DSR_implied, 0.5 × SR_backtest)` and position sizing uses that number.
Publishing the backtest Sharpe as the expectation is the organizational root of most blowups.

### 8.2 Paper trading protocol (G13)

Paper trading is a **test of the execution and data pipeline**, and only weakly a test of alpha
(the sample is far too short to resolve Sharpe).

*Protocol:*
- Duration: ≥ 3 months, and ≥ 60 trading days, and ≥ 100 fills — whichever binds last.
- Run against **live streaming data with the production code path** — not a replayed backtest. The
  #1 thing paper trading catches is a data-pipeline discrepancy (different field, different
  timestamp, different survivorship) between research and production.
- Log, per trade: signal timestamp, order timestamp, fill timestamp, decision price, arrival price,
  fill price. Compute **implementation shortfall** vs. arrival price.
- *Gates:* (a) realized slippage ≤ 1.5× modelled; (b) realized turnover within ±20% of backtest;
  (c) **signal reproduction test** — the live signal for date `t` must equal the backtest's signal
  for date `t` to within tolerance, for ≥ 99% of (date, name) pairs. Failures here are almost always
  PIT-data bugs and are the most valuable output of the whole exercise.
- Do **not** gate hard on paper-trading Sharpe: over 60 days, the standard error of Sharpe is
  ~`√(252/60)` ≈ 2.0, so it cannot distinguish 0 from 2. Gate on *process* metrics and only reject
  on Sharpe if it is catastrophically negative (< −1.0 annualized), which indicates a sign error or
  pipeline break rather than bad luck.

### 8.3 Shadow deployment and champion–challenger

**Shadow:** the challenger runs in production, generating real orders that are logged but not sent
(or sent to a simulated venue seeded by real market data). Measures: signal agreement with backtest,
latency, error rates, would-be fills vs. the actual book. Duration ≥ 1 month before any capital.

**Champion–challenger** (borrowed from credit-model governance, and the right frame here):
- The **champion** holds the capital allocation for a strategy slot.
- **Challengers** run in shadow or at a small capital carve-out (5–10%).
- Promotion requires the challenger to beat the champion on a **pre-registered metric over a
  pre-registered horizon**, with a **pre-registered minimum margin** — not just "it's ahead now."
  Suggested: challenger must beat champion's information ratio by ≥ 0.3 over ≥ 6 months, *or* show
  materially lower correlation to the existing book at comparable IR (diversification promotion).
- **Important:** champion-vs-challenger comparison is itself a multiple test. If you run 20
  challengers, one will beat the champion by chance. Apply Romano–Wolf across the challenger set, or
  restrict to ≤ 3 concurrent challengers per slot.
- Never swap the whole allocation at once: ramp challenger 10% → 25% → 50% → 100% with a gate at
  each step.

**Capital ramp schedule (G14):**
```
Month 1–3:   10% of target  (paper/shadow ran before this)
Month 4–6:   25%  if realized IR ≥ 0 and slippage within tolerance
Month 7–12:  50%  if realized IR ≥ 0.3 × expected and no gate breaches
Month 13+:   100% if realized IR ≥ 0.5 × expected
```
Each step is pre-registered; the platform enforces it. This bounds the cost of a false positive to a
small multiple of the carve-out.

### 8.4 Monitoring for alpha decay

**Baseline expectation from the literature:** McLean & Pontiff found published anomalies decay ~26%
between sample end and publication (OOS but pre-publication) and a further ~32% post-publication —
~58% total. Later global evidence is harsher. Recent (2025–2026) work on crowding and decay finds:
- Alpha decays **hyperbolically** `α(t) = K/(1+λt)` under a profit-splitting equilibrium — a better
  fit than exponential or linear for momentum (R² 0.65 vs 0.61/0.51).
- Only **mechanical** factors (momentum, short-term reversal) decay systematically; **judgment**
  factors (value, quality, investment) show no consistent decay (R² < 0.10).
- **Post-2015 acceleration:** models trained 1995–2015 over-predict remaining alpha — 0.30 predicted
  vs 0.15 actual for 2016–2024 — correlated with factor-ETF AUM growth (ρ = −0.63).
- **Crowding does not predict average returns** (crowding-timed Sharpe 0.22 vs 0.39 for naive factor
  momentum) but **does predict crashes**, heterogeneously: reversal factors 1.7–1.8× crash
  probability when crowded; momentum 0.38× (protective).

*Practical monitoring gates:*
- **SPRT / CUSUM on the live Sharpe** against `H₀: SR = SR_expected` vs `H₁: SR = 0`. This is the
  statistically correct way to ask "has it stopped working?" with minimal sample. Emit a warning at
  a likelihood ratio implying 80% confidence, a halt at 95%.
- **Rolling 6-month IC** vs. backtest IC distribution: alert if live IC falls below the backtest's
  10th percentile for 2 consecutive quarters.
- **Drawdown gate:** halt at MDD > 1.5 × backtest MDD, or time-under-water > 1.5 × backtest max TUW.
  Pre-register both.
- **Correlation-to-peers drift:** rising correlation to known factor ETFs or to your own other
  strategies = crowding; right-size down, especially for reversal-type factors.
- **Turnover / cost drift:** cost-to-gross-alpha ratio rising above its backtest value by > 50% means
  the edge is being eaten by execution — often the first symptom of crowding.
- **Decay model:** fit `α(t) = K/(1+λt)` to the live alpha series; project remaining alpha; retire
  when projected net-of-cost alpha < hurdle. Do not wait for a drawdown to make the decision.

*Rule:* pre-register the **kill criteria** at deployment time, in the registry. A strategy without
pre-registered kill criteria cannot be deployed. Post-hoc kill decisions are always made too late,
because the drawdown that should trigger them always arrives with a plausible excuse.

---

## 9. Reference gate configuration

A concrete, copy-able default. Numbers are opinionated; calibrate per asset class, but **change them
by pre-registered amendment, not per-candidate.**

```yaml
gates:
  pre_registration:
    required: true
    fields: [hypothesis, features, universe, horizon, cost_model, thresholds, kill_criteria]
    hash_locked: true

  leakage_suite:
    all_tests_must_pass: true
    cv_vs_walkforward_sharpe_gap_max: 1.0

  cost:
    break_even_cost_multiple_min: 3.0
    sharpe_at_3x_costs_min_ratio: 0.5
    cost_to_gross_alpha_max: 0.5

  capacity:
    participation_soft_cap_pct_adv: 5
    participation_hard_cap_pct_adv: 10
    deploy_fraction_of_half_sharpe_capacity_max: 0.20

  cross_validation:
    scheme: cpcv
    n_groups: 10
    k_test_groups: 2          # -> 45 splits, 9 paths
    embargo_bars: "h + max_feature_lookback + settlement_lag"
    path_sharpe_p05_min: 0.0
    path_sharpe_median_min: 0.5
    path_sharpe_iqr_max: 1.0

  walk_forward:
    required_in_addition_to_cpcv: true
    min_folds: 10
    fold_sharpe_positive_fraction_min: 0.60
    mann_kendall_trend_p_min: 0.05     # no significant trend either direction

  overfitting:
    pbo_max: 0.20
    pbo_hard_reject: 0.50
    degradation_slope_min: 0.0         # OOS-on-IS regression slope
    prob_of_loss_max: 0.20
    dsr_min: 0.95
    n_eff_method: correlation_clustering_rho_0.5
    min_backtest_years: 5
    min_independent_events: 300
    noise_sharpe_margin: 1.5           # SR >= 1.5 * sqrt(2*ln(N_eff)/years)

  attribution:
    factor_set: [MKT, SMB, HML, RMW, CMA, MOM, STR, BAB, QMJ]
    standard_errors: newey_west
    alpha_t_stat_min: 3.0
    alpha_computed_on: net_returns
    market_beta_abs_max: 0.30          # for market-neutral claims
    factor_r2_max: 0.70

  regime:
    min_distinct_vol_regimes: 3
    min_crisis_windows: 2
    max_single_regime_pnl_share: 0.50
    worst_regime_sharpe_min: -0.50
    mandatory_windows: [2007-08, 2008-09, 2020-03, 2022, 2024-08]

  robustness:
    param_perturb_pct: [10, 20]
    param_perturb_median_sharpe_min_ratio: 0.70
    param_cliff_max_single_step_drop: 0.40
    universe_subset_draws: 100
    universe_subset_median_sharpe_min_ratio: 0.70
    max_single_name_pnl_share: 0.20
    max_single_sector_pnl_share: 0.40
    bootstrap: stationary
    bootstrap_block_length: politis_white_auto
    bootstrap_reps: 1000
    bootstrap_sharpe_p05_min: 0.0

  multiple_testing:
    family_test: romano_wolf_stepdown
    family_alpha: 0.05
    bootstrap_reps: 5000
    fdr_control: bhy
    fdr_target: 0.10
    trial_budget_per_family: 1000
    trial_budget_first_pass: 100

  holdout:
    fraction: 0.20
    max_evaluations_per_strategy: 1
    enforced_by: platform_ledger

  deployment:
    paper_trading_min_days: 60
    paper_trading_min_fills: 100
    signal_reproduction_match_min: 0.99
    slippage_vs_model_max_ratio: 1.5
    shadow_min_days: 30
    expected_sharpe: "min(dsr_implied_sr, 0.5 * backtest_sr)"
    capital_ramp: [0.10, 0.25, 0.50, 1.00]
    max_concurrent_challengers_per_slot: 3

  kill_criteria:                        # pre-registered, platform-enforced
    mdd_multiple_of_backtest_mdd: 1.5
    time_under_water_multiple: 1.5
    sprt_halt_confidence: 0.95
    rolling_ic_below_backtest_p10_quarters: 2
```

### 9.1 Failure modes of the gate stack itself

Be honest about these:
- **Gate-hacking.** An agent optimizing against the gates is doing multiple testing *on the gates*.
  Mitigations: the trial counter must include gate-failing runs (it does); some gates should be
  **stochastic** (e.g., randomly select which 3 of 8 crisis windows are used for a given candidate,
  from a platform-held seed the agent cannot see); and keep a true **sealed holdout** that no gate
  tuning ever touches.
- **Over-rejection.** With all gates at these thresholds, the pass rate will be very low
  (single-digit percent of candidates at best, and that is the *intended* outcome). The failure mode
  is not "too strict" but "so strict that people route around it." Make the registry the only path
  to capital, and make the gates fast enough that they don't create an incentive to skip them.
- **Gates measure what's measurable.** None of this catches a strategy that is economically
  incoherent but statistically clean. Keep an economic-rationale requirement in pre-registration —
  a one-paragraph mechanism for *why* the edge should exist and *who is on the other side*. Agents
  should be required to produce it, and it should be reviewable.
- **Correlated gates.** DSR, PBO and Romano–Wolf are all multiple-testing corrections; passing all
  three is less independent evidence than it looks. Treat them as one gate with three views.

---

## 10. Sources

**López de Prado framework**
- Bailey, Borwein, López de Prado, Zhu — *The Probability of Backtest Overfitting* (CSCV algorithm, PBO, degradation): https://www.davidhbailey.com/dhbpapers/backtest-prob.pdf
- Bailey & López de Prado — *The Deflated Sharpe Ratio*: https://www.davidhbailey.com/dhbpapers/deflated-sharpe.pdf | SSRN: https://papers.ssrn.com/sol3/papers.cfm?abstract_id=2460551
- López de Prado — *Deflating the Sharpe Ratio / Minimum Track Record Length* (slides): http://boston.qwafafew.org/wp-content/uploads/sites/4/2017/01/Lopez_de_Prado_Sharpe.pdf
- López de Prado — *The 10 Reasons Most Machine Learning Funds Fail* (GARP): https://www.garp.org/hubfs/Whitepapers/a1Z1W0000054x6lUAA.pdf
- *Advances in Financial Machine Learning* (Wiley, table of contents): https://toc.library.ethz.ch/objects/pdf03/e01_978-1-119-48208-6_01.pdf
- Deflated Sharpe ratio — formulas and limitations: https://en.wikipedia.org/wiki/Deflated_Sharpe_ratio
- Purged cross-validation — purging, embargo, CPCV path formulas: https://en.wikipedia.org/wiki/Purged_cross-validation
- RiskLab AI — Backtesting through Cross-Validation (CPCV): https://www.risklab.ai/research/backtesting/backtesting_cross_validation
- QuantInsti — Cross-validation in finance: purging, embargoing, combinatorial: https://blog.quantinsti.com/cross-validation-embargo-purging-combinatorial/
- Quantoisseur — CPCV explained (worked example): https://quantoisseur.com/2019/11/05/combinatorial-purged-cross-validation-explained/

**Evidence and critique of CPCV / CV methods**
- Arian, Norouzi M., Seco — *Backtest Overfitting in the Machine Learning Era: A Comparison of Out-of-Sample Testing Methods in a Synthetic Controlled Environment*, Knowledge-Based Systems 305 (2024): https://www.sciencedirect.com/science/article/abs/pii/S0950705124011110 | SSRN: https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4686376 | ACM: https://dl.acm.org/doi/10.1016/j.knosys.2024.112477
- Palomar — *Portfolio Optimization*, §8.3 The Dangers of Backtesting: https://portfoliooptimizationbook.com/book/8.3-dangers-backtesting.html
- *Interpretable Hypothesis-Driven Trading: A Rigorous Walk-Forward Validation Framework for Market Microstructure Signals*, arXiv 2512.12924: https://arxiv.org/html/2512.12924v1

**Triple-barrier, meta-labeling, fractional differentiation, sampling**
- Singh & Joubert (Hudson & Thames) — *Does Meta-Labeling Add to Signal Efficacy?*: https://hudsonthames.org/wp-content/uploads/2022/04/Does-Meta-Labeling-Add-to-Signal-Efficacy.pdf | https://hudsonthames.org/does-meta-labeling-add-to-signal-efficacy-triple-barrier-method/
- Joubert — *Meta-Labeling: Theory and Framework*, JFDS: https://papers.ssrn.com/sol3/papers.cfm?abstract_id=4032018 | code: https://github.com/hudson-and-thames/meta-labeling
- Hudson & Thames — Fractional differentiation: https://hudsonthames.org/fractional-differentiation/
- *Fractional differentiation and its use in machine learning*, Springer: https://link.springer.com/article/10.1007/s12572-021-00299-5
- *Comparative analysis of financial data differentiation techniques using LSTM* (arXiv 2505.19243): https://pith.science/paper/2505.19243
- Hudson & Thames — Sequential bootstrapping in Python: https://hudsonthames.org/bagging-in-financial-machine-learning-sequential-bootstrapping-python/
- mlfinpy — Data sampling (uniqueness, sequential bootstrap) docs: https://mlfinpy.readthedocs.io/en/latest/Sampling.html

**Multiple testing / data snooping**
- Harvey, Liu & Zhu — *…and the Cross-Section of Expected Returns* (NBER w20592): https://www.nber.org/system/files/working_papers/w20592/w20592.pdf | SSRN: https://papers.ssrn.com/sol3/papers.cfm?abstract_id=2249314
- Harvey & Liu — *Backtesting* (haircut Sharpe; Bonferroni/Holm/BHY), CME edition: https://www.cmegroup.com/education/files/backtesting.pdf | SSRN: https://papers.ssrn.com/sol3/papers.cfm?abstract_id=2345489
- Harvey & Liu — *Practical Applications of Backtesting*: https://people.duke.edu/~charvey/Media/2016/Practical_applications_backtesting.pdf
- Harvey & Liu — *False (and Missed) Discoveries in Financial Economics*: https://papers.ssrn.com/sol3/papers.cfm?abstract_id=3073799
- *An Evaluation of Alternative Multiple Testing Methods for Finance Applications* (FDP Institute, 2024): https://www.fdpinstitute.org/resources/FDP%203.0/2024-Q2/Topics%20in%20Financial%20Data%20Science/9.3%20An%20Evaluation%20of%20Alternative%20Multiple%20Testing%20Methods.pdf
- Hsu & Kuan — *Re-Examining the Profitability of Technical Analysis with White's Reality Check and Hansen's SPA Test*: https://papers.ssrn.com/sol3/papers.cfm?abstract_id=685361
- Hsu, Hsu & Kuan — *Testing the predictive ability of technical analysis using a new stepwise test without data snooping bias* (Step-SPA): https://homepage.ntu.edu.tw/~ckuan/pdf/Step-SPA-20090720.pdf
- R implementation of haircut Sharpe: https://rdrr.io/github/braverock/quantstrat/man/SharpeRatio.haircut.html
- False discovery rate (BH / BHY): https://en.wikipedia.org/wiki/False_discovery_rate

**Sharpe ratio inference**
- Two Sigma — *Sharpe Ratio: Estimation, Confidence Intervals, and Hypothesis Testing*: https://www.twosigma.com/wp-content/uploads/sharpe-tr-1.pdf
- Portfolio Optimizer — PSR: bias adjustment, CIs, hypothesis testing, MinTRL: https://portfoliooptimizer.io/blog/the-probabilistic-sharpe-ratio-bias-adjustment-confidence-intervals-hypothesis-testing-and-minimum-track-record-length/
- Lo — *The Statistics of Sharpe Ratios* / non-i.i.d. moments: https://www.researchgate.net/publication/228139699_The_Statistics_of_Sharpe_Ratios

**Execution, impact, market microstructure**
- *Realistic Market Impact Modeling for Reinforcement Learning Trading Environments* (square-root law, Almgren–Chriss decomposition, decay, participation caps), arXiv 2603.29086: https://arxiv.org/html/2603.29086
- Almgren–Chriss model: https://en.wikipedia.org/wiki/Almgren%E2%80%93Chriss_model
- Almgren et al. — *Direct Estimation of Equity Market Impact*: https://www.researchgate.net/publication/228754794_Direct_Estimation_of_Equity_Market_Impact
- Rosenbaum, Souilmi et al. — *Bridging the Reality Gap in Limit Order Book Simulation*, arXiv 2603.24137: https://arxiv.org/html/2603.24137v1
- Moallemi & Yuan — *A Model for Queue Position Valuation in a Limit Order Book*: https://papers.ssrn.com/sol3/Delivery.cfm/SSRN_ID2996221_code2323791.pdf?abstractid=2996221
- hftbacktest — probabilistic queue position models (implementable): https://hftbacktest.readthedocs.io/en/latest/tutorials/Probability%20Queue%20Models.html
- O'Neill — *Capacity Analysis for Equity Funds*: https://openresearch-repository.anu.edu.au/server/api/core/bitstreams/8038e723-975d-4222-9608-9a3a544d8d34/content
- *The Capacity of Trading Strategies*: https://www.aeaweb.org/conference/2016/retrieve.php?pdfid=21020&tk=BGQnasd4

**Leakage, biases, data**
- *Look-Ahead Bias in Quant Research: How to Detect and Eliminate*: https://ariaanalyst.pro/blog/look-ahead-bias-quant
- *A Practical Guide To The Backtesting Mistakes That Kill Quant Strategies*: https://hedgefundalpha.com/education/backtesting-mistakes-kill-quant-strategies-guide/
- StarQube — Point-in-time data: https://starqube.com/point-in-time-data/ | critical pitfalls of backtesting: https://starqube.com/backtesting-investment-strategies/
- CFA — Problems in Backtesting and Biases in Data: https://analystprep.com/study-notes/cfa-level-2/problems-in-backtesting/
- *Data leakage detection in machine learning code* (PeerJ CS, 2025): https://peerj.com/articles/cs-2730/
- *A taxonomy for detecting and preventing temporal data leakage in machine learning* (PLOS One, 2025): https://journals.plos.org/plosone/article?id=10.1371%2Fjournal.pone.0340167
- *Hidden Leaks in Time Series Forecasting: How Data Leakage Affects LSTM Evaluation*, arXiv 2512.06932: https://arxiv.org/html/2512.06932v1
- FlashAlpha — Point-in-time options data: https://flashalpha.com/articles/point-in-time-options-data-backtest-integrity

**Regimes, robustness, synthetic data**
- *Adaptive Hierarchical Hidden Markov Models for Structural Market Change* (JRFM, 2026): https://www.mdpi.com/1911-8074/19/1/15
- *A forest of opinions: multi-model ensemble-HMM voting for market regime shift detection and trading*: https://www.aimspress.com/article/id/69045d2fba35de34708adb5d
- Politis & White — *Automatic Block-Length Selection for the Dependent Bootstrap*: https://public.econ.duke.edu/~ap172/Politis_White_2004.pdf
- Stationary bootstrap (Politis–Romano) reference implementation: https://metricgate.com/docs/stationary-bootstrap-politis-romano/
- *Generation of synthetic financial time series by diffusion models*, Quantitative Finance (2025): https://www.tandfonline.com/doi/full/10.1080/14697688.2025.2528697 | arXiv 2410.18897: https://arxiv.org/abs/2410.18897
- *A diffusion-based generative model for financial time series*, arXiv 2507.19003: https://arxiv.org/pdf/2507.19003
- *High-Quality Synthetic Financial Time-Series using a GAN–Diffusion Framework*, arXiv 2605.27113: https://arxiv.org/abs/2605.27113
- *StockSim: A Dual-Mode Order-Level Simulator for Evaluating Multi-Agent LLMs in Financial Markets*, arXiv 2507.09255: https://pith.science/paper/2507.09255

**Alpha decay, crowding, live deployment**
- McLean & Pontiff — *Does Academic Research Destroy Stock Return Predictability?*: https://www.semanticscholar.org/paper/8b4aa199805cc655a86dfc88909b107c56d3327d
- Jacobs & Müller — *Anomalies across the globe: Once public, no longer existent?*, JFE: https://www.sciencedirect.com/science/article/abs/pii/S0304405X19301618
- *Not All Factors Crowd Equally: Modeling, Measuring, and Trading on Alpha Decay*, arXiv 2512.11913: https://arxiv.org/html/2512.11913v1
- *What Drives Anomaly Decay?* (AEA 2024): https://www.aeaweb.org/conference/2024/program/paper/SNQSBFkB
- Champion/Challenger model testing in production: https://theneuralbase.com/ai-for-finance/learn/intermediate/champion-challenger/
- Grinold & Kahn / Clarke, de Silva & Thorley — *Portfolio Constraints and the Fundamental Law of Active Management* (transfer coefficient): https://www.tandfonline.com/doi/abs/10.2469/faj.v58.n5.2468

**Agentic strategy discovery (and its current validation gap)**
- *QuantEvolve: Automating Quantitative Strategy Discovery through Multi-Agent Evolutionary Framework*, arXiv 2510.18569: https://arxiv.org/html/2510.18569v1
- *Automate Strategy Finding with LLM in Quant Investment*, arXiv 2409.06289 / EMNLP Findings 2025: https://aclanthology.org/2025.findings-emnlp.1005/
