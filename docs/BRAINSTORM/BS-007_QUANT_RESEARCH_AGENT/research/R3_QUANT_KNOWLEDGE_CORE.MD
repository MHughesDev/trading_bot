# BS-007 / R3 — Quant knowledge core (seed content for the core skills)

> **Informative.** This was doc 06 before the 2026-09-11 restructure. It is the **seed
> content for the core skills** (13 §5, 12 §2 L1). The cards here become `SKILL.md`
> files, one per topic or group, plus the QC rules in 11 §7.
>
> Where they conflict, **11 EVALUATION_STANDARD wins**: HARQ over HAR, EDGE over
> Roll/Corwin–Schultz, equal-weight combination by default, theme-posterior alongside
> DSR.
>
> Old refs map as in research/R1's header: 03 → 14, 04 → 03/04/16, 05 → 12, 08 §x →
> 03/05–11. Part D's "Today" column reflects the 2026-09-10 baseline (01); Sets M–O
> change it.

Status: draft. These are **seed cards**: the knowledge the agent should be able to
retrieve, each tied to the tool that computes it (03) and its platform status (01). The
math is condensed. Every formula states its assumptions; the agent must too.

Conventions: log returns $r_t=\ln(P_t/P_{t-1})$. Crypto trades 24/7: periods per year
$=365\cdot 86400/\Delta t_{\text{sec}}$ (1m → 525,600; 1h → 8,760). Equities: 252 days.
$P$ = physical (real-world) measure, $Q$ = pricing measure. **Almost everything this
platform does lives under $P$.**

---

## Part A — Stochastic foundations, and why each matters here

**A1. Filtration = `available_time`.** $(\Omega,\mathcal F,(\mathcal F_t),P)$; $\mathcal F_t$ is
the information available at $t$. A feature is admissible iff it is $\mathcal F_t$-adapted.
A label over horizon $h$ is $\mathcal F_{t+h}$-measurable. Leakage means using a
non-adapted input. *Platform:* `timestamp.rs` sorts replay by `available_time`.
*Pitfall:* smoothed HMM states, full-sample scalers, and models trained on the test window
are all non-adapted.

**A2. P&L is a discrete Itô integral.** $\Pi_T=\sum_i\theta_{t_i}(S_{t_{i+1}}-S_{t_i})$ with
$\theta$ **predictable**, i.e. decided with information up to $t_i$. Computing $\theta$ from
the bar that closes at $t_{i+1}$ and filling at that same close makes the sum anticipating
(not an Itô sum): look-ahead. *Platform:* this is gap G-14 (fill timing) stated precisely.
*Tool:* `run_robustness_suite` "delay-by-one-bar" preset.

**A3. Doob decomposition: alpha is the predictable part.** $\Delta S_{t+1}=\mathbb E[\Delta S_{t+1}\mid\mathcal F_t]+\varepsilon_{t+1}$.
Then $\mathbb E[\Pi]=\mathbb E[\sum\theta_t\,\mathbb E[\Delta S_{t+1}\mid\mathcal F_t]]$: expected
P&L is the covariance of position with conditional drift. Grinold's fundamental law,
$IR\approx IC\sqrt{\text{breadth}}$, is the same idea with a skill correlation $IC$ and
independent bets. *Tool:* `signal_forensics` (IC decay).

**A4. Optional stopping: exits cannot create edge.** If $S$ is a martingale and $\tau$ a
bounded stopping time, $\mathbb E[S_\tau]=S_0$. Any stop/target/time exit on a driftless
price has zero expected P&L before costs and negative after. Exits reshape the
distribution (win rate, skew) but not its mean. That's why win rate is gamed by tight
targets and wide stops, and why it is never a primary objective. *Platform:* Objective
excludes win rate; G-04 exits. *Playbook:* "win rate high, expectancy negative" → the edge
is in entries or there is none.

**A5. Brownian motion and quadratic variation.** Independent Gaussian increments,
$\text{Var}(W_t-W_s)=t-s$; $[W]_t=t$. Realised variance $\sum r_i^2\to\int\sigma^2dt$ as
sampling gets finer (in the absence of microstructure noise). $\sqrt{t}$ scaling of
volatility and Sharpe holds only for uncorrelated increments.

