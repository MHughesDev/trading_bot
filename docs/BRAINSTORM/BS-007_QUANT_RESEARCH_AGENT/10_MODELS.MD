# BS-007 / 10 — Models: datasets, families, bring-your-own, honest use in strategies

**Spec status:** spec-ready draft · **Req prefix:** MD- · **Formal spec:** FEAT-006
(updates AI_MODELS_SUITE_CAPABILITY_SPEC) · **Plan set:** N · **Gaps fixed:** G-03, G-07,
G-10 · **Keeps:** the trainer/inference bundle contract, `scoring.py`, the quality monitor

---

## 1. Scope

Let the agent build models in a session, with granular inputs and honest evaluation, and
use them in strategies without look-ahead. Keep the strong parts:
- scoring (CRPS, PIT, Kupiec, Christoffersen);
- the bundle contract;
- the quality monitor.

Rebuild the interface around datasets as artifacts and bring-your-own-model training
jobs.

## 2. DatasetSpec (a first-class artifact)

```yaml
DatasetSpec:
  universe: [BTC-USD, ETH-USD]            # or universe(name, as_of) (06 §5)
  timeframe: 1h
  window: { start, end }                  # clipped by the research cutoff
  features: [ "ema(close,21)/close - 1", "yang_zhang(20)", "hurst(logret(close),256)", … ]   # 07
  transforms: { winsorize: 0.01, scaler: per_fold_standard, lags: [1,2,4] }   # declared ex ante (11 §4 3c–3e)
  sampling: every_bar | cusum(h=2σ) | strategy_events(strategy_ref)
  target:   forward_return(h=24, log) | realized_vol(h=24) | triple_barrier(pt=2σ, sl=1σ, max=48, path=high_low)
            | meta_label(strategy_ref) | rank(h=24)
  weights:  uniqueness | time_decay(half_life=180d) | none
  cv:       walk_forward(train=365d, test=30d, refit=30d) | purged_kfold(k=5) | cpcv(n=6,k=2)
            # purge/embargo derived from the label horizon, never hand-set
```

- A `dataset_build` job (05) materialises content-hashed Parquet with label intervals,
  purge and embargo metadata, and a PIT self-check (the leakage harness). The agent loads
  the same bytes in its sandbox as the trainer does.
- Adding features, interactions or transforms increments the Experiment's
  **specification count** (11 §3).
- Triple-barrier labels use intrabar high and low, not the close path only (fixes the
  current labeler).

## 3. Model families

| Family | Examples | Role and rules |
|---|---|---|
| Baselines | Historical mean, EWMA, ridge/elastic-net, **HAR / Log-HAR / HARQ** | Always run. A vol model must beat **HARQ** on QLIKE (11 T7) |
| GBMs (exist) | LightGBM/XGBoost point and quantile, LambdaRank | The workhorse; GBM on lagged features is the hard baseline for returns |
| Volatility | GARCH/GJR/EGARCH (with **conditional serving fixed**), realised GARCH | Sizing and risk inputs |
| Regime | HMM, statistical jump models (JM, continuous JM, sparse JM) | **Filtered** probabilities only (no smoothing) |
| Sequence | Real-window LSTM/GRU (fix seq-len-1), TCN, DLinear, N-HiTS, PatchTST, TFT, TSMixer (e.g. via Apache-2.0 `neuralforecast`) | Only where they beat GBM on lagged features; windows come from the DatasetSpec |
| Probabilistic | NGBoost, quantile regression forests, conformal wrappers | Calibrated uncertainty |
| Zero-shot foundation | Chronos / TimesFM / TTM class | **Baselines only**, carrying a contamination label; TTM + Log-HAR equal-weight combination as a vol baseline (research/R2 §4) |
| Combination | Equal-weight ensembles by default | Estimated weights must beat EW out of sample (11 T6) |
| **Bring your own** | Any agent-written class meeting §4 | The coding agent's main leverage |

## 4. Bring-your-own-model contract and training jobs

```python
class Model(tbot.models.ModelInterface):
    output = "quantiles"                       # typed → InferenceOutput fields
    def fit(self, ds: Dataset, fold: Fold) -> None: ...
    def predict(self, window: FeatureWindow) -> InferenceOutput: ...
    def save(self, path) -> None: ...  ;  @classmethod def load(cls, path): ...
```

- `tbot models train models/my_tcn.py --dataset art_… [--hpo n]` submits a `train` job
  (05). It:
  - runs in the trainer worker (GPU if available) with the code snapshot hashed;
  - produces one model per fold under the DatasetSpec's CV;
  - scores against baselines (§6);
  - registers the result through the bundle contract (feature order from the engine, 07).
- **HPO** trials count on a **model trial counter** in the model's Experiment. The
  optimiser picks hyperparameters (P1).
- **Solution-tree search** (research/R1 §2.1) is how the agent iterates on model code.
  Each node is a code-plus-DatasetSpec training job scored on walk-forward CRPS/QLIKE vs
  baselines. The tree lives in the job service.

