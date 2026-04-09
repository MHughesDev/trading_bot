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
| HG-05 | Hardening | Open | Coinbase **live**: real signed orders, cancel, fills, idempotency (not synthetic ack) | `execution/adapters/coinbase_live.py` |
| HG-06 | Hardening | Open | Full sentiment + news pipeline wired (not stubs) | `data_plane/features/pipeline.py`, `news_ingest.py` |
| HG-07 | Hardening | Open | Qdrant: real embeddings, versioned payloads, integration tests | `data_plane/memory/` |
| HG-08 | Hardening | Open | HMM trained, persisted, loaded in inference | `models/regime/`, `decision_engine/pipeline.py` |
| HG-09 | Hardening | Open | Forecast: TFT or explicit production substitute + training path | `models/forecast/tft_forecast.py` |
| HG-10 | Hardening | Open | Orchestrated train → evaluate → MLflow → manual promotion (Prefect path) | `orchestration/`, `models/registry/` |
| HG-11 | Hardening | Open | Stage latency + PnL metrics; ship logs to Loki; Grafana dashboards as code | `observability/`, `infra/` |

### Backlog items (open)

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| FB-X1 | Feature | Open | Coinbase live: CDP/JWT signing, real submit/cancel/fills, idempotency | `execution/adapters/coinbase_live.py` |
| FB-X2 | Improvement | Open | Coinbase live: `cancel_order` / `fetch_positions` real impl | same |
| FB-ML1 | Feature | Open | HMM: fit, persist artifact, load in `DecisionPipeline` | `models/regime/hmm_regime.py`, `decision_engine/pipeline.py` |
| FB-ML2 | Feature | Open | TFT: PyTorch or explicit substitute + training pipeline | `models/forecast/tft_forecast.py` |
| FB-ML3 | Feature | Open | MLflow: real training runs, artifacts; registry workflow | `models/registry/mlflow_registry.py` |
| FB-ML4 | Feature | Open | Prefect: nightly retrain job (replace stub) | `orchestration/nightly_retrain.py` |
| FB-M1 | Improvement | Open | Real embedding vectors for memory | `data_plane/memory/qdrant_memory.py` |
| FB-M2 | Feature | Open | 60s loop: top-K, symbol filter, recency → feature vector | `retrieval_loop.py`, `app/runtime/live_service.py` |
| FB-F1 | Feature | Open | Sentiment: FinBERT + frequency + shocks in `sentiment_features()` | `data_plane/features/pipeline.py` |
| FB-O1 | Feature | Open | Metrics: stage latency, PnL/drawdown, order success rate | `observability/metrics.py` |
| FB-O2 | Platform | Open | Promtail → Loki; Grafana dashboards in repo | `infra/docker-compose.yml`, `observability/logging.py` |

### Completed gates (reference)

| ID | Type | Status | Summary |
|----|------|--------|---------|
| HG-01 | Hardening | Done | Coinbase-only market data (CI) |
| HG-02 | Hardening | Done | Risk HMAC + intent gate |
| HG-03 | Hardening | Done | No raw text → trades |
| HG-04 | Hardening | Done | No auto MLflow promotion in code (CI guard) |

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
| — | — | *Move done FB/HG rows here or rely on git history.* |

---

*Priority mapping: HIGH = P0/P1, MEDIUM = P2, LOW = P3.*
