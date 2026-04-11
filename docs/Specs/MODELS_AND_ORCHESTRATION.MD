# Models and orchestration

## Models (`models/`)

- **`registry/mlflow_registry.py`** — Stubs / helpers for MLflow; **no automatic model promotion in code** (CI guard)

Forecaster weights and training live under **`forecaster_model/`** and **`orchestration/`** (master pipeline — see [`Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD`](../Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD)).

## Orchestration (`orchestration/`)

- **`nightly_retrain.py`** — Train/eval/log pipeline; optional Prefect flow and MLflow logging when dependencies installed

## Optional extras

- **`[orchestration]`**, **`[models_torch]`**, **`[sentiment_nlp]`** in `pyproject.toml`
