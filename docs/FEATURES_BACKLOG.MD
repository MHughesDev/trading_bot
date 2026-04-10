# Features backlog

Structured backlog for **all planned work**: new capabilities, production hardening, reliability, docs, and spikes. **Fixes to existing behavior** — separate list — live in [`ISSUE_LOG.MD`](ISSUE_LOG.MD) only.

**References:** [`RISK_PRECEDENCE.MD`](RISK_PRECEDENCE.MD) · [`QUESTDB_TRACES.MD`](QUESTDB_TRACES.MD) · [`GRACEFUL_SHUTDOWN.MD`](GRACEFUL_SHUTDOWN.MD) · [`COINBASE_GRANULARITY.MD`](COINBASE_GRANULARITY.MD) · [`BACKTESTING_SIMULATOR.MD`](BACKTESTING_SIMULATOR.MD) · [`RUNBOOKS.MD`](RUNBOOKS.MD)

---

## Priority levels

| Level | Maps from | Meaning |
|-------|-----------|---------|
| **HIGH** | P0, P1 | Blocks credible production / launch, or core product promises (live execution, models, memory, observability baseline). |
| **MEDIUM** | P2 | Important after launch: polish, platform, broader CI, runbooks, Streamlit depth. |
| **LOW** | P3 | Nice-to-have, experiments, optional CI smokes, minor replay ergonomics. |

---

## 1. Conventions

### Request types

| Type | Meaning |
|------|---------|
| **Feature** | New user- or operator-visible capability |
| **Hardening** | Production readiness: compliance gates, security, correctness under load |
| **Improvement** | Deeper implementation of something that already exists |
| **Reliability** | Ops: runbooks, backups, DR |
| **Platform** | Infra & CI: compose, workflows |
| **Tech debt** | Refactors, deduplication |
| **Spike** | Time-boxed research |

### Status

| Status | Meaning |
|--------|---------|
| **Open** | Not started |
| **In progress** | Active |
| **Done** | Shipped / verified |

---

## 2. HIGH priority

*P0 / P1 — release-critical and core architecture.*

### Compliance & release gates (open)

| ID | Type | Status | Summary | Verify / pointer |
|----|------|--------|---------|------------------|
| — | — | — | *No open HIGH gates — see Completed gates below.* | — |

### Backlog items (open)

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| — | — | — | *No open HIGH backlog rows — see §8 Completed archive.* | — |

### Completed gates (reference)

| ID | Type | Status | Summary |
|----|------|--------|---------|
| HG-01 | Hardening | Done | Coinbase-only market data (CI) |
| HG-02 | Hardening | Done | Risk HMAC + intent gate |
| HG-03 | Hardening | Done | No raw text → trades |
| HG-04 | Hardening | Done | No auto MLflow promotion in code (CI guard) |
| HG-05 | Hardening | Done | Coinbase live: CDP JWT REST submit/cancel/accounts (`execution/adapters/coinbase_live.py`, `execution/coinbase_advanced_http.py`) |
| HG-06 | Hardening | Done | Sentiment overlay wired: `aggregate_sentiment_for_symbols_async` + optional FinBERT (`data_plane/ingest/sentiment_nlp.py`); RSS in `news_ingest.py` |
| HG-07 | Hardening | Done | Qdrant: deterministic embeddings + `schema_version` payload; live per-symbol loop |
| HG-08 | Hardening | Done | HMM bootstrap + `save`/`load`; `models.regime_path` / `NM_MODELS_REGIME_PATH` |
| HG-09 | Hardening | Done | Forecast: Ridge substitute + bootstrap + `save`/`load`; `models.forecast_path` / `NM_MODELS_FORECAST_PATH` |
| HG-10 | Hardening | Done | `run_train_evaluate_log` + optional Prefect flow; MLflow log (manual promotion unchanged) |
| HG-11 | Hardening | Done | Decision latency histogram in `run_decision_tick`; Promtail + Grafana provisioning + Loki in compose |

**Release-ready (definition):** all **Open** rows in **HIGH** are **Done** or waived in writing.

---

## 3. MEDIUM priority

*P2 — important after initial launch.*

### Compliance & gates (open)

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| — | — | — | *No open items — see §8 Completed archive.* | — |

### Backlog (open)

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| — | — | — | *No open items — see §8 Completed archive.* | — |

