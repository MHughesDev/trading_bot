# Issue log

**Purpose:** A single place for **things we need to fix** on **existing** functions and components — bugs, incorrect behavior, stubs that break contracts, missing wiring in code paths that already exist, and operational/debt items that are **fixes**, not new capabilities.

**Not here:** new features, greenfield work, or release gates — use **[`FEATURES_BACKLOG.MD`](FEATURES_BACKLOG.MD)** only. No duplicate line items between this file and the backlog.

---

## Priority levels

| Level | Meaning |
|-------|---------|
| **HIGH** | Blocks or severely misrepresents production behavior; safety, correctness, or core pipeline integrity. |
| **MEDIUM** | Important gaps in observability, ops, UX, or data quality that do not block basic paper/sandbox use. |
| **LOW** | Test coverage, polish, or minor verification gaps. |

---

## Conventions

| Field | Meaning |
|-------|---------|
| **ID** | `IL-###` (local) |
| **Area** | data, models, execution, risk, runtime, backtest, control_plane, observability, ops, ci |
| **Status** | Open / In progress / Done |

When you close an item: set **Status** to **Done**, add **Resolved** date + commit/PR in **Resolved (recent)**, or remove the row.

---

## HIGH priority (open)

| ID | Area | Summary |
|----|------|---------|
| — | — | *No open HIGH items — see Resolved (recent).* |

---

## MEDIUM priority (open)

| ID | Area | Summary |
|----|------|---------|
| IL-005 | features | Sentiment / FinBERT / news pipeline returns zeros or stubs end-to-end. |
| IL-006 | storage | QuestDB: batching, retention, backup runbooks incomplete vs production needs. |
| IL-007 | observability | Loki: no Promtail (or equivalent) wiring from app logs; Grafana dashboards not as code. |
| IL-009 | control_plane | Streamlit pages thin vs live API + storage. |
| IL-010 | ci | No GitHub Actions workflow; integration tests for Redis/QuestDB/Qdrant not in repo CI. |
| IL-101 | data | `news_ingest`: `fetch_news_stub` only — no live news pipeline. |

---

## LOW priority (open)

| ID | Area | Summary |
|----|------|---------|
| IL-011 | runtime | `NotImplementedError` handler paths in `live_service` — verify coverage. |
| IL-102 | data | Expand WS normalizer fixtures beyond current samples. |

---

## Syncing to GitHub Issues (optional)

Embedded issue titles and bodies for `gh issue create` live in **`scripts/create_github_issues.sh`**. To push them to GitHub:

1. Install and auth: `gh auth login` (repo scope).
2. From repo root: `bash scripts/create_github_issues.sh`  
   Or: `REPO=owner/repo bash scripts/create_github_issues.sh`
3. Edit the script if titles/bodies drift from this log; prefer **updating this file** for operational fixes, then mirroring to the script if you still use bulk create.

**Note:** GitHub issues are optional; this markdown file remains the **local** source of truth for fix tracking.

---

## Resolved (recent)

| ID | Resolved | Summary |
|----|----------|---------|
| IL-001 | 2026-04-08 | Coinbase live: CDP JWT REST for `submit_order` / `cancel_order` / `fetch_positions` via `execution/coinbase_advanced_http.py` (no synthetic `pending_implementation` ack). |
| IL-002 | 2026-04-08 | HMM: bootstrap fit on 32-dim synthetic features + `save`/`load`; `DecisionPipeline` loads `NM_MODELS_REGIME_PATH` when set. |
| IL-003 | 2026-04-08 | Forecast: Ridge surrogate bootstrapped + `save`/`load`; `NM_MODELS_FORECAST_PATH`; optional PyTorch path still via extras. |
| IL-004 | 2026-04-08 | Memory: deterministic `feature_dict_to_embedding` for Qdrant queries; per-symbol 60s loop; payload `schema_version: 1`. |
| IL-008 | 2026-04-08 | Orchestration: `run_train_evaluate_log` + optional Prefect `@flow` in `orchestration/nightly_retrain.py`; MLflow params/metrics (no auto-promotion). |

---

## Related

- [`FEATURES_BACKLOG.MD`](FEATURES_BACKLOG.MD) — planned work only (no overlap with rows above)  
- [`RISK_PRECEDENCE.MD`](RISK_PRECEDENCE.MD) · [`BACKTESTING_SIMULATOR.MD`](BACKTESTING_SIMULATOR.MD)
