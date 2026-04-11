# Features backlog

Structured backlog for **all planned work**: new capabilities, production hardening, reliability, platform work, and improvements. **Fixes to existing behavior** — separate list — live in [`ISSUE_LOG.MD`](ISSUE_LOG.MD) only.

**References:** [`RISK_PRECEDENCE.MD`](RISK_PRECEDENCE.MD) · [`QUESTDB_TRACES.MD`](QUESTDB_TRACES.MD) · [`GRACEFUL_SHUTDOWN.MD`](GRACEFUL_SHUTDOWN.MD) · [`COINBASE_GRANULARITY.MD`](COINBASE_GRANULARITY.MD) · [`BACKTESTING_SIMULATOR.MD`](BACKTESTING_SIMULATOR.MD) · [`RUNBOOKS.MD`](RUNBOOKS.MD) · [`Human Provided Specs/`](Human%20Provided%20Specs/README.MD)

**Status:** Open items below track **spec pipeline rollout**, **human-spec forecaster + RL policy**, and **known production gaps** (artifacts, ops). Completed items archive to §8.

---

## Contents

| § | Section |
|---|---------|
| [§1](#1-conventions) | Conventions (types, status) |
| [§2](#2-high-priority) | HIGH — open items + gates |
| [§3](#3-medium-priority) | MEDIUM — open items |
| [§4](#4-low-priority) | LOW + out of scope |
| [§5–§7](#5-how-to-add-a-request) | How to add, bugs policy, healthy areas |
| [§8](#8-completed-archive-optional) | Completed archive (by theme) |

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

### ID granularity

- **Epic rows** (`FB-FR-P0`, `FB-PL-P0`) are decomposed into **sub-rows** (`FB-FR-P0-01` …) so work can be tracked and closed incrementally without losing the parent goal in [`FORECASTER_MODEL_ARCHITECTURE_SPEC.MD`](Human%20Provided%20Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD) / [`POLICY_MODEL_ARCHITECTURE_SPEC.MD`](Human%20Provided%20Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD).

---

## 2. HIGH priority

*P0 / P1 — release-critical and core architecture.*

### Compliance & release gates (open)

| ID | Type | Status | Summary | Verify / pointer |
|----|------|--------|---------|------------------|
| — | — | — | *No open HIGH gates — see §8 Completed archive.* | — |

### Backlog items (open) — forecaster epic **FB-FR-P0**

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| FB-FR-P0-01 | Feature | Open | **Data:** leakage-safe windows, `DatasetManifest` / walk-forward alignment with master spec | `forecaster_model/data/`, `forecaster_model/training/` |
| FB-FR-P0-02 | Feature | Open | **Graph:** VSN → latent CNN → multi-res xLSTM → regime fusion → quantile head (PyTorch), spec geometry | `forecaster_model/models/` |
| FB-FR-P0-03 | Feature | Open | **Train:** end-to-end PyTorch training loop, checkpoints, optional `[models_torch]` / `NM_MODELS_TORCH_DEVICE` | `forecaster_model/training/torch_trainer.py` |
| FB-FR-P0-04 | Feature | Open | **Calibrate:** conformal + ensemble integration on trained outputs (not RNG reference only) | `forecaster_model/calibration/`, `forecaster_model/models/ensemble.py` |
| FB-FR-P0-05 | Feature | Open | **Serve:** export path from trained checkpoint to hot path (extends NPZ reference / replaces RNG-only) | `decision_engine/pipeline.py`, `forecaster_model/inference/` |
| FB-SPEC-07 | Feature | Open | **Multi-asset portfolio coordination:** cross-symbol capital allocation, correlation-aware or risk-parity style sizing, or documented limits when running many `market_data_symbols` (beyond independent per-symbol ticks) | `risk_engine/`, `decision_engine/`, `docs/Specs/` |

*Parent epic:* full neural forecaster per [`FORECASTER_MODEL_ARCHITECTURE_SPEC.MD`](Human%20Provided%20Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD). **Close epic** when **FB-FR-P0-01 … FB-FR-P0-05** are Done or explicitly waived.

**Blocked (not started):** PyTorch/xLSTM stack + large training loop — unsafe to claim done without full model + data pipeline validation.

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

### Backlog items (open) — policy epic **FB-PL-P0**

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| — | — | — | *No open sub-rows — see §8.* | — |

*Parent epic:* RL policy per [`POLICY_MODEL_ARCHITECTURE_SPEC.MD`](Human%20Provided%20Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD). **Sub-rows FB-PL-P0-01 … FB-PL-P0-04 Done 2026-04-11** (NumPy stubs + offline env; full PyTorch RL remains future hardening).

*See also §2 HIGH (**FB-FR-P0-***, **FB-SPEC-07**).*

---

## 4. LOW priority

*P3 — optional experiments and polish.*

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| — | — | — | *No open LOW items — see §8.* | — |

### Out of scope (not prioritized)

| ID | Summary |
|----|---------|
| FB-N1 | Multi-exchange execution |
| FB-N3 | Portfolio optimization (generic portfolio optimization libs) |

---

## 5. How to add a request

1. Assign **HIGH** / **MEDIUM** / **LOW** using §Priority levels (and P0–P3 mapping).  
2. Add a row in §2, §3, or §4. Use the next **FB-** or **HG-** id; for large epics prefer **-01, -02** sub-ids.  
3. Link **code paths** or tickets.

---

## 6. Bugs and incidents

Do **not** track bugs here — use [`ISSUE_LOG.MD`](ISSUE_LOG.MD). Link IDs when a bug blocks a feature (e.g. “IL-001 blocks FB-X1”).

---

## 7. What is already in good shape

| Area | Notes |
|------|--------|
| **Decision** | Shared **`run_decision_tick`**; **`replay_decisions`** / **`replay_multi_asset_decisions`**. |
| **Execution** | **Alpaca paper** adapter; **Coinbase WS + REST** (retry, fallback). |
| **Features** | **Feature pipeline** (RSI, MACD, ATR, ADX, VWAP, microstructure). |
| **Risk** | **Risk** + HMAC + CI guards. |
| **Control / observability** | **FastAPI** + **Prometheus**; **Compose** core services. |

---

## 8. Completed archive (optional)

Grouped by theme for quicker lookup. *Chronological merge history is preserved in git; this table is operator-oriented.*

### 8.1 Execution, data, memory

| ID | Completed | Notes |
|----|-------------|-------|
| FB-X1, FB-X2 | 2026-04-08 | Coinbase live JWT REST orders, cancel, positions from accounts |
| FB-X3 | 2026-04-10 | `create_execution_adapter` in `execution/router.py` (used by `ExecutionService`) |
| FB-F3 | 2026-04-10 | `test_coinbase_rest_candles_multi.py` (BTC/ETH/SOL Exchange fallback path) |
| FB-M1, FB-M2 | 2026-04-08 | `feature_dict_to_embedding`; live `run_memory_retrieval_loop` per symbol |
| FB-M3 | 2026-04-10 | Qdrant `schema_version: 1` in payload; CI integration test in `test_integration_optional_services.py` |
| FB-F2 | 2026-04-10 | RSS fetch + dedup (`rss_news.py`, `news_ingest.py`); `NM_NEWS_RSS_FEEDS` via YAML/env |
| FB-F4 | 2026-04-10 | Strict `BarEvent` OHLC validation (`app/contracts/events.py`) |

### 8.2 Models, training, orchestration (summary)

| ID | Completed | Notes |
|----|-------------|-------|
| FB-ML1–ML4 | 2026-04-08 | HMM / Ridge / MLflow / Prefect orchestration *(historical paths; master pipeline superseded HMM/Ridge runtime)* |
| FB-FR-P0 | 2026-04-11 | **Partial deliverable** (superseded by §2 **FB-FR-P0-01…**): NumPy `ForecasterModel` + ensemble + optional `train_forecaster_torch` |
| FB-FR-P1 | 2026-04-11 | Walk-forward splits, pinball, JSON checkpoints (`forecaster_model/training/`) |
| FB-PL-P0 | 2026-04-11 | Epic parent: `run_stub_episode`; sub-rows **FB-PL-P0-01…04** completed 2026-04-11 — §8.5 |
| FB-PL-P0-01 … FB-PL-P0-04 | 2026-04-11 | Protocol runtime check, offline `ForecastPacket` env, buffer/trainer + reward tests — `tests/test_policy_pl_p0.py` |

### 8.3 Spec pipeline, master path, ops (summary)

| ID | Completed | Notes |
|----|-------------|-------|
| FB-SPEC-00 | 2026-04-11 | Spec pipeline wiring (`decision_pipeline_mode` — **removed** later; see [`MIGRATION_TO_SPEC_PIPELINE.MD`](MIGRATION_TO_SPEC_PIPELINE.MD)) |
| FB-SPEC-01 | 2026-04-11 | Default `spec_policy` *(historical row)* |
| FB-SPEC-02–08 | 2026-04-11 | NPZ hot path, artifact contract, torch device, registry, multi-asset doc, preflight |
| FB-SPEC-04 | 2026-04-11 | Real historical training (`training_campaign`, Coinbase-only) |
| FB-HS-01–04 | 2026-04-10 / 11 | Human specs, methodology, layout |
| FB-PL-P2b | 2026-04-10 | `policy_model.risk_bridge` |
| HG-12, FB-I3 | 2026-04-10 | Runbooks [`RUNBOOKS.MD`](RUNBOOKS.MD) |

### 8.4 UI, observability, CI, backtesting (summary)

| ID | Completed | Notes |
|----|-------------|-------|
| FB-O1, FB-O2 | 2026-04-08 | Decision latency metric; Promtail + Grafana |
| FB-S1 | 2026-04-10 | QuestDB batch flush + runbooks |
| FB-C1 | 2026-04-10 | Streamlit + `streamlit_util` |
| FB-I1, FB-I2, HG-13 | 2026-04-10 | README orchestration note; CI workflow + optional integration |
| FB-B1 | 2026-04-10 | `available_cash_usd` / replay cash |
| FB-T1, FB-T2 | 2026-04-10 | Alpaca smoke; shutdown signals |

### 8.5 Complete archive (one row per ID)

*Sorted by **ID** for lookup. Theme tags are non-normative.*

| ID | Theme | Completed | Notes |
|----|-------|-------------|-------|
| FB-AUDIT-01 | ops | 2026-04-11 | `ForecasterConfig` parity: `_cfg_from_spec` vs runtime (`tests/test_training_campaign_config.py`) |
| FB-AUDIT-02 | ops | 2026-04-11 | Campaign vs runtime docstring (`training_campaign.py`) |
| FB-AUDIT-03 | models | 2026-04-11 | Promotion compare: `training_report.json` scores (`tests/test_promotion_decision.py`) |
| FB-AUDIT-04 | docs | 2026-04-11 | Archive note: legacy `decision_pipeline_mode` rows pre–master-pipeline |
| FB-AUDIT-05 | test | 2026-04-11 | Golden `ForecastPacket` tests + fixtures |
| FB-AUDIT-06 | data | 2026-04-11 | Flat OHLC runbook + `ohlc_arrays_from_feature_row` docstring |
| FB-AUDIT-07 | observability | 2026-04-11 | `forecast_packet_summary` on `decision_trace` |
| FB-AUDIT-08 | observability | 2026-04-11 | Grafana ↔ Prometheus doc (`OBSERVABILITY_AND_INFRA.MD`) |
| FB-AUDIT-09 | ops | 2026-04-11 | Single-host alerting pointers (`RUNBOOKS.MD`) |
| FB-AUDIT-10 | ci | 2026-04-11 | Integration test cadence (`TESTING_AND_CI.MD`) |
| FB-AUDIT-11 | test | 2026-04-11 | `test_property_invariants.py` |
| FB-AUDIT-12 | docs | 2026-04-11 | `MASTER_SPEC_RISK_STATE_GAP.MD` |
| FB-AUDIT-13 | platform | 2026-04-11 | Pin `torch`, `alpaca-py` in optional extras |
| FB-AUDIT-14 | ops | 2026-04-11 | Rollback bundle runbook |
| FB-B1 | risk | 2026-04-10 | `available_cash_usd` + replay |
| FB-C1 | control_plane | 2026-04-10 | Streamlit + API helpers |
| FB-F1 | data | 2026-04-08 | FinBERT optional + sentiment aggregates |
| FB-F2 | data | 2026-04-10 | RSS + dedup; `NM_NEWS_RSS_FEEDS` |
| FB-F3 | data | 2026-04-10 | Multi-symbol Coinbase REST test |
| FB-F4 | data | 2026-04-10 | Strict `BarEvent` OHLC |
| FB-FR-P0 | models | 2026-04-11 | NumPy forecaster + ensemble + toy torch trainer *(full graph = §2 sub-items)* |
| FB-FR-P1 | models | 2026-04-11 | Walk-forward, pinball, JSON checkpoints |
| FB-FR-PG1 | models | 2026-04-11 | Routing `ridge`/`packet` *(historical)* |
| FB-FR-PG2 | models | 2026-04-11 | `train_forecaster_stub` + `train_forecaster_torch` |
| FB-FR-PG3 | models | 2026-04-11 | Conformal save/load + `build_forecast_packet_methodology` |
| FB-FR-PG4 | models | 2026-04-11 | `DatasetManifest` |
| FB-FR-PG5 | models | 2026-04-11 | `build_ensemble_forecast_packet`, `forward_ensemble_numpy` |
| FB-FR-PG6 | models | 2026-04-11 | `safe_build_forecast_packet`, forecaster metrics |
| FB-FR-PG7 | models | 2026-04-11 | `ABLATION_SIGNED_OFF_LSTM_BASELINE` |
| FB-FR-PG8 | models | 2026-04-11 | Drift z-score + `test_forecast_guard_drift` |
| FB-FR-PG9 | models | 2026-04-11 | `test_fault_injection_forecast` |
| FB-HS-01 | specs | 2026-04-10 | Human specs + stubs + tests |
| FB-HS-02 | specs | 2026-04-10 | `FORECASTER_AND_POLICY.MD` |
| FB-HS-03 | specs | 2026-04-11 | Methodology wired in code |
| FB-HS-04 | specs | 2026-04-10 | Literal package layout |
| FB-I1 | docs | 2026-04-10 | README orchestration note |
| FB-I2 | ci | 2026-04-10 | GitHub Actions + optional integration |
| FB-I3 | ops | 2026-04-10 | Runbooks (with HG-12) |
| FB-M1 | memory | 2026-04-08 | `feature_dict_to_embedding` |
| FB-M2 | memory | 2026-04-08 | Per-symbol memory loop |
| FB-M3 | memory | 2026-04-10 | Qdrant `schema_version` + integration test |
| FB-ML1 | models | 2026-04-08 | HMM joblib *(historical)* |
| FB-ML2 | models | 2026-04-08 | Ridge surrogate *(historical)* |
| FB-ML3 | models | 2026-04-08 | MLflow logging |
| FB-ML4 | models | 2026-04-08 | Prefect flow |
| FB-O1 | observability | 2026-04-08 | `DECISION_LATENCY` |
| FB-O2 | observability | 2026-04-08 | Promtail + Grafana provisioning |
| FB-PL-P0 | models | 2026-04-11 | `run_stub_episode`; **FB-PL-P0-01…04** closed 2026-04-11 |
| FB-PL-P0-01 | models | 2026-04-11 | `@runtime_checkable` `TradingPolicyEnvironment` + `assert_trading_policy_environment` |
| FB-PL-P0-02 | models | 2026-04-11 | `ReplayBuffer` + `ActorCriticTrainer` coverage in `tests/test_policy_pl_p0.py` |
| FB-PL-P0-03 | models | 2026-04-11 | `OfflineForecastPacketEnvironment` — precomputed packets + aligned mids |
| FB-PL-P0-04 | models | 2026-04-11 | `one_step_reward` λ-term regression in `tests/test_policy_pl_p0.py` |
| FB-PL-P1 | models | 2026-04-11 | Forecast packet enabled + `last_forecast_packet` *(historical flag)* |
| FB-PL-P2 | models | 2026-04-11 | `policy_envelope_from_app_settings` |
| FB-PL-P2b | models | 2026-04-10 | `risk_state_to_policy_envelope` |
| FB-PL-PG1 | models | 2026-04-11 | `ReplayPolicyEnvironment` |
| FB-PL-PG2 | models | 2026-04-11 | Replay buffer, actor–critic trainer, walk-forward slices |
| FB-PL-PG3 | models | 2026-04-11 | `shadow_compare_actions` + metric |
| FB-PL-PG4 | models | 2026-04-11 | `reward_eval.py` |
| FB-PL-PG5 | models | 2026-04-11 | `test_execution_parity` |
| FB-PL-PG6 | models | 2026-04-11 | `run_training_campaign` + `nightly_retrain` |
| FB-PL-PG7 | models | 2026-04-11 | `POST /models/version` + Grafana panels |
| FB-PL-PG8 | models | 2026-04-11 | `behavior_cloning.py` |
| FB-PL-PG9 | models | 2026-04-11 | Runbook model paths *(historical routing)* |
| FB-S1 | observability | 2026-04-10 | QuestDB batch + runbooks |
| FB-SPEC-00 | pipeline | 2026-04-11 | Spec pipeline wiring *(legacy mode since removed)* |
| FB-SPEC-01 | pipeline | 2026-04-11 | Default `spec_policy` *(historical)* |
| FB-SPEC-02 | pipeline | 2026-04-11 | NPZ forecaster + policy weights on hot path |
| FB-SPEC-03 | pipeline | 2026-04-11 | `model_artifact_contract` + `GET /status` |
| FB-SPEC-04 | pipeline | 2026-04-11 | Real-candle training campaign |
| FB-SPEC-05 | pipeline | 2026-04-11 | PyTorch `resolve_torch_device` |
| FB-SPEC-06 | pipeline | 2026-04-11 | Filesystem `active_model_set.json` + drift |
| FB-SPEC-07 | pipeline | 2026-04-11 | `MULTI_ASSET_PORTFOLIO.MD` |
| FB-SPEC-08 | pipeline | 2026-04-11 | Preflight script + API |
| FB-T1 | test | 2026-04-10 | `smoke_credentials.py` |
| FB-T2 | runtime | 2026-04-10 | Shutdown signals test |
| FB-X1 | execution | 2026-04-08 | Coinbase live JWT orders |
| FB-X2 | execution | 2026-04-08 | Cancel + positions |
| FB-X3 | execution | 2026-04-10 | `create_execution_adapter` in router |
| HG-12 | ops | 2026-04-10 | Runbook pointer (with FB-I3) |
| HG-13 | ci | 2026-04-10 | CI workflow (with FB-I2) |

---

*Priority mapping: HIGH = P0/P1, MEDIUM = P2, LOW = P3.*
