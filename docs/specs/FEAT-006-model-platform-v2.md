# FEAT-006: Model Platform v2 — Datasets, Families, Bring-Your-Own Models, Prediction Series

**Status:** Proposed (Phase 0 contract; not implemented)
**Version:** 0.1
**ADR(s):** ADR-0015/0016 (model format and distributional contract, kept and extended),
ADR-0017 (walk-forward CV and leakage), ADR-0018 (ensembles and conformal), ADR-0029,
ADR-0030
**Derived from:** BS-007 [10_MODELS](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/10_MODELS.MD)
**Updates:** [AI_MODELS_SUITE_CAPABILITY_SPEC](AI_MODELS_SUITE_CAPABILITY_SPEC.md) (the
handoff contract stands)
**Plan set:** N (Models)
**Crates and apps:**
- `crates/domain::model_def` (v1.2 additions);
- `crates/model-registry`;
- `apps/model-trainer` (`worker.py` routing, `engine.py`, `labelers.py`, `scoring.py`,
  new `families/`, BYO runner);
- `apps/model-inference`;
- `crates/jobs` kinds `dataset_build`, `train`, `hpo`, `predict_series`;
- `migrations/0044_model_platform_v2.sql`;
- `clickhouse/08_prediction_series.sql`.

**Fixes:** G-03 (honest model use in strategies, with FEAT-005), G-07 (training in
session), G-10 (real sequence windows)

---

## 1. Purpose

Let the agent build models in a session with granular, declared inputs, score them
honestly against strong baselines, and use them in strategies without look-ahead.

**Keep:**
- the bundle contract (feature order + scaler + objective);
- `scoring.py`'s proper scores and calibration tests;
- the quality monitor;
- the model-definition format.

## 2. Current state (verified 2026-09-10)

- **Kinds:** `forecaster | signal_ranker | trade_decision | risk_sizing | embedding |
  external_llm_adapter`. **Frameworks:** `xgboost | lightgbm | sklearn | torch |
  external_api`, plus `garch` routed in `worker.py::_route`.
- **Torch LSTM:** `LSTMNet.forward` does `x.unsqueeze(1)` on 2-D rows, so the sequence
  length is 1. There is no Transformer, TFT, TCN or N-BEATS.
- **Labels:** triple barrier on the close path only (`labelers.py`), quantile,
  devolatized.
- **Scoring:** pinball, CRPS, log score, PIT, coverage, reliability, Kupiec,
  Christoffersen, naive/seasonal/zero-shot baselines, per-fold and per-regime scores,
  deflated CRPS, Diebold–Mariano.
- **`model_predictions`** exists in ClickHouse (point direction, magnitude, confidence).
- **GARCH serving** falls back to unconditional σ without live bars.

## 3. DatasetSpec (model definition v1.2 addition; stored as a `dataset` artifact)

```yaml
DatasetSpec:                             # canonical YAML/JSON; dataset_hash = sha256(canonical ‖ engine_version ‖ data_snapshot)
  universe: [BTC-USD, ETH-USD] | {universe: crypto_top20, as_of_membership: true}
  timeframe: 1h
  window: {start: 2021-01-01, end: 2026-03-31}          # server clips to the project cutoff
  features: ["ema(close,21)/close - 1", "yang_zhang(20)", "hurst(logret(close),256)", "prediction(art_…).sigma"]
  transforms: {winsorize: 0.01, scaler: per_fold_standard | per_fold_robust | none, lags: [1,2,4]}
  sampling: every_bar | cusum(h: 2sigma) | strategy_events(strategy_version)
  target:
    kind: forward_return | realized_vol | triple_barrier | meta_label | rank | direction
    horizon: 24                                          # bars
    params: {log: true} | {pt: 2sigma, sl: 1sigma, max_bars: 48, path: high_low} | {strategy_version: …}
  weights: uniqueness | time_decay(half_life: 180d) | none
  sequence: {window: 64, stride: 1} | null               # for sequence families; rows become windows
  cv: walk_forward(train: 365d, test: 30d, refit: 30d, mode: expanding|rolling) | purged_kfold(k: 5) | cpcv(n: 6, k: 2)
      # purge = label horizon; embargo = max(label horizon, 1% of sample) — derived, never hand-set
```

- **The `dataset_build` job** (COMP-005):
  - computes features via DATA-006 batch mode;
  - builds labels (intrabar high/low paths for triple barrier);
  - writes Parquet (features, label, label_start, label_end, weight, fold ids, sequence
    index);
  - runs the **leakage harness** (feature truncation self-test + label/fold overlap
    check);
  - records a manifest.
- **Specification counting:** every feature, transform or interaction added relative to
  the parent dataset increments the owning Experiment's specification count
  (BACKTEST_SUITE v2 §3).

## 4. Families (`apps/model-trainer/app/families/`)

