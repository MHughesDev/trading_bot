# Models and orchestration

> **Architecture status:** This file describes the **repository as implemented today**. For **target** system architecture during the canonical APEX migration, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`../Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/). Where this document conflicts with canonical APEX specs on **future** design, treat this file as **as-built** only, not the roadmap.


## Models (`models/`)

- **`registry/mlflow_registry.py`** — Stubs / helpers for MLflow; **no automatic model promotion in code** (CI guard)

Forecaster weights and training live under **`forecaster_model/`** and **`orchestration/`** (master pipeline — see [`Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD`](../Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD)).

## Orchestration (`orchestration/`)

- **`nightly_retrain.py`** — Train/eval/log pipeline; optional Prefect flow and MLflow logging when dependencies installed

## Optional extras

- **`[orchestration]`**, **`[models_torch]`**, **`[sentiment_nlp]`** in `pyproject.toml`
