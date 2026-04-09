# Features backlog

Structured backlog for **all planned work**: new capabilities, production hardening, reliability, docs, and spikes. **Fixes to existing behavior** — separate list — live in [`ISSUE_LOG.MD`](ISSUE_LOG.MD) only.

**References:** [`RISK_PRECEDENCE.MD`](RISK_PRECEDENCE.MD) · [`QUESTDB_TRACES.MD`](QUESTDB_TRACES.MD) · [`GRACEFUL_SHUTDOWN.MD`](GRACEFUL_SHUTDOWN.MD) · [`COINBASE_GRANULARITY.MD`](COINBASE_GRANULARITY.MD) · [`BACKTESTING_SIMULATOR.MD`](BACKTESTING_SIMULATOR.MD)

---

## 1. Conventions

### Request types

| Type | Meaning |
|------|---------|
| **Feature** | New user- or operator-visible capability |
| **Hardening** | Production readiness: compliance gates, security, correctness under load |
| **Improvement** | Deeper implementation of something that already exists (models, metrics, UI) |
| **Reliability** | Ops: runbooks, backups, DR, on-call clarity |
| **Platform** | Infra & CI: compose, workflows, test environments |
| **Tech debt** | Refactors, deduplication, cleanup |
| **Spike** | Time-boxed research; outcome may be a Feature or “won’t do” |

### Priority

| P | Meaning |
|---|---------|
| **P0** | Blocks safe or correct production use |
| **P1** | Needed for credible production / launch |
| **P2** | Important but can follow launch |
| **P3** | Nice-to-have |

### Status

| Status | Meaning |
|--------|---------|
| **Open** | Not started |
| **In progress** | Active |
| **Done** | Shipped / verified — keep for history or move to §7 |

---

## 2. Compliance & release gates (hardening)

These are **non-functional requirements** tracked as hardening items. Release “done” when all **Open** gates here are closed or explicitly waived.

| ID | Type | P | Status | Summary | Verify |
|----|------|---|--------|---------|--------|
| HG-01 | Hardening | — | Done | Coinbase-only market data — no Alpaca data imports outside paper adapter | `scripts/ci_spec_compliance.sh` |
| HG-02 | Hardening | — | Done | Risk HMAC + intent gate when signing enabled | `execution/intent_gate.py`, `NM_RISK_SIGNING_SECRET` |
| HG-03 | Hardening | — | Done | No raw text → trades — `OrderIntent` metadata rules | contracts + validators |
| HG-04 | Hardening | — | Done | No automatic MLflow model stage promotion in code | `scripts/ci_mlflow_promotion_policy.sh`, `docs/MLFLOW_PROMOTION.MD` |
| HG-05 | Hardening | P0 | Open | Coinbase **live**: real signed orders, cancel, fills, idempotency (not synthetic ack) | `execution/adapters/coinbase_live.py` → same as FB-X1 |
| HG-06 | Hardening | P1 | Open | Full sentiment + news pipeline wired (not stubs) | `data_plane/features/pipeline.py`, `news_ingest.py` → FB-F1/F2 |
| HG-07 | Hardening | P1 | Open | Qdrant: real embeddings, versioned payloads, integration tests | → FB-M1–M3 |
| HG-08 | Hardening | P1 | Open | HMM trained, persisted, loaded in inference | → FB-ML1 |
| HG-09 | Hardening | P1 | Open | Forecast: TFT or explicit production substitute + training path | → FB-ML2 |
| HG-10 | Hardening | P1 | Open | Orchestrated train → evaluate → MLflow → manual promotion (Prefect path) | → FB-ML3/ML4 |
| HG-11 | Hardening | P1 | Open | Stage latency + PnL metrics; ship logs to Loki; Grafana dashboards as code | → FB-O1/O2 |
| HG-12 | Hardening | P2 | Open | Runbooks: secrets, incident, flatten, QuestDB restore | → FB-I3 |
| HG-13 | Hardening | P2 | Open | CI integration tests: Redis, QuestDB, Qdrant | → FB-I2 |

**Definition of release-ready:** all **Open** rows in §2 and §3 that are **P0–P1** are **Done** or waived in writing; release notes reference commit.

---

## 3. Backlog by area

Columns: **ID** | **Type** | **P** | **Status** | **Summary** | **Pointer**

### Execution & routing

| ID | Type | P | Status | Summary | Pointer |
|----|------|---|--------|---------|---------|
| FB-X1 | Feature | P0 | Open | Coinbase live: CDP/JWT signing, real submit/cancel/fills, idempotency | `execution/adapters/coinbase_live.py` |
| FB-X2 | Improvement | P1 | Open | Coinbase live: `cancel_order` / `fetch_positions` real impl | same |
| FB-X3 | Tech debt | P2 | Open | Single adapter factory for paper/live in `ExecutionService` + tests | `execution/service.py`, `execution/router.py` |

### Models & orchestration

| ID | Type | P | Status | Summary | Pointer |
|----|------|---|--------|---------|---------|
| FB-ML1 | Feature | P1 | Open | HMM: fit on data, persist artifact, load in `DecisionPipeline` (no unfitted default path) | `models/regime/hmm_regime.py`, `decision_engine/pipeline.py` |
| FB-ML2 | Feature | P1 | Open | TFT: PyTorch model or explicit substitute + training pipeline | `models/forecast/tft_forecast.py` |
| FB-ML3 | Feature | P1 | Open | MLflow: real training runs, artifacts; registry workflow (promote stays manual) | `models/registry/mlflow_registry.py` |
| FB-ML4 | Feature | P1 | Open | Prefect: nightly retrain job (replace `nightly_flow_stub`) | `orchestration/nightly_retrain.py` |