**A6. Merton's estimation asymmetry (the most useful fact in this pack).** With $\sigma$
known, $\text{SE}(\hat\mu)=\sigma/\sqrt T$, **independent of sampling frequency**, whereas
$\hat\sigma$ improves with frequency. At $\sigma=60\%$/yr (crypto), one year of data gives
$\text{SE}(\hat\mu)\approx 60\%$/yr; you need ~36 years for 10%. Direction (drift) is
nearly unidentifiable from short histories; volatility is easy. **Implication for the
agent:** prefer volatility-aware sizing and risk structure over directional forecasting,
and distrust any drift estimate from months of data.

**A7. Itô's lemma.** For $dX=a\,dt+b\,dW$ and smooth $f(t,x)$:
$df=(f_t+af_x+\tfrac12b^2f_{xx})dt+bf_x\,dW$. Multivariate:
$df=f_tdt+\sum_if_idX^i+\tfrac12\sum_{ij}f_{ij}\,d\langle X^i,X^j\rangle$. With jumps, add
$f(X_t)-f(X_{t-})$ at each jump. Uses here: log-price dynamics, variance drag, and
correct simulator transformations. *Pitfall:* the lemma is not a signal. An "Itô
indicator" needs a separately defined observable and hypothesis.

**A8. Volatility drag and Kelly.** GBM: $d\ln S=(\mu-\tfrac12\sigma^2)dt+\sigma dW$. The
geometric growth of a constant-fraction position $f$ is
$g(f)=r+f(\mu-r)-\tfrac12f^2\sigma^2$, maximised at Kelly $f^*=(\mu-r)/\sigma^2$
(multi-asset: $w^*=\Sigma^{-1}(\mu-r)$). Fraction $c$ of Kelly earns $c(2-c)$ of the maximum
excess growth: half-Kelly keeps 75% of growth at half the volatility. Because $\mu$ is
poorly estimated (A6), full Kelly is reckless. *Platform:* `RiskSizing` models, `Sizing`
node; blocked by G-05 in backtests.

**A9. Variance swap identity.** From Itô on $\ln S$:
$\int_0^T\sigma_t^2dt=2\int_0^T dS_t/S_t-2\ln(S_T/S_0)$. Realised variance is replicable with
a log contract plus continuous delta hedging, the basis of VIX-style indices.

**A10. SDE catalogue (under $P$), with what each explains and its null use.**

| Process | Dynamics | Explains | Estimation from bars | Use as a null |
|---|---|---|---|---|
| GBM | $dS=\mu Sdt+\sigma SdW$ | Benchmark only | trivial | "Pure noise + drift" null; too easy to beat |
| OU | $dX=\kappa(\theta-X)dt+\eta dW$ | Mean-reverting spreads | AR(1): $b=e^{-\kappa\Delta}$, $\kappa=-\ln b/\Delta$, $\theta=a/(1-b)$, half-life $\ln2/\kappa$; Kendall bias $\approx-(1+3b)/T$ | Spread null |
| GARCH(1,1)-t | $\sigma_t^2=\omega+\alpha\varepsilon_{t-1}^2+\beta\sigma_{t-1}^2$ | Vol clustering, fat tails | MLE (`arch`) | **Exists** (`SyntheticGarch`): "same clustering, zero drift" is the right null for trend strategies |
| GJR / EGARCH | asymmetric terms | Leverage effect | MLE | Asymmetric-vol null |
| Heston | $dv=\kappa(\theta-v)dt+\xi\sqrt v\,dW^2$, $\rho$ | Stochastic vol, skew | Hard from bars; option calibration is a separate task | Scenario generator; simulate with QE (Andersen 2008) |
| Merton jump | $dS/S=(\mu-\lambda k)dt+\sigma dW+(J-1)dN$ | Jumps, tails | Weakly identified from short samples | Crash-scenario null |
| Hawkes | $\lambda(t)=\mu+\sum\alpha e^{-\beta(t-t_i)}$, branching $n=\alpha/\beta<1$ | Clustered events | MLE on event times (e.g. >3σ moves) | Event-clustering null |
| Regime switching / jump model | Markov-modulated params | Regimes | EM / jump-penalised clustering (FEAT-003) | `RegimeBlock` exists |
| Rough vol | $\log\sigma$ ~ fBM, $H\approx0.1$ | Vol roughness | Variogram of log-RV: $\mathbb E\lvert\Delta\log\sigma\rvert^q\propto\Delta^{qH}$ | Research only |

