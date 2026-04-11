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
| IL-103 | models | **Runtime vs trained artifacts:** hot path uses NumPy `ForecasterModel` methodology + `HeuristicTargetPolicy` (not campaign quantile joblib / not PyTorch weights) until **FB-SPEC-02** — fix by **explicit README/RUNBOOKS callouts** and/or **one-line startup log** when non-checkpoint policy/forecaster is active (`decision_engine/pipeline.py`, `policy_model/system.py`, [`docs/RUNBOOKS.MD`](RUNBOOKS.MD)) |
| IL-106 | docs | **Resolved/issue text drift:** [`ISSUE_LOG.MD`](ISSUE_LOG.MD) “Resolved” rows **IL-002**, **IL-003** still describe HMM + Ridge `NM_MODELS_*` paths **removed** by master pipeline work — rewrite resolved summaries so history matches current code (or archive note) |
| IL-107 | observability | **Conformal load silent fallback:** `build_forecast_packet_methodology` catches `OSError` on `load_conformal_state` and continues without conformal — add **visible log line** (WARNING) when path set but load fails so operators do not assume calibration is active (`forecaster_model/inference/build_from_ohlc.py`) |
| IL-108 | ops | **`.env.example` drift:** file uses raw `API_KEY` / `API_SECRET` names; app expects **`NM_*`** (`NM_ALPACA_*`, etc.) per `AppSettings` — audit and align example + short comment to avoid single-machine misconfiguration |
| IL-109 | runtime | **`execution_plan_to_proposal` scaling:** `size_fraction = min(1, abs(delta)*4)` in `decision_engine/spec_policy_proposal.py` is undocumented magic; audit behavior for tiny/large equity and document or replace with explicit sizing from master spec (`decision_engine/spec_policy_proposal.py`) |

---

## LOW priority (open)

| ID | Area | Summary |
|----|------|---------|
| IL-105 | ops | **Production preflight gap:** no first-class automated check that signing/secrets/adapter config match policy before live trade (complements **FB-SPEC-08**); optional smoke script or `/status` extension |
| IL-110 | control_plane | **Local bind / API key:** default `control_plane_host=0.0.0.0` exposes FastAPI on all interfaces; for single-machine untrusted LAN use **`127.0.0.1`** + keep **`NM_CONTROL_PLANE_API_KEY`** for mutating routes — document in [`RUNBOOKS.MD`](RUNBOOKS.MD) (light security hygiene) |

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

- [`FEATURES_BACKLOG.MD`](FEATURES_BACKLOG.MD) — planned work; **FB-SPEC-02 … FB-SPEC-08** and **FB-AUDIT-*** (post-audit follow-ups) cover trained weights, wiring, parity, ops (no duplicate rows: issues here are **fixes** / operator-misleading behavior)  
- [`RISK_PRECEDENCE.MD`](RISK_PRECEDENCE.MD) · [`BACKTESTING_SIMULATOR.MD`](BACKTESTING_SIMULATOR.MD)