## 5. Walk-forward prediction series (the honest path to model-driven strategies)

A model trained once on 2024–2026 can't be used to backtest 2025. The canonical path:
1. A `train` job with `cv: walk_forward(refit=…)` produces one model per fold.
2. A `predict_series` job concatenates out-of-fold predictions into a **prediction
   series** artifact (ClickHouse `model_predictions`). Every prediction was made by a
   model trained only on data available before it.
3. Backtests (09 §4) and Layer 1 strategies read the series as a feature stream
   (`prediction(series_ref).field`). Gate 0 rejects training-window overlap.
4. Live and paper use the latest refit through the inference gateway, so the live model
   is the last step of the same process (once G-11 is wired).

This is also the implementation of **meta-labeling**: primary strategy events → a
triple-barrier-with-side dataset → classifier → prediction series → SLv2 sizing or
filter.

## 6. Scoring, baselines and calibration

Added to `scoring.py`, alongside CRPS, pinball, PIT/KS, coverage, Kupiec and
Christoffersen (all exist):

| Addition | Purpose |
|---|---|
| **Campbell–Thompson OOS R²** vs historical mean; **Clark–West** (nested); **Diebold–Mariano** | Is the predictability real out of sample? (11 T4) |
| **QLIKE** | Volatility forecast loss |
| **Mincer–Zarnowitz recalibration** | Strip scaling "gains" (research/R2 §4) |
| **Acerbi–Szekely ES backtests**; joint VaR–ES (Fissler–Ziegel) loss | Required before a strategy may size on `var_95`/`es_95` (11 T20) |
| Utility gain of a mean-variance investor | Economic value of the forecast |
| **Purged MDA / clustered feature importance** | Not MDI |
| Vol-timed momentum baseline | Complex models must beat it (11 T5) |

**Calibration:** `tbot models calibrate --method aci|aci_cusum` returns a wrapper model
version with coverage-through-time. Sizing consumes the calibrated `spread_90`. CUSUM
change-point resets hook into `quality_monitor.rs`.

**Hard rules:**
- No forecast feature enters a strategy unless its walk-forward OOS R² > 0 with CW
  significance (returns), or it beats HARQ on QLIKE (volatility).
- Every model scorecard shows its baselines.

## 7. Serving and promotion

- GARCH serving: fix the fallback to unconditional σ. Serve the conditional forecast
  from live bars.
- **Alias promotion** (`@production`) affects live strategies, so it is an approvals-inbox
  item. The agent can only propose it (D-12).
- The quality monitor (PIT-KS drift, PSI, staleness) continues and feeds retrain
  proposals.

## 8. Requirements

| ID | Requirement | Pri |
|---|---|---|
| MD-01 | DatasetSpec MUST support §2 fields and materialise as a content-hashed artifact with label intervals, purge/embargo metadata and a PIT self-check | MUST |
| MD-02 | Purge and embargo MUST be derived from the label horizon | MUST |
| MD-03 | Triple-barrier labels MUST support intrabar high/low paths | MUST |
| MD-04 | The bring-your-own-model contract MUST run as a `train` job in the trainer worker, with a code hash, per-fold models and bundle registration | MUST |
| MD-05 | HPO trials MUST count on a model trial counter | MUST |
| MD-06 | The §3 families MUST be available: baselines, GBM, vol, regime, probabilistic and combination MUST; sequence SHOULD; foundation MAY | MUST/SHOULD/MAY |
| MD-07 | Sequence models MUST consume real windows (fix seq-len 1) | MUST |
| MD-08 | `predict_series` MUST produce walk-forward out-of-fold prediction series consumable by backtests and Layer 1 | MUST |
| MD-09 | §6 scoring additions MUST exist, and every scorecard MUST show baselines | MUST |
| MD-10 | ES backtests MUST pass before any strategy sizes on `var_95`/`es_95` | MUST |
| MD-11 | ACI calibration SHOULD be available as a wrapper model version | SHOULD |
| MD-12 | GARCH conditional serving MUST be fixed | MUST |
| MD-13 | Alias promotion MUST be human-approved | MUST |

## 9. Acceptance criteria

1. The agent builds a DatasetSpec, trains an agent-written TCN with walk-forward CV, and
   gets a scorecard vs HAR/GBM baselines, all through jobs.
2. A meta-labelled strategy completes the funnel end to end using a prediction series.
3. Substituting a full-period model for the prediction series fails Gate 0.
4. An LSTM trained on 64-bar windows shows sequence length 64 in its bundle metadata.
5. A vol model that loses to HARQ on QLIKE is flagged in its scorecard and blocked from
   strategy sizing by the rule in §6.

## 10. Open questions

1. GPU availability (00 cross-cutting question 2) decides whether the sequence family is
   MUST.
2. Retention for per-fold models vs the final refit only.