*Tools:* `fit_process`, `simulate_paths`, and a proposed generic `ProcessNull{artifact}`
that generalises `SyntheticGarch`.

**A11. Simulation numerics.** Euler–Maruyama (strong order 0.5, weak 1); Milstein (strong
1); exact schemes for GBM and OU:
$X_{t+\Delta}=\theta+(X_t-\theta)e^{-\kappa\Delta}+\eta\sqrt{(1-e^{-2\kappa\Delta})/2\kappa}\,Z$;
full truncation or QE for CIR/Heston. **Brownian-bridge barrier crossing** between grid
points: $P(\max\ge b)=\exp(-2(b-x)(b-y)/(\sigma^2\Delta))$. Use it for intrabar stop/target
checks on simulated paths. On historical OHLC, when both stop and target lie inside
$[L,H]$ the order is unidentified: use a conservative (stop-first) rule and flag it.
Variance reduction: antithetic, control variates, common random numbers, Sobol QMC.
Record seeds (`RunConfig.seed` exists but is ignored today, G-06).

**A12. Girsanov / change of measure.** $dQ/dP=\exp(-\int\lambda dW-\tfrac12\int\lambda^2dt)$
shifts drift by $\lambda$ (Novikov condition). Uses: risk-neutral pricing, and
**importance sampling of rare events** (drawdown > X probability) in `TradeMonteCarlo` and
`SyntheticPaths` studies. *Pitfall:* $Q$-probabilities are not real-world probabilities.

**A13. Feynman–Kac and Kolmogorov.** $u(t,x)=\mathbb E[e^{-\int r}g(X_T)\mid X_t=x]$ solves
$u_t+au_x+\tfrac12b^2u_{xx}-ru=0$; densities obey Fokker–Planck
$\partial_tp=-\partial_x(ap)+\tfrac12\partial_{xx}(b^2p)$. First-passage with drift:
$P(\max_{s\le T}(\mu s+\sigma W_s)\ge b)=\Phi\!\big(\tfrac{-b+\mu T}{\sigma\sqrt T}\big)+e^{2\mu b/\sigma^2}\Phi\!\big(\tfrac{-b-\mu T}{\sigma\sqrt T}\big)$.
*Platform:* **Kalshi contracts are digital and touch options.** A fair YES price under a
fitted $P$-model is a well-posed quantity to compare with the market price. *Tool:*
`get_prediction_markets` + `fit_process`.

**A14. Martingale representation, Clark–Ocone.** $F=\mathbb E[F]+\int_0^T\mathbb E[D_tF\mid\mathcal F_t]\,dW_t$:
the hedge is the conditional expectation of the Malliavin derivative. Bridge to A15.

