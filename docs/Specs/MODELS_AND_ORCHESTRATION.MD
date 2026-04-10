# Models and orchestration

## Models (`models/`)

- **`regime/hmm_regime.py`** — HMM regime; optional joblib load/save paths via settings
- **`forecast/tft_forecast.py`** — Forecast path (Ridge surrogate / optional PyTorch extras)
- **`routing/route_selector.py`** — Deterministic route selection from config thresholds
- **`registry/mlflow_registry.py`** — Stubs / helpers for MLflow; **no automatic model promotion in code** (CI guard)

## Orchestration (`orchestration/`)

- **`nightly_retrain.py`** — Train/eval/log pipeline; optional Prefect flow and MLflow logging when dependencies installed

## Optional extras

- **`[orchestration]`**, **`[models_torch]`**, **`[sentiment_nlp]`** in `pyproject.toml`