| Family | Implementations | Rules |
|---|---|---|
| `baseline` | historical mean, EWMA, ridge/elastic-net, HAR, Log-HAR, **HARQ** | Always trained alongside any candidate on the same folds |
| `gbm` (exists) | LightGBM/XGBoost point and quantile, LambdaRank | Hard baseline for return/direction targets |
| `vol` | GARCH/GJR/EGARCH (`arch`), realised GARCH | Serving uses the conditional forecast (§8) |
| `regime` | Gaussian HMM (filtered probabilities), statistical jump model (JM, continuous JM, sparse JM) | Outputs **filtered** labels only; artifact type `regime_labels` |
| `sequence` | LSTM/GRU (real windows), TCN, DLinear, N-HiTS, PatchTST, TFT, TSMixer (e.g. Apache-2.0 `neuralforecast`) | Requires `DatasetSpec.sequence`; must beat GBM on lagged features to be used downstream |
| `probabilistic` | NGBoost, quantile regression forest, conformal wrappers | Calibrated intervals |
| `foundation` | Chronos / TimesFM / TTM class (zero-shot) | **Baselines only**, carrying `contamination: {model_cutoff}`. TTM + Log-HAR equal-weight combination offered as a vol baseline |
| `ensemble` (exists per ADR-0018) | Equal-weight by default; stacking only on the calibration role | Estimated weights must beat EW out of sample |
| `byo` | Agent-written classes (§5) | — |

**Routing:** `worker.py::_route` is replaced by a family registry keyed by
`(family, variant)`. Each entry declares:
- supported targets and outputs (point, quantiles, distribution, class probabilities);
- whether it needs sequences;
- whether it uses a GPU.

## 5. Bring-your-own-model contract

```python
class Model(tbot.models.ModelInterface):
    output = "quantiles"                       # point | quantiles | distribution | class_probs → InferenceOutput fields
    quantile_levels = [0.05, 0.25, 0.5, 0.75, 0.95]
    def fit(self, ds: "Dataset", fold: "Fold") -> None: ...
    def predict(self, window: "FeatureWindow") -> "InferenceOutput": ...
    def save(self, path: str) -> None: ...
    @classmethod
    def load(cls, path: str) -> "Model": ...
```

- **Submission:** `tbot models train models/my_tcn.py --dataset art_… [--hpo 40]` sends
  a code snapshot artifact plus the dataset handle and creates a `train` job (or `hpo`
  with children).
- **Runs in** the trainer worker sandbox: no network, a read-only code snapshot, scratch
  space, and CPU or GPU per the project tier.
- **Output:** one fitted model per fold plus the final refit, bundled per the existing
  contract. `feature_order` holds DATA-006 `feature_hash` values and `engine_version`.
  The code hash is recorded.
- **HPO:** Optuna in-fold (as `hpo.py` does), objective CRPS or QLIKE. **Every trial
  counts** on the model Experiment's trial counter (COMP-005 §10). The optimiser
  chooses hyperparameters (P1).

## 6. Walk-forward prediction series

1. A `train` job with `cv: walk_forward(refit …)` produces per-fold models `m_1…m_F`
   with `(train_start, train_end)`.
2. **The `predict_series` job** applies each `m_f` to its test window only, and
   concatenates the results into a **prediction series** artifact. It is also written to
   ClickHouse `model_prediction_series`.
3. **Consumers:**
   - FEAT-005 model slots;
   - Layer 1 `data.prediction(handle)`;
   - DATA-006 `prediction(series_ref).field`.

   Each consumer verifies `train_end < t − embargo` at read time (the G0 check in
   FEAT-005 §4.3).
4. **Live:** the latest refit serves through the inference gateway. Its output fields
   are identical to the series fields.

`clickhouse/08_prediction_series.sql`:

```sql
CREATE TABLE model_prediction_series (
  series_handle String, model_id String, model_version UInt32, fold UInt16,
  instrument_id String, event_time DateTime64(9,'UTC'), available_time DateTime64(9,'UTC'),
  train_start DateTime64(9,'UTC'), train_end DateTime64(9,'UTC'),
  median_return Nullable(Float64), sigma Nullable(Float64), direction Nullable(Int8), confidence Nullable(Float64),
  quantile_levels Array(Float64), quantiles_return Array(Float64),
  var_95 Nullable(Float64), es_95 Nullable(Float64), size_fraction Nullable(Float64), action_class Nullable(String)
) ENGINE = MergeTree ORDER BY (series_handle, instrument_id, event_time);
```

**Meta-labeling:** `tbot models meta-labeler <strategy_version> --experiment <id>`:
1. builds a `DatasetSpec` with `sampling: strategy_events(…)` and
   `target: meta_label` (triple barrier with the primary side);
2. trains a classifier through walk-forward;
3. produces a prediction series;
4. proposes an SLv2 move (`add_filter` or `change_sizing` using the series).

## 7. Scoring additions (`scoring.py`)

