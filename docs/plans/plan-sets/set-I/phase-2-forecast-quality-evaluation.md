# Phase 2 — Forecast-quality evaluation suite

**Completion: 0% (0 / 12 tasks)**

**Goal:** Build the **measuring instrument** — the proper-scoring, calibration, and
significance machinery that makes a distributional forecast trustworthy. CRPS,
pinball, and log-score; PIT, coverage, and reliability; VaR backtests (Kupiec,
Christoffersen); naive/seasonal/GARCH/zero-shot baselines; per-fold and per-regime
breakdowns; overfitting diagnostics (trial count, deflated metrics, single-use final
holdout); Diebold–Mariano significance; a model/ensemble **leaderboard**; and
shareable reports. Close the Set H `metrics.rs` stub by joining forecasts to
**realized outcomes**. This is forecast quality, **never strategy P&L** (D-7).

**Depends on:** Phase 0 (test windows, leakage harness), Phase 1 (distributions,
HPO trial counts).
**Blocks:** Phase 4 (ensembles use the *same* suite), Phase 5 (monitoring reuses
these scores over time), Phase 6 (reports/leaderboard UI).

---

## Design notes

**Parity-preserving eval loop (D-9).** Evaluation must score the *published* path,
not a re-implementation. Rust orchestrates each eval run:

1. Materialize the **test** windows (Phase 0, PIT, `as_of`-bounded).
2. Produce predicted distributions through the **inference path** (the same bundle
   serve uses) — not a separate scoring-time model.
3. Join **realized outcomes** PIT-correctly: outcome for a forecast made at `t` over
   horizon `H` is only joined once `available_time ≥ t+H` (closes the `metrics.rs`
   `actual = null` gap).
4. Dispatch `(quantiles, levels, realized)` to the sidecar scoring module for the
   metrics below.
5. Persist a scorecard + a leaderboard row + a shareable report.

**Metrics (all in the Python sidecar, D-3):**

| Metric | What it answers | Notes |
|--------|-----------------|-------|
| **CRPS** | Is the *whole distribution* sharp & calibrated? | the headline score; never RMSE for distributions |
| **Pinball / quantile loss** | Per-level accuracy | averaged ≈ CRPS; per-level for asymmetry |
| **Log-score** | Density quality | needs a density/interp from quantiles |
| **PIT** | Calibration | uniform ⇒ calibrated; KS/χ² on the PIT histogram |
| **Interval coverage** | Do 90% intervals cover ~90%? | per nominal level |
| **Reliability diagram** | Calibration curve | data for the plot (predicted vs empirical) |
| **Kupiec POF** | VaR unconditional coverage | likelihood-ratio test on exceptions |
| **Christoffersen** | VaR exception independence | conditional-coverage LR test |
| **Diebold–Mariano** | Is model A's score sig. better than B's? | loss-differential test |
| **Deflated metric** | Is the score real or HPO-mined? | adjust by trial count from Phase 1 |

**Baselines** are first-class comparators, scored by the identical loop: naive
(last value / random-walk), seasonal-naive (same bar-of-session/day), GARCH-only
(Phase 1 GARCH-t with no features), and **foundation zero-shot** (a pretrained
time-series model via an adapter, scored, never trained here).

---

## Tasks

### ☐ I-2.1 Eval orchestration + scoring module scaffold — M
Add Rust `EvalManager` (or extend `ModelManager::drive_eval`) implementing the
parity loop above, and a sidecar `scoring` module with an `/evaluate` (or `/score`)
endpoint taking `(quantile_levels, predicted_quantiles[], realized[])`. Decide
trainer-sidecar-extension vs a dedicated `apps/model-eval` (recommend extending the
trainer sidecar first; split later if monitoring load demands). New deps
(`scipy.stats`) behind the sidecar extra.
**Acceptance:** an eval run drives predict→join→score→persist end to end on a fixture; the scoring endpoint returns a metrics dict; no Rust ML crate added.

### ☐ I-2.2 Realized-outcome join (close the `metrics.rs` stub) — M
Replace the `metrics.rs` null proxies and `build_forecast_vs_actual_series`
(`actual = null`) with a PIT-correct join: for each prediction at `t/H`, fetch the
realized return once `available_time ≥ t+H`. Persist the aligned
`(prediction, realized)` series for scoring and for the UI.
**Acceptance:** the forecast-vs-actual series has non-null actuals; a unit test asserts an outcome at `t+H` is never joined before its `available_time`.

### ☐ I-2.3 Proper scoring: CRPS, pinball, log-score — M
Implement CRPS (from the quantile representation), per-level + mean pinball loss,
and log-score (density from monotone quantiles). Return per-fold and aggregate.
**Acceptance:** on a fixture where the predictive distribution equals the data-generating distribution, CRPS/pinball are near their theoretical minima and beat a flat baseline; unit tests pin known values.

