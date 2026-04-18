# Codebase specs (derived)

> **Architecture status:** This file describes the **repository as implemented today**. For **target** system architecture during the canonical APEX migration, see [`../CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD) and [`../Human Provided Specs/new_specs/canonical/`](../Human%20Provided%20Specs/new_specs/canonical/). Where this document conflicts with canonical APEX specs on **future** design, treat this file as **as-built** only, not the roadmap.

**FB-CAN-023 — Precedence:** Every file in **`docs/Specs/`** is **as-built engineering documentation**, not the APEX contract. For ordering rules, read **[`CANONICAL_PRECEDENCE.MD`](CANONICAL_PRECEDENCE.MD)**. Do not treat `docs/Specs/*.MD` as authoritative for **future** canonical requirements.

This folder holds **specifications that describe the repository as implemented**. They are maintained to stay aligned with the code; when behavior changes, update the relevant file here (or ask an agent to).

**Not product marketing** — these are engineer-facing maps: modules, responsibilities, main entry points, and boundaries (e.g. Kraken-only market data, no Alpaca *data* imports outside the paper adapter, Alpaca paper / Coinbase live execution).

## How this relates to other docs

| Location | Role |
|----------|------|
| **[`docs/CANONICAL_SPEC_INDEX.MD`](../CANONICAL_SPEC_INDEX.MD)** | **Target** APEX architecture (canonical `new_specs/canonical/`) — precedence vs as-built |
| **[`CANONICAL_PRECEDENCE.MD`](CANONICAL_PRECEDENCE.MD)** | **FB-CAN-023** — explicit ordering: canonical vs as-built vs legacy human specs |
| **[`docs/CANONICAL_MODULE_MAP.MD`](../CANONICAL_MODULE_MAP.MD)** | **Code ↔ domains** — primary Python modules for state/trigger/auction/risk/execution/replay/governance |
| **`docs/Specs/`** (this folder) | What the code **is** and **does** today |
| **`docs/Human Provided Specs/new_specs/canonical/`** | Canonical APEX specs (see index above) |
| **`docs/Human Provided Specs/`** (root `*.MD`) | Legacy human intent; backlog driver; compare to code + Specs |
| **[`docs/QUEUE_SCHEMA.md`](../QUEUE_SCHEMA.md)** | **Queue system** — index of all backlog machinery |
| **`docs/QUEUE.MD`** | Queue protocol + conventions |
| **`docs/QUEUE_STACK.csv`** | Next tasks (`agent_task` per row) |
| **`docs/QUEUE_ARCHIVE.MD`** | Open queue tables, resolved fixes (`IL-*`), gates (`HG-*`), archive (`FB-*`) |
| **`docs/AUTOMATION_QUEUE_SLICE_PROMPT.MD`** | Agent: one slice → PR → merge (part of queue system) |
| **[`docs/FULL_AUDIT.md`](../FULL_AUDIT.md)** | Full-scope audit playbook; **§8** standalone report spec; skills **`draft-audit-report`**, **`audit-report-to-queue`** |
| **[`docs/reports/AUDIT_REPORT_TEMPLATE.md`](../reports/AUDIT_REPORT_TEMPLATE.md)** | Skeleton for **`AUDIT_REPORT_*.md`** deliverables |
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
| [`MULTI_ASSET_PORTFOLIO.MD`](MULTI_ASSET_PORTFOLIO.MD) | Independent ticks vs shared risk caps; no cross-symbol optimizer (FB-SPEC-07) |
| [`../MIGRATION_TO_SPEC_PIPELINE.MD`](../MIGRATION_TO_SPEC_PIPELINE.MD) | Historical note: legacy Ridge path removed; single master pipeline |
| [`../SYSTEM_WALKTHROUGH.MD`](../SYSTEM_WALKTHROUGH.MD) | Step-by-step: live, paper, live execution, backtest (master pipeline) |
| [`../Human Provided Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD`](../Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD) | Authoritative end-to-end system spec |

## Naming convention

- Files use **`.MD`** (uppercase), consistent with `docs/` in this repo.
- Specs are **topic-based**, not one file per Python module (avoids churn).