---

## 4. LOW priority

*P3 — optional experiments and polish.*

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| — | — | — | *No open items — see §8 Completed archive.* | — |

### Out of scope (not prioritized)

| ID | Summary |
|----|---------|
| FB-N1 | Multi-exchange execution |
| FB-N2 | RL controllers |
| FB-N3 | Portfolio optimization |

---

## 5. How to add a request

1. Assign **HIGH** / **MEDIUM** / **LOW** using §Priority levels (and P0–P3 mapping).  
2. Add a row in §2, §3, or §4. Use the next **FB-** or **HG-** id.  
3. Link **code paths** or tickets.

---

## 6. Bugs and incidents

Do **not** track bugs here — use [`ISSUE_LOG.MD`](ISSUE_LOG.MD). Link IDs when a bug blocks a feature (e.g. “IL-001 blocks FB-X1”).

---

## 7. What is already in good shape

- Shared **`run_decision_tick`**; **`replay_decisions`** / **`replay_multi_asset_decisions`**.  
- **Alpaca paper** adapter; **Coinbase WS + REST** (retry, fallback).  
- **Feature pipeline** (RSI, MACD, ATR, ADX, VWAP, microstructure).  
- **Risk** + HMAC + CI guards.  
- **FastAPI** + **Prometheus**; **Compose** core services.

---

## 8. Completed archive (optional)

| ID | Completed | Notes |
|----|-------------|-------|
| FB-X1, FB-X2 | 2026-04-08 | Coinbase live JWT REST orders, cancel, positions from accounts |
| FB-ML1 | 2026-04-08 | HMM joblib + pipeline load path |
| FB-ML2 | 2026-04-08 | Ridge surrogate + bootstrap + joblib (full PyTorch TFT still optional extra) |
| FB-ML3 | 2026-04-08 | `run_train_evaluate_log` logs to MLflow when installed |
| FB-ML4 | 2026-04-08 | Prefect `@flow` when `prefect` installed; else inline train |
| FB-M1, FB-M2 | 2026-04-08 | `feature_dict_to_embedding`; live `run_memory_retrieval_loop` per symbol |
| FB-F1 | 2026-04-08 | FinBERT optional + `sentiment_features` from aggregates |
| FB-O1, FB-O2 | 2026-04-08 | `DECISION_LATENCY` in `run_decision_tick`; Promtail + Grafana JSON + datasources |
| HG-12, FB-I3 | 2026-04-10 | Runbooks: [`docs/RUNBOOKS.MD`](RUNBOOKS.MD) |
| HG-13, FB-I2 | 2026-04-10 | `.github/workflows/ci.yml`; optional integration (`workflow_dispatch` or `NM_INTEGRATION_SERVICES=1`) |
| FB-X3 | 2026-04-10 | `create_execution_adapter` in `execution/router.py` (used by `ExecutionService`) |
| FB-M3 | 2026-04-10 | Qdrant `schema_version: 1` in payload; CI integration test in `test_integration_optional_services.py` |
| FB-F2 | 2026-04-10 | RSS fetch + dedup (`rss_news.py`, `news_ingest.py`); `NM_NEWS_RSS_FEEDS` via YAML/env |
| FB-F3 | 2026-04-10 | `test_coinbase_rest_candles_multi.py` (BTC/ETH/SOL Exchange fallback path) |
| FB-S1 | 2026-04-10 | QuestDB batch flush + runbooks |
| FB-C1 | 2026-04-10 | Streamlit + `streamlit_util` API and observability links |
| FB-I1 | 2026-04-10 | README: MLflow/Prefect not in default compose; use `[all]` / orchestration docs |
| FB-F4 | 2026-04-10 | Strict `BarEvent` OHLC validation (`app/contracts/events.py`) |
| FB-B1 | 2026-04-10 | `available_cash_usd` in `RiskEngine` + `run_decision_tick`; `backtesting.replay_available_cash` |
| FB-T1 | 2026-04-10 | Alpaca smoke: `scripts/smoke_credentials.py` (no secrets in default CI) |
| FB-T2 | 2026-04-10 | `register_shutdown_signals` + `tests/test_live_service_signals.py` |

---

*Priority mapping: HIGH = P0/P1, MEDIUM = P2, LOW = P3.*