### ☐ I-2.4 Calibration: PIT, coverage, reliability — M
Compute the PIT series + histogram (with a KS/χ² uniformity stat), interval coverage
per nominal level, and reliability-diagram data (predicted vs empirical frequency).
**Acceptance:** a well-calibrated synthetic forecaster yields ~uniform PIT and coverage within tolerance of nominal; a deliberately overconfident one is flagged (coverage ≪ nominal).

### ☐ I-2.5 VaR backtests: Kupiec + Christoffersen — M
From the distribution's lower tail, compute VaR exceptions and run Kupiec POF
(unconditional coverage) and Christoffersen (independence/conditional coverage)
likelihood-ratio tests at configured levels.
**Acceptance:** a correctly-calibrated tail passes both at α=0.05; a tail with clustered/too-many exceptions is rejected; test statistics match a reference implementation on a fixture.

### ☐ I-2.6 Baselines: naive, seasonal-naive, GARCH-only, zero-shot — M
Add the four baselines as scored comparators run through the same loop; persist their
scorecards alongside the model's so every eval includes "beats baseline?" verdicts.
The foundation zero-shot baseline is an inference-only adapter (no training).
**Acceptance:** an eval report lists the model and all four baselines with CRPS/coverage; the "beats naive" / "beats GARCH" verdicts are computed.

### ☐ I-2.7 Per-fold / per-regime / per-segment breakdowns — M
Break every metric down by fold and by regime/segment (e.g. high- vs low-vol buckets,
session, trend vs chop). Persist the breakdown for the report and leaderboard filters.
**Acceptance:** a report shows CRPS per fold and per vol-regime; a model that is good only in one regime is visibly so.

### ☐ I-2.8 Overfitting diagnostics — M
Surface the HPO trial count (Phase 1), compute **deflated** scores (adjust for trials),
and enforce a **single-use final holdout**: a held-out tail that may be scored exactly
once per version and is recorded as used (refuse a second scoring).
**Acceptance:** deflated CRPS is reported next to raw; a second attempt to score the final holdout for the same version is refused with a clear error.

### ☐ I-2.9 Diebold–Mariano significance test — S
Implement the DM test on the loss differential between two forecasters (model vs
baseline, or model A vs model B), returning the statistic + p-value.
**Acceptance:** DM on identical forecasts gives ~0 (no difference); on a clearly-better forecaster it reports significance at p<0.05; matches a reference on a fixture.

### ☐ I-2.10 Scorecard upgrade (Quality from proper scores) — S
Make the Set H `scorecard.rs` Quality sub-score derive from CRPS + calibration (not
the `val_auc` proxy); persist the full metric set on the evaluation run. Leave
Speed/Cost/Safety/Reliability as the existing operational signals (out of scope to
rework here).
**Acceptance:** Quality reflects CRPS/coverage; a better-calibrated model scores higher Quality; the eval run stores CRPS/pinball/log-score/PIT/coverage/VaR/DM.

### ☐ I-2.11 Model/ensemble leaderboard — M
Add a leaderboard over evaluation runs: rank models *and* ensembles (Phase 4) by a
chosen metric, filterable by asset/timeframe/regime, with DM significance vs the
current leader. Add `GET /api/models/leaderboard`.
**Acceptance:** the endpoint returns ranked entries with CRPS/coverage + DM-vs-leader; two versions of one model and a baseline appear side by side.

### ☐ I-2.12 Shareable evaluation reports — M
Persist a complete, immutable evaluation report (scores, breakdowns, PIT/reliability
data, baseline comparisons, DM, deflated metrics) keyed to version + dataset hash;
expose `GET /api/models/{id}/versions/{v}/report` and a JSON/HTML export. Stream eval
progress on the `models.jobs` WS lane.
**Acceptance:** a report renders all sections from persisted data, exports to a shareable file, and is reproducible from the version + dataset hash.

---

## Phase 2 exit criteria

- Forecasts are scored against **real realized outcomes** (no `null` actuals); the
  `metrics.rs` stub is gone.
- CRPS, pinball, log-score, PIT, coverage, reliability, Kupiec, Christoffersen, and
  Diebold–Mariano are all computed and persisted.
- Every eval includes naive/seasonal/GARCH/zero-shot baselines and per-fold +
  per-regime breakdowns.
- Overfitting diagnostics (trial count, deflated metrics, single-use final holdout)
  are enforced.
- A leaderboard ranks models/ensembles with significance; reports are shareable and
  reproducible.
- The leakage harness (Phase 0) flags the planted-leak model via impossibly-good
  scores. `cargo test -p model-registry` + sidecar pytest green; `just lint`,
  `just check-money` green.