### Memory (Qdrant)

| ID | Type | P | Status | Summary | Pointer |
|----|------|---|--------|---------|---------|
| FB-M1 | Improvement | P1 | Open | Real embedding vectors for memory (not zeros/placeholders) | `data_plane/memory/qdrant_memory.py` |
| FB-M2 | Feature | P1 | Open | 60s loop: top-K, symbol filter, recency → feature vector in pipeline | `retrieval_loop.py`, `app/runtime/live_service.py` |
| FB-M3 | Hardening | P2 | Open | Payload schema version + integration tests vs Qdrant in CI | `data_plane/memory/` |

### Features & data

| ID | Type | P | Status | Summary | Pointer |
|----|------|---|--------|---------|---------|
| FB-F1 | Feature | P1 | Open | Sentiment: FinBERT + frequency + shocks wired into `sentiment_features()` | `data_plane/features/pipeline.py` |
| FB-F2 | Feature | P2 | Open | News ingest: replace `fetch_news_stub` with real sources + dedup | `data_plane/ingest/news_ingest.py` |
| FB-F3 | Improvement | P2 | Open | REST: more tests/fixtures for BTC/ETH/SOL candles | `data_plane/ingest/coinbase_rest.py` |
| FB-F4 | Spike | P3 | Open | Optional strict `BarEvent` schema (`schema_version`, `source`) end-to-end | `app/contracts/` |

### Storage

| ID | Type | P | Status | Summary | Pointer |
|----|------|---|--------|---------|---------|
| FB-S1 | Reliability | P2 | Open | QuestDB: batching, retention, backup/restore runbooks | `data_plane/storage/questdb.py` |

### Observability

| ID | Type | P | Status | Summary | Pointer |
|----|------|---|--------|---------|---------|
| FB-O1 | Feature | P1 | Open | Metrics: stage latency, PnL/drawdown, order success rate | `observability/metrics.py` |
| FB-O2 | Platform | P1 | Open | Promtail (or equivalent) → Loki; Grafana dashboards in repo | `infra/docker-compose.yml`, `observability/logging.py` |

### Control plane

| ID | Type | P | Status | Summary | Pointer |
|----|------|---|--------|---------|---------|
| FB-C1 | Feature | P2 | Open | Streamlit: bind pages to API + QuestDB/Loki + emergency actions | `control_plane/` |

### Platform & CI

| ID | Type | P | Status | Summary | Pointer |
|----|------|---|--------|---------|---------|
| FB-I1 | Platform | P2 | Open | Compose: add MLflow/Prefect or document omission; align ports with README | `infra/docker-compose.yml` |
| FB-I2 | Platform | P2 | Open | GitHub Actions: lint, pytest, spec scripts; integration job for Redis/QuestDB/Qdrant | `.github/workflows/` |
| FB-I3 | Reliability | P2 | Open | Runbooks (secrets, incident, flatten, QuestDB) | `docs/` |

### Backtesting

| ID | Type | P | Status | Summary | Pointer |
|----|------|---|--------|---------|---------|
| FB-B1 | Improvement | P3 | Open | Optional `available_cash` in `RiskEngine` for replay solvency | `backtesting/replay.py`, `risk_engine/engine.py` |

### Quality & tests

| ID | Type | P | Status | Summary | Pointer |
|----|------|---|--------|---------|---------|
| FB-T1 | Platform | P3 | Open | Optional CI: Alpaca paper API smoke (secrets) | `execution/adapters/alpaca_paper.py` |
| FB-T2 | Hardening | P3 | Open | SIGINT/SIGTERM shutdown tests | `app/runtime/live_service.py` |

### Out of scope (reference)

| ID | Type | Status | Summary |
|----|------|--------|---------|
| FB-N1 | Feature | — | Multi-exchange execution |
| FB-N2 | Feature | — | RL controllers |
| FB-N3 | Feature | — | Portfolio optimization |

---

## 4. How to add a request

1. Pick **Type** and **Priority** using §1.  
2. Add a row in §3 (or §2 if it is a **gate**). Use the next **FB-** or **HG-** id.  
3. Link **code paths** or tickets.  
4. When **Done**, set status; optionally move to §7 or keep for audit.

---

## 5. Bugs and incidents

Do **not** track bugs or “fix this component” items here — use [`ISSUE_LOG.MD`](ISSUE_LOG.MD). Link IDs when a bug blocks a feature (e.g. “IL-001 blocks FB-X1”).

---

## 6. What is already in good shape

- Shared **`run_decision_tick`**; **`replay_decisions`** / **`replay_multi_asset_decisions`** with portfolio + solvency.  
- **Alpaca paper** adapter (retries, symbol map, optional reconcile).  
- **Coinbase WS + REST** (retry, Advanced Trade → Exchange fallback on 401/403).  
- **Feature pipeline:** RSI, MACD, ATR, ADX, EMA spread, VWAP, microstructure helper.  
- **Risk** modes + HMAC gate + **CI guards** (`ci_spec_compliance.sh`, `ci_mlflow_promotion_policy.sh`).  
- **FastAPI** control plane + **Prometheus** metrics route.  
- **Compose:** QuestDB, Redis, Qdrant, Prometheus, Grafana, Loki images.

---

## 7. Completed (optional archive)

*Move rows here when done, or rely on git history.*

| ID | Completed | Notes |
|----|-------------|-------|
| — | — | — |

---

*Last structure update: single backlog for features + hardening + platform; replaces standalone production-hardening checklist.*
