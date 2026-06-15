# Phase 3 — Evaluation & Promotion

**Completion: 100% (10 / 10 tasks)**

**Goal:** Decide whether a trained version is *good* and *better than what's
live*, then let it reach `Active` only through an explicit, auditable gate. Build
the evaluation-run engine, per-kind metrics, forecast-vs-actual + backtest P&L
impact, the normalized scorecard, regression detection against the production
baseline, the promotion gate, and deployments/aliases.

**Depends on:** Phases 0–2 (needs real artifacts + datasets). **Blocks:** Phase 4
(inference serves the `production` alias this phase sets).

---

## Design — the promotion gate

A version moves the `production` alias only if **all** gate checks pass (D-9).
Checks are computed from this phase's eval + the live baseline:

| Check | Source | Default rule |
|-------|--------|--------------|
| Passed eval suite | `evaluation_runs.status = succeeded` | required |
| No metric regression | regression report vs `production` baseline | no primary metric worse beyond tolerance |
| Within latency budget | inference traces (Phase 2) | p95 ≤ budget (per kind) |
| Artifact integrity | `model_artifacts.sha256` re-verified | hash matches stored |
| Rollback available | prior `production` version exists | required for live |

The gate is **advisory-strict**: checks render in the UI (Phase 5 promotion
modal); a failing check blocks the default action but an explicit override
(logged in `model_events` with a reason) is allowed for non-live environments.
Live promotion never bypasses the check set.

**Scorecard.** Each evaluated version gets normalized 0–100 sub-scores —
`Quality` (primary metric vs target band), `Speed` (latency vs budget), `Cost`
(inference cost vs budget), `Safety` (guardrail/anomaly checks), `Reliability`
(error rate, calibration) — plus a weighted `Overall`. Stored in
`model_versions.scorecard_json`; drives the card sparkline and the version
timeline badges.

---

## Tasks

### ☑ H-3.1 Evaluation-run engine in `model-registry` — L
`ModelManager::create(kind=Eval)` driver: resolve the eval dataset version, batch
the version through the inference sidecar, collect predictions, compute metrics,
write `evaluation_runs` (+ `scorecard_json`, `sample_outputs_json`). Same async
job machinery, progress, and WS lane as training.
**Acceptance:** `POST …/versions/{v}/evaluate` produces a completed
`evaluation_runs` row with metrics + ≥N sampled prediction/label pairs; pollable
and streamed like a training run.

### ☑ H-3.2 Per-kind metric sets — M
Metric calculators selected by `model_kind`:
- `forecaster`: directional accuracy, RMSE/MAE on magnitude, calibration (Brier),
  hit-rate by confidence bucket.
- `signal_ranker`: rank IC, NDCG@k, top-bucket forward return.
- `trade_decision`: accuracy/precision/recall, confusion matrix.
- `risk_sizing`: realized-vol vs target, drawdown adherence.
- `embedding`/`adapter`: served via traces (latency/cost) — no train metrics.

**Acceptance:** each kind yields its documented metric dict; a classifier eval
returns a confusion matrix the UI can render; metrics are deterministic for a
fixed dataset+artifact.

### ☑ H-3.3 Forecast-vs-actual series — M
For `forecaster`, align each prediction to its realized outcome at horizon
(available-time correct, ADR-0008) and emit a `{t, predicted, actual, error}`
series + confidence bands into `sample_outputs_json`. This is the Test
Lab/eval chart payload (Phase 5).
**Acceptance:** the series is leak-free (actuals strictly post-horizon); plots
sensibly for a known model; band coverage matches stated confidence.

### ☑ H-3.4 Backtest P&L impact (reuse the market simulator) — L
Bridge eval to the existing backtest engine (ADR-0014, `crates/backtest`):
run a reference strategy whose `model_forecast` resolves to **this version**, over
the eval window, and capture P&L/Sharpe/drawdown as the *economic* score — the
question that matters: "does this model make money in a strategy?"
**Acceptance:** an eval can attach a backtest result; switching the version under
the same strategy/window changes the P&L; the run reuses the backtest manager (no
duplicate simulator).

### ☑ H-3.5 Normalized scorecard — M
Compute the 0–100 sub-scores + weighted overall from metrics + traces + backtest;
persist to `scorecard_json`; document the normalization bands and weights (config,
not hardcoded magic).
**Acceptance:** scorecard is reproducible; weights live in config; an obviously
better model scores higher overall than a worse one on the same suite.

### ☑ H-3.6 Regression detection vs baseline — M
Compare a candidate version's metrics to the current `production` version on the
**same** eval dataset; produce `regression_report_json` = per-metric
`{baseline, candidate, delta, verdict}` with tolerances. Verdict roll-up:
`improved | neutral | regressed`.
**Acceptance:** identical artifacts → `neutral` everywhere; a deliberately worse
candidate → `regressed` on the primary metric with correct deltas.

### ☑ H-3.7 Promotion gate evaluator — M
`promote(model_id, v, env)` computes the gate-check table, blocks on any failure
(live) / allows logged override (non-live), then `set_alias('production', v)`,
sets version `Active`, demotes the prior `Active`→`Archived` (or keeps as
`fallback`), writes a `promoted` event with the full check snapshot.
**Acceptance:** promotion with a failing live check is rejected with the failing
checks listed; a passing promotion moves the alias, flips statuses, and records an
auditable event including the gate snapshot.

### ☑ H-3.8 Rollback + fallback wiring — S
`rollback` restores the previous `production` target and records `rolled_back`;
`fallback` alias optionally retains the prior version for the inference gateway
(Phase 4) to fail over to.
**Acceptance:** rollback is one call, fully audited, and immediately reflected in
alias resolution; fallback target is independently settable.

### ☑ H-3.9 Deployments & traffic split — M
`POST …/deployments` records a version serving in `paper`/`live` with
`traffic_pct` (A/B). The inference gateway (Phase 4) reads this. Enforce per-env
traffic sum ≤ 100.
**Acceptance:** a version can serve 100% paper while 0% live; a 70/30 A/B between
two versions is representable and validated; over-allocation rejected.

### ☑ H-3.10 Evaluation API surface completion — S
Finalize `GET …/evaluations[/{eval_id}]` payloads (scorecard, regression report,
forecast-vs-actual samples, optional backtest result) and the compare endpoint
`GET …/evaluations/compare?versions=a,b` powering the side-by-side arena (Phase 5).
**Acceptance:** the compare endpoint returns aligned metrics + a winner verdict
for two versions; payloads match the Phase-1 frozen contract shapes.

---

## Phase 3 exit criteria

- Any version can be evaluated into a scorecard + regression report + (optional)
  backtest P&L, all pollable/streamed.
- Promotion is gated, audited, reversible; aliases + deployments are the single
  source of truth for "what serves."
- The data needed for the evaluations arena and promotion modal (Phase 5) is fully
  available behind the frozen contract.
