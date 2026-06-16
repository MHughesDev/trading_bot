# "AI Models" — Suite Capability Spec

> **Provenance.** Human-provided capability spec for the **AI Models** suite,
> received 2026-06-16. This is the source requirement that the **Set I** plan set
> (`docs/plans/plan-sets/set-I/`) is derived from. It is recorded here verbatim so
> every Set I task can trace to a numbered capability area (§1–§11) or the handoff
> contract. Do not edit the intent; if the requirement changes, supersede this file
> and note it in the Set I `MASTER.md` Progress Log.

The AI Models surface is the model workbench of the trading platform. Users come here to
build, train, test, evaluate, ensemble, and pipeline predictive models — and then publish
them so other surfaces (Strategies, Backtesting, Live Trading, Risk) can consume them.

**Scope boundary (read this first):** AI Models owns the model lifecycle, not the trading
lifecycle. It produces forecasters. It does not place orders, size positions, manage capital,
or compute P&L. The line is: this suite ends when a trained, evaluated, versioned forecaster is
published behind a stable contract. Everything past that line lives on other surfaces.

## The objects users work with

| Object | What it is | Primary verbs |
|--------|-----------|---------------|
| Model | one estimator: a family + config + trained artifact (e.g. LightGBM-Q, GARCH-t) | create, train, test, eval, version, publish |
| Ensemble | a combination of models (roster + combiner + calibration) | compose, train-weights, eval, version, publish |
| Pipeline | a DAG that produces or serves models (data → features → train → eval → register) | build, parameterize, run, schedule |
| Spec | the config that fully describes a Model/Ensemble/Pipeline (the unit of work) | author, validate, clone, hash |
| Run / Experiment | one execution of a spec, with params, metrics, artifacts | track, compare, reproduce |
| Evaluation report | the scored result of a test/eval | generate, view, share |
| Registry entry | a versioned, published artifact other surfaces consume | promote, roll back, publish |

The suite's top-level navigation maps to the first three: **Models · Ensembles · Pipelines**,
each supporting train / test / eval.

## 1. Data access & windowing (read-only into platform data)

- Browse and select asset + timeframe data already in the platform
- Point-in-time, forming-bar-safe resampling to higher timeframes from base bars
- Define walk-forward windows (train / calibration / test; expanding or rolling; purge + embargo)
- Data-quality preview (gaps, dupes, outliers) before training
- Enforced leakage guard on every data view (no future bars accessible)
- **Out of scope:** ingesting/owning raw market data — the suite reads the platform's data layer

## 2. Feature engineering (in-suite)

- A feature library: named, versioned, reusable transforms users can add to a spec
- Multi-resolution feature assembly from base bars
- Built-in families: returns/lags, range-based volatility, momentum, mean-reversion, volume, calendar/session, cross-asset context
- Devolatization (σ-standardization) and fit-on-train scaling
- Feature preview, importance, and correlation inspection (fight collinearity)
- Custom/user-defined feature support (pluggable)

## 3. Target & labeling config

- Choose target type: return, direction, move-size, triple-barrier, volatility, quantile
- Configure horizon H per timeframe
- Devolatized target construction
- Label-overlap metadata feeding the CV engine

## 4. Train a single model

- Pick a model family and configure hyperparameters
- Train with the walk-forward CV engine (the trust foundation, on by default)
- Live training progress / logs / run status
- Overfitting-aware HPO that runs inside folds and counts its trials
- Standardized distributional output contract (sorted quantiles, σ-units) enforced for every model
- Save as a versioned Model artifact
- Preserve the simple "train one model in isolation" path as a first-class flow

## 5. Test & evaluate (forecast-quality, not strategy P&L)

- Proper scoring: CRPS, pinball, log-score (never RMSE for distributions)
- Calibration: interval coverage, PIT, reliability diagrams
- VaR backtests of the model's own risk output (Kupiec, Christoffersen)
- Baseline comparisons: naive / seasonal-naive, GARCH-only, foundation zero-shot
- Per-fold, per-regime / per-segment breakdowns
- Overfitting diagnostics: trial count, deflated metrics, single-use final holdout
- Forecast-comparison significance tests (e.g. Diebold–Mariano)
- A model/ensemble leaderboard for side-by-side comparison
- Shareable evaluation reports
- **Out of scope:** strategy backtesting with execution, costs, sizing, and P&L — that's a
  downstream surface that consumes a published model

## 6. Ensemble

