# Codebase specs (derived)

This folder holds **specifications that describe the repository as implemented**. They are maintained to stay aligned with the code; when behavior changes, update the relevant file here (or ask an agent to).

**Not product marketing** — these are engineer-facing maps: modules, responsibilities, main entry points, and boundaries (e.g. Kraken-only market data, no Alpaca *data* imports outside the paper adapter, Alpaca paper / Coinbase live execution).

## How this relates to other docs

| Location | Role |
|----------|------|
| **`docs/Specs/`** (this folder) | What the code **is** and **does** today |
| **`docs/Human Provided Specs/`** | What **you** want; agents compare to code + Specs and update backlog / implementation |
| **[`docs/QUEUE_SCHEMA.md`](../QUEUE_SCHEMA.md)** | **Queue system** — index of all backlog machinery |
| **`docs/QUEUE.MD`** | Queue protocol + conventions |
| **`docs/QUEUE_STACK.csv`** | Next tasks (`agent_task` per row) |
| **`docs/QUEUE_ARCHIVE.MD`** | Open queue tables, resolved fixes (`IL-*`), gates (`HG-*`), archive (`FB-*`) |
| **`docs/queue/automation_queue_slice_prompt.md`** | Agent: one slice → PR → merge (part of queue system) |
| **[`docs/governance/full_audit.md`](../full_audit.md)** | Full-scope audit playbook; **§8** standalone report spec; skills **`draft-audit-report`**, **`audit-report-to-queue`** |
| **[`docs/reports/AUDIT_REPORT_TEMPLATE.md`](../reports/AUDIT_REPORT_TEMPLATE.md)** | Skeleton for **`AUDIT_REPORT_*.md`** deliverables |
| **`docs/operations/ready_to_run.md`** | Operator checklist (venv, `.env`, Docker, preflight, run UI) |
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
| [`../migration_to_spec_pipeline.md`](../migration_to_spec_pipeline.md) | Historical note: legacy Ridge path removed; single master pipeline |
| [`../system_walkthrough.md`](../system_walkthrough.md) | Step-by-step: live, paper, live execution, backtest (master pipeline) |
| [`../Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD`](../Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD) | Authoritative end-to-end system spec |

## Naming convention

- Files use **`.MD`** (uppercase), consistent with `docs/` in this repo.
- Specs are **topic-based**, not one file per Python module (avoids churn).