| Addition | Use |
|---|---|
| Campbell–Thompson OOS R² vs the historical mean; **Clark–West** (nested); DM (exists) | Return predictability (BACKTEST_SUITE v2 T4) |
| **QLIKE** and MSE on variance | Vol forecasts |
| **Mincer–Zarnowitz** regression and recalibration | Strip scaling artefacts |
| **Acerbi–Szekely** ES backtests (Z1/Z2); **Fissler–Ziegel** joint VaR–ES loss | Required before sizing on `var_95`/`es_95` (T20) |
| Mean-variance utility gain | Economic value |
| **Purged MDA**, clustered feature importance | Replaces MDI in reports |
| Vol-timed momentum baseline | Complex return models must beat it (T5) |

**Scorecards always include the baselines trained on the same folds.** Deflated scores
use the model trial counter.

**Hard usage rules** (enforced by the validator when a model or prediction series is
bound in a strategy that enters G3):
- return or direction forecasts need walk-forward OOS R² > 0 with Clark–West p < 0.05;
- vol forecasts must beat HARQ on QLIKE;
- `var_95`/`es_95` consumers need a passed ES backtest.

## 8. Calibration and serving

- `tbot models calibrate --method aci|aci_cusum` creates a wrapper model version (ADR-0018
  conformal, extended): adaptive conformal inference, with CUSUM change-point resets hooked
  into `quality_monitor.rs`. It reports coverage through time. Sizing consumes the
  calibrated `spread_90`.
- **GARCH serving:** the inference sidecar loads the recent bars window from ClickHouse
  (or the stream) and returns the conditional σ. The unconditional fallback is removed;
  missing data is an error with an abstain.
- **Alias promotion** (`@production`) is an approvals-inbox action. Agents can only
  propose it (AGENT-001 §16).

## 9. Storage (`migrations/0044_model_platform_v2.sql`)

```sql
ALTER TABLE dataset_versions ADD COLUMN IF NOT EXISTS dataset_spec JSONB;     -- 0019 datasets/dataset_versions
ALTER TABLE dataset_versions ADD COLUMN IF NOT EXISTS dataset_handle TEXT;    -- art_…
ALTER TABLE model_versions ADD COLUMN IF NOT EXISTS family TEXT;
ALTER TABLE model_versions ADD COLUMN IF NOT EXISTS code_handle TEXT;          -- BYO snapshot
ALTER TABLE model_versions ADD COLUMN IF NOT EXISTS engine_version TEXT;
ALTER TABLE model_versions ADD COLUMN IF NOT EXISTS requires_retrain BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE model_versions ADD COLUMN IF NOT EXISTS contamination JSONB;       -- foundation models
CREATE TABLE model_experiments (model_experiment_id TEXT PRIMARY KEY, project_id UUID, model_id UUID,
  trial_count BIGINT NOT NULL DEFAULT 0 CHECK (trial_count >= 0), created_at TIMESTAMPTZ DEFAULT now());
CREATE TABLE prediction_series (series_handle TEXT PRIMARY KEY, model_id UUID, model_version INT,
  dataset_handle TEXT, cv JSONB, folds JSONB NOT NULL, created_at TIMESTAMPTZ DEFAULT now());
CREATE TABLE model_series_links (series_handle TEXT, experiment_id TEXT, PRIMARY KEY (series_handle, experiment_id));
```

- `model_experiments.trial_count` is monotonic. It is updated only by the job-service
  counting hook, in the same transaction as the job insert.

## 10. Test plan and acceptance

| # | Test | BS-007 IDs |
|---|---|---|
| M1 | A DatasetSpec builds; the leakage harness passes; purge and embargo are derived; the triple barrier uses high/low | MD-01…MD-03 |
| M2 | The agent trains a BYO TCN with walk-forward CV via jobs; the scorecard shows HAR, HARQ and GBM baselines | MD-04, MD-06, MD-09 |
| M3 | 40 HPO trials → model trial counter +40 | MD-05 |
| M4 | An LSTM on 64-bar windows records `sequence.window=64` in its bundle; inputs are 3-D | MD-07 |
| M5 | `predict_series` output: every row's `train_end < event_time − embargo`; a meta-labelled strategy completes the funnel | MD-08 |
| M6 | A vol model losing to HARQ on QLIKE is blocked from sizing at G3; a model without an ES backtest is blocked from ES sizing | MD-09, MD-10 |
| M7 | The ACI wrapper improves coverage stability on a synthetic regime-shift series | MD-11 |
| M8 | GARCH serving returns conditional σ; no silent unconditional fallback | MD-12 |
| M9 | An agent-proposed alias promotion waits for human approval | MD-13 |

## 11. Open questions

1. GPU availability (decides whether the `sequence` family is MUST in Set N).
2. Retention: keep all per-fold models, or only the fold metadata plus the final refit?
3. Default embargo floor for high-frequency (≤ 5m) datasets.
