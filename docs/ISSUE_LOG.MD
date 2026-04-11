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
| IL-103 | models | **Spec pipeline vs trained weights:** default `spec_policy` still builds `ForecastPacket` via stub + `HeuristicTargetPolicy` until **FB-SPEC-02** / **FB-FR-P0** / **FB-PL-P0** land; operators may assume neural weights are live — fix by **explicit README/RUNBOOKS callouts** and/or **one-line startup log** when stub/heuristic path is active (`decision_engine/pipeline.py`, `policy_model/system.py`, [`docs/RUNBOOKS.MD`](RUNBOOKS.MD)) |
| IL-104 | ops | **Nightly train uses synthetic data:** `run_train_evaluate_log` (`orchestration/nightly_retrain.py`) fits HMM+Ridge on `_synthetic_training_data` unless replaced — add **prominent WARNING log** (and doc pointer to **FB-SPEC-04**) so operators are not misled about production readiness |

---

## LOW priority (open)

| ID | Area | Summary |
|----|------|---------|
| IL-105 | ops | **Production preflight gap:** no first-class automated check that signing/secrets/adapter config match policy before live trade (complements **FB-SPEC-08**); optional smoke script or `/status` extension |

---

## Syncing to GitHub Issues (optional)

Embedded issue titles and bodies for `gh issue create` live in **`scripts/create_github_issues.sh`**. To push them to GitHub:

1. Install and auth: `gh auth login` (repo scope).
2. From repo root: `bash scripts/create_github_issues.sh`  
   Or: `REPO=owner/repo bash scripts/create_github_issues.sh`
3. Edit the script if titles/bodies drift from this file; prefer **updating this file** for operational fixes, then mirroring to the script if you still use bulk create.

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
| IL-005 | 2026-04-10 | Sentiment: RSS/Atom fetch + dedup (`data_plane/ingest/rss_news.py`, `news_ingest.py`); `aggregate_sentiment_for_symbols_async` wired in `live_service`; optional FinBERT unchanged. |
| IL-006 | 2026-04-10 | QuestDB: batched `decision_traces` writes (`questdb_batch_max_rows`, periodic `flush_interval_seconds`); runbooks in `docs/RUNBOOKS.MD`. |
| IL-007 | 2026-04-10 | Observability: Promtail + Grafana provisioning already in compose; Streamlit links to Loki/Grafana URLs (`control_plane/streamlit_util.py`). |
| IL-009 | 2026-04-10 | Streamlit: shared API helpers + Home/Live/Routes/Models/Emergency/Logs/Regimes bound to control plane or observability URLs. |
| IL-010 | 2026-04-10 | CI: `.github/workflows/ci.yml` (ruff, pytest, spec scripts); optional integration workflow + `tests/test_integration_optional_services.py`. |
| IL-101 | 2026-04-10 | `news_ingest`: live RSS pipeline + dedup (see IL-005). |
| IL-011 | 2026-04-10 | `NotImplementedError` in `main()` signal handlers: extracted `register_shutdown_signals`; test `tests/test_live_service_signals.py`. |
| IL-102 | 2026-04-10 | WS normalizer fixtures expanded: `trade_tick.json`, `l2_snapshot.json` + tests in `test_normalizer_fixtures.py`. |
| IL-104 | 2026-04-11 | **Nightly/orchestration training:** removed `_synthetic_training_data` / Ridge-only `run_train_evaluate_log`; `orchestration/nightly_retrain.py` now drives **real Coinbase candle** fetch + walk-forward quantile forecaster + real-data heuristic RL eval per human training specs (`orchestration/training_campaign.py`). |

---

## Related

- [`FEATURES_BACKLOG.MD`](FEATURES_BACKLOG.MD) — planned work; **FB-SPEC-02 … FB-SPEC-08** cover trained weights, wiring, training data, GPU, registry, multi-asset, preflight (no duplicate rows: issues here are **fixes** / operator-misleading behavior)  
- [`RISK_PRECEDENCE.MD`](RISK_PRECEDENCE.MD) · [`BACKTESTING_SIMULATOR.MD`](BACKTESTING_SIMULATOR.MD)
