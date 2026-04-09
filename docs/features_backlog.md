# Features backlog

**Purpose:** Things we **want to add** — gaps between **this repository today** and the **target NautilusMonster V3 architecture** (Coinbase-only market data, Alpaca paper, shared decision + risk path, typed contracts, backtest ≈ live). Items are derived from **code inspection** and product intent, not from a separate spec document.

**Related:** [`issue_log.md`](issue_log.md) (bugs and fixes in flight) · [`PRODUCTION_HARDENING.md`](PRODUCTION_HARDENING.md) (checklist stub) · [`RISK_PRECEDENCE.md`](RISK_PRECEDENCE.md)

---

## Target architecture (summary)

Multi-route AI stack: **Coinbase** → Polars features → **HMM regime** → **Qdrant memory** → **TFT forecast** → route selector → action generator → **risk engine** → execution (**Alpaca paper** | **Coinbase live**). Storage: QuestDB, Redis, Qdrant. Control: FastAPI + Streamlit. Ops: MLflow, Prefect, Prometheus/Grafana/Loki. **Rules:** Coinbase-only data; Alpaca never for data; risk cannot be bypassed; no raw news text → orders; manual model promotion only.

---

## Execution

| ID | Gap (code / intent) | Pointer |
|----|---------------------|---------|
| FB-X1 | **Coinbase live orders** — `CoinbaseExecutionAdapter` returns synthetic `OrderAck` with `status=pending_implementation`; no CDP/JWT signing, real submit/cancel/fills, or idempotency. | `execution/adapters/coinbase_live.py` |
| FB-X2 | **Cancel / positions on Coinbase live** — `cancel_order` logs stub; `fetch_positions` returns `[]`. | same |
| FB-X3 | **Single adapter factory** — ensure one construction path for paper/live in `ExecutionService` and tests (avoid drift). | `execution/service.py`, `execution/router.py` |

## Models

| ID | Gap | Pointer |
|----|-----|---------|
| FB-ML1 | **HMM not trained in production path** — `GaussianHMMRegimeModel.predict_proba_last` uses **unfitted** HMM → fixed SIDEWAYS + uniform probs until `fit()` is called with real data; no persisted artifact load in `DecisionPipeline`. | `models/regime/hmm_regime.py`, `decision_engine/pipeline.py` |
| FB-ML2 | **Forecast is Ridge surrogate, not TFT** — docstring states PyTorch TFT optional; core behavior is **sklearn Ridge** per horizon, not Temporal Fusion Transformer. | `models/forecast/tft_forecast.py` |
| FB-ML3 | **MLflow** — `MLflowModelRegistry` logs if mlflow installed; **`promote()`** is intentional no-op; no orchestrated train → evaluate → register workflow. | `models/registry/mlflow_registry.py`, `orchestration/nightly_retrain.py` |
| FB-ML4 | **Nightly retrain** — `nightly_flow_stub()` only logs; no Prefect deployment, data pull, or evaluation gate. | `orchestration/nightly_retrain.py` |

## Memory (Qdrant)

| ID | Gap | Pointer |
|----|-----|---------|
| FB-M1 | **Embeddings are placeholders** — Qdrant helpers expect vectors; live loop uses **zeros / placeholders** until a news encoder exists. | `data_plane/memory/qdrant_memory.py`, `data_plane/memory/retrieval_loop.py` |
| FB-M2 | **60s loop not feeding real memory features** — `run_memory_retrieval_loop` documents placeholder query embedding; must wire symbol filter, top-K, recency, aggregated features into `FeaturePipeline`. | `app/runtime/live_service.py`, `retrieval_loop.py` |
| FB-M3 | **Payload versioning & tests** — collection schema, embedding model version in payload, integration tests against Qdrant in CI. | `data_plane/memory/` |

## Features & data