- Compose an ensemble by selecting a roster of models
- Choose the combiner: linear opinion pool, CRPS-weighted (adaptive) weighting, or stacking
- Configurable weight floors / temperature
- Spine-as-coordinate-setter handling (combine standardized shapes, rescale by σ)
- Conformal calibration wrapper (adaptive) on the ensemble output
- Quantile-crossing repair
- Evaluate an ensemble with the exact same test/eval suite as a model
- Save as a first-class versioned Ensemble artifact

## 7. Pipelines

- A pipeline builder (visual or declarative) for the DAG: data → features → target → train → calibrate → evaluate → register
- Templated, spec-driven pipelines reusable across (asset, timeframe, window) — the factory
- Fan-out a single pipeline across many assets / timeframes / windows in one run
- Fast/slow window instances of the same architecture
- Schedule pipeline runs and retrains on a bar-based cadence
- Run history, caching, incremental / partial re-runs, retries
- Two pipeline kinds: training pipelines (produce bundles) and inference pipelines
  (assemble bundles → calibrated forecast) — the latter is what gets published
- Trigger retrain pipelines from monitoring/drift signals

## 8. Model management & registry

- Versioned models, ensembles, and pipelines keyed by spec
- Rich metadata: config, data snapshot, metrics, lineage, training timestamp
- Champion / challenger promotion & staging; rollback
- Tag, search, organize, and annotate artifacts
- Publish an artifact behind a stable contract for downstream surfaces (the handoff)

## 9. Experiment tracking & reproducibility

- Track every run's params, metrics, artifacts, and pinned data snapshot
- Compare runs / experiments side by side
- Reproduce any run from spec + snapshot + seed
- Spec → deterministic hash for caching and reproducibility

## 10. Model-quality monitoring (suite-scoped)

- Rolling forecast-quality drift (CRPS, coverage over time) for published models
- Calibration drift and data/feature drift alerts
- Staleness alerts → trigger a retrain pipeline
- **Scope note:** this monitors model quality, not trade performance — P&L monitoring is elsewhere

## 11. Collaboration & workbench UX

- Save / clone / share models, ensembles, pipelines, and reports
- Templates & presets (a starter spec users can fork)
- Run queue and compute/resource management
- Permissions and provenance (if multi-user)

## The handoff contract (the one seam that matters)

Because the suite publishes forecasters that other surfaces call, the contract is in scope to
define even though trading isn't. A published artifact exposes:

- `predict(asset, timeframe, timestamp) → calibrated distribution` (sorted quantiles + median,
  in return units after σ-rescale), point-in-time correct
- Derived risk read-outs from the distribution (VaR, ES, skew, spread) for consumers that want them
- Versioned, immutable behavior, so a Strategy that pinned version N keeps getting version N
- Train/serve parity guarantee: the published inference path is the same code the
  evaluation used — no second implementation that can drift

Defining this seam cleanly is what lets the Strategies/Backtest/Live surfaces consume models
without ever reaching inside the suite.

## Explicitly OUT of scope (other surfaces)

- Strategy logic / signal-combination-into-trades
- Strategy backtesting with execution, costs, slippage, position sizing, P&L
- Order management, execution, exchange routing, fills/reconciliation
- Risk engine: exposure limits, drawdown circuit breakers, kill switches
- Capital allocation across strategies; portfolio construction
- Live trade P&L and performance monitoring

AI Models feeds all of these; it doesn't do any of them.

## Quality attributes this suite must hit

| Attribute | Stress scenario | What it forces |
|-----------|-----------------|----------------|
| Leakage-safety | "Prove no future info entered training or eval" | point-in-time data views, automated leakage tests in every pipeline |
| Reproducibility | "Re-run this experiment, get identical numbers" | data snapshots, spec hashing, seed + environment pinning |
| Modifiability | "Add a new model family or feature" | pluggable model/feature interfaces, no core rewrite |
| Scalability | "Fan a pipeline across 6 timeframes × N assets" | spec-driven parallel factory, distributed runs |
| Portability | "Same suite works on ETH or TSLA" | per-(asset, timeframe) bundles + adapter pattern |
| Contract stability | "Downstream pinned v3 — it must not change" | immutable published versions, train/serve parity |

## Suggested build order within the suite

1. Spec schema + single-model training (wrap the existing trainer) — modes-of-one first
2. Walk-forward CV + three-role split + leakage tests — trust before features
3. Test/eval suite (CRPS, coverage, VaR, baselines, leaderboard) — the measuring instrument
4. Registry + experiment tracking — versioned artifacts, reproducible runs
5. Ensemble + calibration — compose and score combinations
6. Pipeline builder + fan-out + scheduling — the factory at scale
7. Publish contract + model-quality monitoring — the clean handoff outward

Steps 1–3 are the **trust foundation**; nothing above them is trustworthy until they're solid.