**A15. Malliavin calculus → Monte Carlo sensitivities.** $D_t$ differentiates a path
functional with respect to the driving noise (e.g. $D_tf(W_T)=f'(W_T)\mathbf 1_{t\le T}$);
the Skorokhod integral $\delta$ is its adjoint: $\mathbb E\langle DF,u\rangle=\mathbb E[F\,\delta(u)]$.
Integration by parts gives $\partial_\theta\mathbb E[\varphi(X_T)]=\mathbb E[\varphi(X_T)\,\pi_\theta]$
with a weight $\pi$ that does not depend on $\varphi$, so it works for **discontinuous
payoffs** (digitals) where pathwise derivatives fail and finite differences are noisy
(Fournié et al. 1999). GBM weights (they coincide with likelihood-ratio weights for
European payoffs; Malliavin's advantage is path-dependent payoffs and models without
tractable densities):

| Greek | Weight $\pi$ (multiply $e^{-rT}\varphi(S_T)$) |
|---|---|
| Delta | $W_T/(S_0\sigma T)$ |
| Vega | $W_T^2/(\sigma T)-W_T-1/\sigma$ |
| Gamma | $\big(W_T^2/(\sigma T)-W_T-1/\sigma\big)/(S_0^2\sigma T)$ |
| Rho | $W_T/\sigma-T$ |

Estimator choice: finite differences with CRN (simple, biased), pathwise (unbiased for
Lipschitz payoffs, fails on digitals), likelihood ratio (needs a density, high variance
for small $\sigma\sqrt T$), Malliavin (no density needed, localisation reduces variance),
AAD (many Greeks cheaply). **Platform idea, "strategy scenario Greeks":** for a strategy
evaluated on simulated paths from a fitted process,
$\partial_\theta\mathbb E[\text{metric}]=\mathbb E[\text{metric}\cdot\partial_\theta\log p_\theta(\text{path})]$
(score function). This tells the agent which market property the strategy is really
betting on (vol level, vol-of-vol, jump intensity). CRN finite differences serve as the
cross-check. Not a replacement for backprop, and not knowledge of future shocks.

**A16. Stochastic control.** HJB: $\sup_u\{\mathcal L^uV+f\}=0$. Merton portfolio:
$\pi^*=(\mu-r)/(\gamma\sigma^2)$ ($\gamma=1$ is Kelly). Almgren–Chriss execution:
$x(t)=X\sinh(\kappa(T-t))/\sinh(\kappa T)$, $\kappa=\sqrt{\lambda\sigma^2/\eta}$.
Avellaneda–Stoikov market making: reservation price $r=s-q\gamma\sigma^2(T-t)$. Optimal
stopping for OU spreads gives analytic entry/exit bands with costs (Bertram 2010; Leung &
Li 2015). *Platform:* candidate `change_exit` move to optimal-stopping bands; execution
research on paper engines.

**A17. Filtering.** Kalman (linear-Gaussian): predict $\hat x^-=F\hat x$, $P^-=FPF^\top+Q$;
update $K=P^-H^\top(HP^-H^\top+R)^{-1}$. Classic use: time-varying hedge ratio
$y_t=\beta_tx_t+\varepsilon_t$, $\beta_t=\beta_{t-1}+w_t$ (baseline: rolling OLS). HMM forward
algorithm gives **filtered** probabilities; backtests may only use filtered, never
smoothed. Particle filters handle nonlinear models. Statistical jump models add a
switching penalty for persistent regimes (FEAT-003 §10).

**A18. Path signatures.** $S(X)_{s,t}=(1,\int dX,\iint dX\otimes dX,\dots)$; Chen's identity;
linear functionals of signatures approximate continuous path functionals. The level-2
terms of (time, log-price) carry drift and quadratic variation; the Lévy area of (price,
volume) captures lead–lag. Dimension to level $m$: $(d^{m+1}-1)/(d-1)$. Candidate feature
family for models; needs ablation against plain lags.

**A19. BSDEs and deep BSDE.** $Y_t=\xi+\int_t^Tf\,ds-\int_t^TZ\,dW$; nonlinear Feynman–Kac;
$Z$ is the hedge. Deep BSDE solvers (Han–Jentzen–E 2018) handle high-dimensional PDEs.
Specialised; only with a concrete pricing/control problem.

---

## Part B — Quant toolkit (P-measure, bar data)

**B1. Stylized facts** (Cont 2001): heavy tails, near-zero linear autocorrelation, volatility
clustering (slow decay of $|r|$ autocorrelation), leverage effect, aggregational
Gaussianity, volume–volatility correlation. Tests: Jarque–Bera, Ljung–Box on $r$ and $r^2$,
ARCH-LM, Hill tail index. A synthetic null that lacks these is too easy to beat.
*Tool:* `describe_returns`.

**B2. Range volatility estimators** (per period):
Parkinson $\tfrac{1}{4\ln2}(\ln\tfrac HL)^2$ (~5× close-close efficiency); Garman–Klass
$\tfrac12(\ln\tfrac HL)^2-(2\ln2-1)(\ln\tfrac CO)^2$ (~7×); Rogers–Satchell
$\ln\tfrac HC\ln\tfrac HO+\ln\tfrac LC\ln\tfrac LO$ (drift-robust); Yang–Zhang
$\sigma_o^2+k\sigma_c^2+(1-k)\sigma_{RS}^2$, $k=0.34/(1.34+\tfrac{n+1}{n-1})$. *Platform:*
Parkinson and GK exist in `crates/features` (not in backtests); RS and YZ are missing.

**B3. Realised measures.** $RV=\sum r_i^2$; bipower $BV=\tfrac\pi2\sum|r_i||r_{i-1}|$ is robust
to jumps, so $(RV-BV)/RV$ is the jump share (BNS test). Microstructure noise biases 1m RV;
prefer 5m or realised kernels. HAR-RV (Corsi 2009):
$RV_{t+1}=\beta_0+\beta_dRV_t+\beta_wRV^{(w)}_t+\beta_mRV^{(m)}_t$, a hard-to-beat baseline.
Evaluate volatility forecasts with **QLIKE** $RV/\hat\sigma^2-\ln(RV/\hat\sigma^2)-1$ (robust to
a noisy proxy; Patton 2011). *Tool:* `volatility_report`; `Volatility` target exists.

**B4. GARCH facts.** Persistence $\alpha+\beta$; shock half-life $\ln0.5/\ln(\alpha+\beta)$; EWMA
$\lambda=0.94$ (RiskMetrics) is the simple baseline. Evaluate VaR with Kupiec and
Christoffersen (both exist in `scoring.py`). *Platform caveat:* GARCH currently serves
unconditional σ (01 §6).

**B5. Memory and stationarity.** Variance ratio $VR(q)=\text{Var}(r^{(q)})/(q\,\text{Var}(r))$,
Lo–MacKinlay $z=(VR-1)/\sqrt{2(2q-1)(q-1)/(3qT)}$ (use the heteroskedasticity-robust form).
Hurst via R/S (Anis–Lloyd small-sample correction) or DFA: $H<0.5$ anti-persistent, $>0.5$
persistent. **Hurst of returns (≈0.5) and Hurst of log-volatility (≈0.1) are different
objects**; say which one. ADF (null: unit root) and KPSS (null: stationary) are run
together. Fractional differencing keeps memory while achieving stationarity (López de
Prado). *Tool:* `regime_report`, `test_stationarity`.

**B6. Mean reversion and relative value.** Engle–Granger: OLS hedge, then an ADF on the
residual **with EG critical values**. Johansen for more than 2 series. OU fit → half-life
→ bands. Correlation ≠ cointegration; beware spurious regression and look-ahead hedge
ratios. *Platform:* strategies are single-instrument (OQ-040), so pairs need a synthetic
spread instrument or an architecture decision.

**B7. Trend.** Time-series momentum (Moskowitz–Ooi–Pedersen 2012), vol-scaled position
$\text{sign}(r_{t-L,t})\cdot\sigma^*/\hat\sigma_t$. Moving-average crossovers are linear
filters, weighted sums of past returns with hump-shaped weights (Levine–Pedersen 2016);
EMA($n$) lag is $(n-1)/2$ bars. Nearby (fast, slow) pairs are highly correlated, so the EMA
parameter surface is smooth and plateaus are expected.

**B8. Microstructure from bars.** Roll $s=2\sqrt{-\text{Cov}(\Delta p_t,\Delta p_{t-1})}$;
Corwin–Schultz high-low spread; Amihud $\overline{|r_t|/\text{\$Vol}_t}$; bulk volume
classification $V_{buy}=V\,\Phi(\Delta p/\sigma_{\Delta p})$; order-flow imbalance
(Cont–Kukanov–Stoikov 2014) needs L2. Square-root impact $\approx Y\sigma_d\sqrt{Q/V_d}$.
*Use:* data-driven `cost_model_ref` per instrument (G-06) and capacity estimates.
*Tool:* `liquidity_cost_report`.

**B9. Sizing and portfolio.** Volatility targeting $w=\sigma^*/\hat\sigma$ (Moreira–Muir 2017;
out-of-sample evidence is mixed: Cederburg et al. 2020). Fractional Kelly (A8).
Drawdown control: exposure proportional to the cushion above a drawdown floor
(Grossman–Zhou). Ledoit–Wolf shrinkage; HRP (López de Prado 2016); risk parity;
Marchenko–Pastur noise band $\lambda_\pm=\sigma^2(1\pm\sqrt{N/T})^2$; CVaR minimisation as an
LP (Rockafellar–Uryasev). **Compare sized vs unsized strategies at matched risk**; lower
exposure alone is not an improvement.

**B10. Labels and sampling** (López de Prado, AFML): triple barrier (exists, close path
only); **meta-labeling**: a primary rule picks side, a secondary model predicts $P(\text{win})$
and sizes or filters. This is the natural bridge between the platform's rule strategies
and its model suite: `Sizing` node with $f=p-(1-p)/b$ (Kelly for a binary bet with payoff
ratio $b$). Sample weighting by label uniqueness, sequential bootstrap, event-based
sampling (CUSUM filter), and volume/dollar bars all apply.

**B11. Inference at scale.** Sharpe SE (non-normal; Lo 2002, Mertens 2002):
$\sqrt{(1+\tfrac12SR^2-\gamma_3SR+\tfrac{\gamma_4-3}{4}SR^2)/T}$.
PSR $=\Phi\big((\widehat{SR}-SR^*)\sqrt{T-1}/\sqrt{1-\gamma_3\widehat{SR}+\tfrac{\gamma_4-1}4\widehat{SR}^2}\big)$;
DSR sets $SR^*$ to the expected maximum of $N$ trials (exists). MinTRL. Multiple testing:
Holm, Benjamini–Hochberg, White's Reality Check, Hansen SPA, Romano–Wolf stepdown,
Harvey–Liu–Zhu $t>3$. **Effective number of trials:** 200 near-duplicate sweep members are
not 200 independent tests. Cluster trials by return correlation before DSR. Stationary
bootstrap (Politis–Romano) with automatic block length (Politis–White). Annualising 1m
Sharpe multiplies by $\sqrt{525600}\approx725$ and assumes independence; don't.

**B12. Derivatives and prediction markets.** Black–Scholes $d_{1,2}$, Greeks, put–call
parity, IV inversion (Brent or Jäckel), SVI smile, butterfly/calendar no-arbitrage;
digital $=-\partial C/\partial K$. Kalshi YES price ≈ implied probability net of fees;
score model probabilities with Brier and log loss; favourite–longshot bias (Snowberg &
Wolfers 2010). *Platform:* Tradier gives per-contract quotes (no chain or IV computed);
Kalshi prices are collected but not agent-reachable.

**B13. Crypto specifics.** No overnight gap (Yang–Zhang's overnight term ≈ 0); weekend
liquidity; perpetual funding ≈ premium index plus a clamped interest component; basis;
venue fragmentation; stablecoin risk.

---

## Part C — ML for finance

**C1. Target first, architecture second.** Map the question to `TargetField`: return
distribution → quantile/CRPS; volatility → QLIKE vs HAR; event → calibrated
classification; action → `TradeDecision` plus meta-labeling.

**C2. Hard baselines.** LightGBM-quantile on lagged features; HAR for volatility; naive and
seasonal-naive (exist in `scoring.py`); DLinear (Zeng et al. 2023, which showed simple
linear models beating many Transformer forecasters).

**C3. Sequence models, honestly.** Real sequence models need windowed tensors
$(N,L,F)$ from `ModelInput.lookback` (already in the schema) and a bundle header with
lookback and layout. Today the LSTM sees length-1 sequences (01 §6). Candidates: TCN,
LSTM/GRU, PatchTST (Nie et al. 2023), TFT (Lim et al.; needs known-future covariates, which
for crypto are mostly calendar), N-BEATS/N-HiTS, DeepAR, TSMixer, SSM/Mamba. Foundation
models (Chronos, TimesFM, Moirai) are useful zero-shot baselines but may have seen the
test period in pretraining (contamination).

**C4. Losses.** Pinball/CRPS (exist); Gaussian/Student-t NLL; fix quantile crossing;
decision-aware losses such as directly optimising Sharpe of vol-scaled positions
(Lim–Zohren–Roberts 2019) overfit easily and must go through the same gates.

**C5. Comparing forecasts.** Diebold–Mariano with HAC variance; Giacomini–White conditional
ability; Model Confidence Set (Hansen–Lunde–Nason 2011). Model HPO trials are trials:
count them.

**C6. Calibration under drift.** Isotonic/Platt/temperature; conformalised quantile
regression (Romano et al. 2019); **adaptive conformal inference** (Gibbs & Candès 2021)
$\alpha_{t+1}=\alpha_t+\gamma(\alpha-\text{err}_t)$ tracks coverage under drift. It fits as an
online layer on `spread_90`, with `quality_monitor.rs` already tracking coverage.

**C7. Feature importance.** Avoid MDI (biased). Use MDA with purged CV and clustered
importance (substitution effects). SHAP on time series needs care with correlated
features.

**C8. Stacking.** Downstream models train on out-of-fold upstream predictions only.
Averaging quantiles across models (Vincentization) is valid; averaging σ is not the same
as a mixture's σ (relevant to the `Combine` node).

**C9. LLM and text features.** Constrained extraction (entity, event type, target,
novelty); cache results with document version, prompt version and model version;
contamination (05 §6); `LlmInference` backtests are not reproducible without the cache, so
treat them as forward-only evidence.

**C10. RL.** Non-stationarity, sample inefficiency and sim-to-real gaps make it a late,
narrow tool (execution, market making on a simulator), not a strategy-search engine.

---

## Part D — Strategy archetype library (seeds the agent's hypothesis generation)

Feasibility today: ✅ expressible now · 🟡 needs features (G-01/G-02) · 🔴 needs
architecture (sizing, exits, AI-in-backtest, multi-instrument, new lanes).

| Archetype | Rationale (who pays?) | Key inputs | Canonical null | Regime affinity | Main failure | Today |
|---|---|---|---|---|---|---|
| Trend: MA cross / breakout | Under-reaction, hedging flows | EMA, rolling max/min | `StationaryBootstrap`, GARCH zero-drift | $H>0.5$, high ADX | Chop whipsaw, costs | ✅ (EMA) / 🟡 (breakout) |
| Short-horizon mean reversion | Liquidity provision | RSI, z-score | `BlockPermutation` | $H<0.5$ | Trend crashes, costs at 1m | ✅ (RSI) / 🟡 |
| Volatility breakout | Vol clustering after compression | range, RV percentile | `RandomEntryMatched` | Low→high vol | False breakouts | 🟡 |
| Volatility-managed exposure | Weak vol–return link | σ forecast | Unmanaged at matched risk | Any | Estimator lag in spikes | 🔴 (sizing) |
| Large-move reversal / continuation | Overreaction / information | jump detector | Merton-jump null | High jump share | Regime-dependent sign | 🟡 |
| Calendar / session | Flows, liquidity cycles | hour/dow | within-day permutation, BH | Any | Data mining | 🟡 |
| Volume-shock | Attention, liquidity | rel. volume, Amihud | `RandomEntryMatched` | — | Venue artefacts | 🟡 |
| Regime-gated switching | Different dynamics by state | regime labels | `RegimeBlock` | By design | Label instability | 🟡 (FEAT-003 Ph3) |
| Meta-labelled rule | Model filters a rule's bad trades | primary signal + features | Primary alone | — | Overfit secondary | 🔴 (G-03) |
| Model-directional | Learned conditional drift | `Inference` | `SignalReturnDecouple` | — | A6: drift unidentifiable | 🔴 (G-03) |
| Carry / funding | Risk premium | funding rate | Zero-carry null | Calm | Crash risk | 🔴 (lane) |
| Pairs / stat-arb | Relative mispricing | spread, OU | Spread null | Stable cointegration | Breaks | 🔴 (single-instrument) |
| Prediction-market mispricing | Behavioural biases | Kalshi price, $P$-model | Market-probability null | — | Fees, thin books | 🔴 (lane) |
| News / social drift | Slow diffusion | PIT text features | Shuffled-timestamp null | — | Contamination | 🔴 (lane) |
| Order-flow imbalance | Inventory pressure | L2, signed trades | `BarPermutation` | Liquid | Latency | 🔴 (L2 history) |

**Today the agent's reachable space is essentially two archetypes on EMA/RSI.** That is
the strongest argument for fixing G-01 before adding agent sophistication.

---

## Part E — Diagnostic → move playbook

| Diagnostic pattern (from `get_diagnostics`) | Likely cause | Move to test |
|---|---|---|
| Losses concentrated in high-RV months | Vol-insensitive entries | `add_regime_gate` (vol) or vol-target sizing |
| Wins in trend regimes, losses in chop | Trend rule without context | Hurst/ADX gate; compare family per regime |
| Median hold < 3 bars and cost drag > 30% of gross | Overtrading | Hysteresis band, slower timeframe, event filter |
| High win rate, negative expectancy | Asymmetric exits (A4) | Remove target, widen stop, or reject the idea |
| MAE p50 ≈ stop distance | Stop inside noise | Stop in σ units (needs exits) |
| Top 5 trades > 100% of P&L | Tail-dependent | Longer history; `TradeMonteCarlo`; treat as fragile |
| Performance decays month over month | Alpha decay or regime shift | Walk-forward refit test; regime slices |
| Profitable only at one parameter spike | Overfit | Surface says fragile; drop or simplify |
| Gate 2 passes, Gate 3 fails | Too many trials for the effect size | Stop searching this family; record the finding |
| Signal correlation > 0.9 with momentum_20 | Disguised beta/momentum | De-dup; test incremental value over the benchmark |
