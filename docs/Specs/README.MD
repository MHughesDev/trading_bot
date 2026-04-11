# Codebase specs (derived)

This folder holds **specifications that describe the repository as implemented**. They are maintained to stay aligned with the code; when behavior changes, update the relevant file here (or ask an agent to).

**Not product marketing** — these are engineer-facing maps: modules, responsibilities, main entry points, and boundaries (e.g. Coinbase-only market data, Alpaca paper execution only).

## How this relates to other docs

| Location | Role |
|----------|------|
| **`docs/Specs/`** (this folder) | What the code **is** and **does** today |
| **`docs/Human Provided Specs/`** | What **you** want; agents compare to code + Specs and update backlog / implementation |
| **`docs/QUEUE.MD`** | Open queue, resolved fixes (`IL-*`), gates (`HG-*`), completed archive (`FB-*`) |
| **`docs/READY_TO_RUN.MD`** | Operator checklist (venv, `.env`, Docker, preflight, run UI) |
| **`docs/*.MD` reference docs** | Deep dives (risk order, backtesting semantics, QuestDB, etc.) |

## Index

| File | Scope |
|------|--------|
| [`SYSTEM_OVERVIEW.MD`](SYSTEM_OVERVIEW.MD) | Product boundaries, repository map, tech stack |
| [`APP_CONFIG_AND_CONTRACTS.MD`](APP_CONFIG_AND_CONTRACTS.MD) | `app/config/`, `NM_*`, `app/contracts/` |
| [`DATA_PLANE.MD`](DATA_PLANE.MD) | Ingest, bars, features, memory, storage |
| [`DECISION_PIPELINE.MD`](DECISION_PIPELINE.MD) | `decision_engine/` — shared tick path |
| [`RISK_ENGINE.MD`](RISK_ENGINE.MD) | `risk_engine/` — constraints, signing |
| [`EXECUTION_LAYER.MD`](EXECUTION_LAYER.MD) | `execution/` — router, adapters, CDP REST |
| [`BACKTESTING.MD`](BACKTESTING.MD) | `backtesting/` — replay parity with live |
| [`CONTROL_PLANE.MD`](CONTROL_PLANE.MD) | FastAPI + Streamlit |
| [`MODELS_AND_ORCHESTRATION.MD`](MODELS_AND_ORCHESTRATION.MD) | Regime, forecast, routing, retrain |
| [`OBSERVABILITY_AND_INFRA.MD`](OBSERVABILITY_AND_INFRA.MD) | Metrics, logging, `infra/docker-compose.yml` |
| [`TESTING_AND_CI.MD`](TESTING_AND_CI.MD) | `tests/`, `scripts/`, GitHub Actions |
| [`FORECASTER_AND_POLICY.MD`](FORECASTER_AND_POLICY.MD) | `forecaster_model/`, `policy_model/`, `ForecastPacket` |
| [`MASTER_SPEC_RISK_STATE_GAP.MD`](MASTER_SPEC_RISK_STATE_GAP.MD) | App vs policy `RiskState` vs master spec §12 (FB-AUDIT-12) |
| [`MULTI_ASSET_PORTFOLIO.MD`](MULTI_ASSET_PORTFOLIO.MD) | Independent ticks vs shared risk caps; no cross-symbol optimizer (FB-SPEC-07) |
| [`../MIGRATION_TO_SPEC_PIPELINE.MD`](../MIGRATION_TO_SPEC_PIPELINE.MD) | Historical note: legacy Ridge path removed; single master pipeline |
| [`../SYSTEM_WALKTHROUGH.MD`](../SYSTEM_WALKTHROUGH.MD) | Step-by-step: live, paper, live execution, backtest (master pipeline) |
| [`../Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD`](../Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD) | Authoritative end-to-end system spec |

## Naming convention

- Files use **`.MD`** (uppercase), consistent with `docs/` in this repo.
- Specs are **topic-based**, not one file per Python module (avoids churn).
