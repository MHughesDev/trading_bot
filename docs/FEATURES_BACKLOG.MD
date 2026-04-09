# Features backlog

Structured backlog for **all planned work**: new capabilities, production hardening, reliability, docs, and spikes. **Fixes to existing behavior** — separate list — live in [`ISSUE_LOG.MD`](ISSUE_LOG.MD) only.

**References:** [`RISK_PRECEDENCE.MD`](RISK_PRECEDENCE.MD) · [`QUESTDB_TRACES.MD`](QUESTDB_TRACES.MD) · [`GRACEFUL_SHUTDOWN.MD`](GRACEFUL_SHUTDOWN.MD) · [`COINBASE_GRANULARITY.MD`](COINBASE_GRANULARITY.MD) · [`BACKTESTING_SIMULATOR.MD`](BACKTESTING_SIMULATOR.MD)

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
|------|--------|
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
| HG-06 | Hardening | Done | Sentiment overlay wired: `aggregate_sentiment_for_symbols` + optional FinBERT (`data_plane/ingest/sentiment_nlp.py`); news still stub until provider |
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
| HG-12 | Hardening | Open | Runbooks: secrets, incident, flatten, QuestDB restore | `docs/` |
| HG-13 | Hardening | Open | CI integration tests: Redis, QuestDB, Qdrant | `.github/workflows/` |

### Backlog (open)

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| FB-X3 | Tech debt | Open | Single adapter factory for paper/live | `execution/service.py`, `execution/router.py` |
| FB-M3 | Hardening | Open | Qdrant payload schema version + CI integration tests | `data_plane/memory/` |
| FB-F2 | Feature | Open | News ingest: real sources + dedup | `data_plane/ingest/news_ingest.py` |
| FB-F3 | Improvement | Open | REST: more tests/fixtures for BTC/ETH/SOL candles | `data_plane/ingest/coinbase_rest.py` |
| FB-S1 | Reliability | Open | QuestDB: batching, retention, backup/restore runbooks | `data_plane/storage/questdb.py` |
| FB-C1 | Feature | Open | Streamlit: bind pages to API + QuestDB/Loki + emergency | `control_plane/` |
| FB-I1 | Platform | Open | Compose: MLflow/Prefect or document omission; README ports | `infra/docker-compose.yml` |
| FB-I2 | Platform | Open | GitHub Actions: lint, pytest, spec scripts, integration job | `.github/workflows/` |
| FB-I3 | Reliability | Open | Runbooks (secrets, incident, flatten, QuestDB) | `docs/` |

---

## 4. LOW priority

*P3 — optional experiments and polish.*

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| FB-F4 | Spike | Open | Optional strict `BarEvent` schema end-to-end | `app/contracts/` |
| FB-B1 | Improvement | Open | Optional `available_cash` in `RiskEngine` for replay solvency | `backtesting/replay.py`, `risk_engine/engine.py` |
| FB-T1 | Platform | Open | Optional CI: Alpaca paper API smoke (secrets) | `execution/adapters/alpaca_paper.py` |
| FB-T2 | Hardening | Open | SIGINT/SIGTERM shutdown tests | `app/runtime/live_service.py` |

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
| FB-F1 | 2026-04-08 | FinBERT optional + `sentiment_features` from `aggregate_sentiment_for_symbols` |
| FB-O1, FB-O2 | 2026-04-08 | `DECISION_LATENCY` in `run_decision_tick`; Promtail + Grafana JSON + datasources |

---

*Priority mapping: HIGH = P0/P1, MEDIUM = P2, LOW = P3.*