| ID | Gap | Pointer |
|----|-----|---------|
| FB-F1 | **Sentiment** — `FeaturePipeline.sentiment_features()` returns **stub keys** (defaults 0) until FinBERT + news frequency + shocks are wired from `news_ingest`. | `data_plane/features/pipeline.py`, `data_plane/ingest/news_ingest.py` |
| FB-F2 | **News ingest** — `fetch_news_stub`; real sources + NLP pipeline TBD. | `news_ingest.py` |
| FB-F3 | **REST validation** — expand automated tests / fixtures for V1 symbols (BTC-USD, ETH-USD, SOL-USD) beyond smoke script. | `data_plane/ingest/coinbase_rest.py`, `scripts/smoke_credentials.py` |
| FB-F4 | **BarEvent-style strict schemas** — optional end-to-end `schema_version` + `source` on all bar-like events if we want spec-style contracts everywhere. | `app/contracts/`, ingest |

## Storage (QuestDB)

| ID | Gap | Pointer |
|----|-----|---------|
| FB-S1 | **QuestDB production path** — `insert_decision_trace_dict` exists; batching, retention policy, backup/restore runbooks still thin. | `data_plane/storage/questdb.py`, `docs/QUESTDB_TRACES.md` |

## Observability

| ID | Gap | Pointer |
|----|-----|---------|
| FB-O1 | **Metrics** — partial counters (feed stale, normalizer); missing full **stage latency**, PnL/drawdown gauges, order success rates as first-class series. | `observability/metrics.py` |
| FB-O2 | **Loki / Grafana wiring** — images exist in `infra/docker-compose.yml` but **no Promtail/driver** shipping app JSON logs to Loki; Grafana dashboards/alerts not checked in as code. | `infra/docker-compose.yml`, `observability/logging.py` |

## Control plane

| ID | Gap | Pointer |
|----|-----|---------|
| FB-C1 | **Streamlit** — multipage shell exists; pages are **thin** until bound to QuestDB, Loki, and mutating API for emergency actions. | `control_plane/` |

## Infra & CI

| ID | Gap | Pointer |
|----|-----|---------|
| FB-I1 | **Compose** — QuestDB, Redis, Qdrant, Prometheus, Grafana, Loki present; **MLflow and Prefect** not in compose; ports/docs may drift from README. | `infra/docker-compose.yml` |
| FB-I2 | **CI** — no `.github/workflows` in repo; integration tests (Redis, QuestDB, Qdrant) and optional E2E paper dry run not automated here. | — |
| FB-I3 | **Runbooks** — secrets rotation, incident, flatten, QuestDB restore — operational docs TBD. | `docs/` |

## Backtesting

| ID | Gap | Pointer |
|----|-----|---------|
| FB-B1 | **RiskEngine + replay cash** — solvency enforced in replay layer; optional pass **`available_cash`** into `RiskEngine` for replay-only alignment. | `backtesting/replay.py`, `risk_engine/engine.py` |

## Tests & tooling

| ID | Gap | Pointer |
|----|-----|---------|
| FB-T1 | **Alpaca paper CI** — optional integration test against live paper API (secrets). | `execution/adapters/alpaca_paper.py` |
| FB-T2 | **Shutdown tests** — SIGINT/SIGTERM handling not exercised in CI. | `app/runtime/live_service.py`, `docs/GRACEFUL_SHUTDOWN.md` |

## Nice-to-haves (out of V1 scope)

| ID | Note |
|----|------|
| FB-N1 | Multi-exchange execution |
| FB-N2 | RL controllers |
| FB-N3 | Portfolio optimization |

---

## What is already in good shape (short)

- Shared **`run_decision_tick`** for live and replay; **`replay_decisions`** / **`replay_multi_asset_decisions`** with portfolio + solvency options.
- **Alpaca paper** adapter with retries, symbol mapping, optional position reconcile.
- **Coinbase WS + REST** (including Advanced Trade → Exchange fallback on 401/403).
- **Feature pipeline** implements RSI, MACD, ATR, ADX, EMA spread, VWAP distance, microstructure helper; sentiment hooks exist but need real inputs.
- **Risk engine** modes, HMAC gate, CI guards (`ci_spec_compliance.sh`, `ci_mlflow_promotion_policy.sh`).
- **Control plane FastAPI** + **Prometheus** scrape on metrics route.
- **`infra/docker-compose.yml`** — core data stores + Prometheus + Grafana + Loki images.

---

## How to add an item

1. Add a row with the next **FB-** id.  
2. Point to **files or symbols** so the gap is verifiable.  
3. When done, move the row to a **Done** subsection or delete it.

---

*Backlog maintained against the codebase; update when architecture or code changes.*
