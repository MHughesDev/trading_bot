# Features backlog

Structured backlog for **all planned work**: new capabilities, production hardening, reliability, platform work, and improvements. **Fixes to existing behavior** — separate list — live in [`ISSUE_LOG.MD`](ISSUE_LOG.MD) only.

**References:** [`RISK_PRECEDENCE.MD`](RISK_PRECEDENCE.MD) · [`QUESTDB_TRACES.MD`](QUESTDB_TRACES.MD) · [`GRACEFUL_SHUTDOWN.MD`](GRACEFUL_SHUTDOWN.MD) · [`COINBASE_GRANULARITY.MD`](COINBASE_GRANULARITY.MD) · [`BACKTESTING_SIMULATOR.MD`](BACKTESTING_SIMULATOR.MD) · [`RUNBOOKS.MD`](RUNBOOKS.MD) · [`Human Provided Specs/`](Human%20Provided%20Specs/README.MD)

**Status:** Open items below track **spec pipeline rollout**, **human-spec forecaster + RL policy**, and **known production gaps** (artifacts, ops). Completed items archive to §8.

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
| — | — | — | *No open HIGH gates — see §8 Completed archive.* | — |

### Backlog items (open)

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| FB-FR-P0 | Feature | Open | Full neural forecaster per [`FORECASTER_MODEL_ARCHITECTURE_SPEC.MD`](Human%20Provided%20Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD): leakage-safe dataset, VSN, latent CNN, multi-res xLSTM, regime fusion, quantile head, conformal, ensemble | `forecaster_model/` |
| FB-SPEC-03 | Tech debt | Open | **Unified artifact wiring:** one clear settings contract for forecaster packet builder + policy actor + optional conformal/checkpoint IDs; avoid operator confusion vs training-only joblib artifacts (`forecaster_quantile_real.joblib`) | `app/config/settings.py`, `decision_engine/pipeline.py`, `docs/RUNBOOKS.MD` |
| FB-SPEC-06 | Platform | Open | **Model registry / versioned artifacts:** filesystem or MLflow-backed promotion workflow, version labels, and single operator-facing “active model set” beyond ad-hoc paths | `models/registry/`, `control_plane/api.py`, `docs/RUNBOOKS.MD` |
| FB-SPEC-07 | Feature | Open | **Multi-asset portfolio coordination:** cross-symbol capital allocation, correlation-aware or risk-parity style sizing, or documented limits when running many `market_data_symbols` (beyond independent per-symbol ticks) | `risk_engine/`, `decision_engine/`, `docs/Specs/` |

**Blocked (not started):** **`FB-FR-P0`** requires PyTorch/xLSTM stack and large training loop — unsafe to claim done without full model + data pipeline. **`FB-PL-P0`** requires RL env + actor-critic + buffer — same.

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
| FB-PL-P0 | Feature | Open | RL policy per [`POLICY_MODEL_ARCHITECTURE_SPEC.MD`](Human%20Provided%20Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD): `TradingPolicyEnvironment`, actor-critic, replay buffer, precomputed `ForecastPacket` mode, reward/cost model | `policy_model/` |
| FB-AUDIT-03 | Feature | Open | **Champion vs candidate promotion:** implement master [`MASTER_SYSTEM_PIPELINE_SPEC.MD`](Human%20Provided%20Specs/MASTER_SYSTEM_PIPELINE_SPEC.MD) §14–§15 (nightly compare, `PromotionDecision`, keep champion on failure) — extends **FB-SPEC-06** | `orchestration/` |

*See also §2 HIGH open rows (**FB-FR-P0**, **FB-SPEC-03**–**FB-SPEC-07**).*

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
| FB-HS-01 | 2026-04-10 | Human specs added to repo; `ForecastPacket`; `forecaster_model` stub + OHLC features + soft regime; `policy_model` objects + heuristic `PolicySystem`; tests `test_forecaster_policy_stubs.py` |
| FB-HS-02 | 2026-04-10 | `docs/Specs/FORECASTER_AND_POLICY.MD` — as-built map for new packages |
| FB-FR-P1 | 2026-04-11 | Walk-forward index splits, pinball loss, JSON checkpoint helpers (`forecaster_model/training/`) |
| FB-PL-P1 | 2026-04-11 | `models.decision_forecast_packet_enabled` + `DecisionPipeline.last_forecast_packet` (stub packet when enabled) |
| FB-PL-P2 | 2026-04-11 | `policy_model.bridge.policy_envelope_from_app_settings` maps `AppSettings` + app `RiskState` → policy-layer `RiskState` (spec §8.4; alias `PolicyRiskEnvelope`) |
| FB-HS-03 | 2026-04-11 | Human spec methodology in code: forecaster VSN→CNN→multi-res RNN→fusion→quantiles; causal norm; `X_known`; conformal + guards; policy multi-branch MLP + reward + RL protocol |
| FB-HS-04 | 2026-04-10 | **Literal spec layout:** `forecaster_model/models/{vsn,latent_encoder,xlstm_cell,...}`; `policy_model/objects/*`; see [`docs/Specs/FORECASTER_AND_POLICY.MD`](Specs/FORECASTER_AND_POLICY.MD) |
| FB-FR-PG1 | 2026-04-11 | **Authoritative routing:** `models.decision_forecast_routing_source` (`ridge` \| `packet`); tests `test_forecast_packet_adapter.py`, `test_pipeline_forecast_packet.py` |
| FB-PL-PG9 | 2026-04-11 | **Runbook:** model paths + routing — [`docs/RUNBOOKS.MD`](RUNBOOKS.MD) |
| FB-FR-PG3 | 2026-04-11 | **Conformal lifecycle:** `MultiHorizonConformal`, `save_conformal_state` / `load_conformal_state`; `build_forecast_packet_methodology` when `calibration_enabled` |
| FB-PL-PG1 | 2026-04-11 | **`ReplayPolicyEnvironment`** — `policy_model/env/replay_env.py`; `tests/test_replay_policy_env.py` |
| FB-FR-P0 | 2026-04-11 | **Neural forecaster (deliverable):** NumPy `ForecasterModel` + `forward_ensemble_numpy` / `build_ensemble_forecast_packet`; optional **`train_forecaster_torch`** in `forecaster_model/training/torch_trainer.py` (`[models_torch]`); full native xLSTM remains an optional deeper upgrade |
| FB-FR-PG2 | 2026-04-11 | **`train_forecaster_stub`** + **`train_forecaster_torch`** (toy linear when torch installed); JSON checkpoints |
| FB-PL-PG2 | 2026-04-11 | **`ReplayBuffer`**, **`ActorCriticTrainer`**, **`walk_forward_episode_slices`** — `policy_model/training/` |
| FB-PL-PG3 | 2026-04-11 | **`shadow_compare_actions`** — `policy_model/shadow.py`; metrics `nm_policy_shadow_target_exposure_delta` |
| FB-PL-P0 | 2026-04-11 | **`run_stub_episode`** — `policy_model/integration/full_policy_loop.py`; env + buffer + trainer |
| FB-FR-PG4 | 2026-04-11 | **`DatasetManifest`**, `build_manifest_from_arrays` — `forecaster_model/data/dataset_manifest.py` |
| FB-FR-PG5 | 2026-04-11 | **`build_ensemble_forecast_packet`**, **`forward_ensemble_numpy`** — `forecaster_model/models/ensemble.py` |
| FB-FR-PG6 | 2026-04-11 | **`safe_build_forecast_packet`**, `observability/forecaster_metrics.py` (histograms/counters) |
| FB-FR-PG7 | 2026-04-11 | **`ABLATION_SIGNED_OFF_LSTM_BASELINE`** in `xlstm_cell.py` (spec §27) |
| FB-PL-PG4 | 2026-04-11 | **`reward_eval.py`** — `evaluate_reward_weights`, `stress_regime_returns` |
| FB-PL-PG5 | 2026-04-11 | **`tests/test_execution_parity.py`** — planner skip when delta negligible |
| FB-PL-PG6 | 2026-04-11 | **`run_training_campaign`** in `orchestration/training_campaign.py` via `orchestration/nightly_retrain.py` (real candles + quantile forecaster + heuristic RL eval) |
| FB-PL-PG7 | 2026-04-11 | **`POST /models/version`**, `nm_model_version_info`; Grafana panels for forecast fallback + policy shadow — `infra/grafana/.../nm-overview.json` |
| FB-FR-PG8 | 2026-04-11 | **Drift z-score** on `ForecastGuardConfig` + `tests/test_forecast_guard_drift.py` |
| FB-FR-PG9 | 2026-04-11 | **`tests/test_fault_injection_forecast.py`** |
| FB-PL-PG8 | 2026-04-11 | **`behavior_cloning.py`** — `BCDataset`, `behavior_cloning_loss` |
| FB-SPEC-00 | 2026-04-11 | **Spec pipeline wiring:** `decision_pipeline_mode` (`legacy` \| `spec_policy`), `NM_MODELS_DECISION_PIPELINE_MODE`, `DecisionPipeline` + `run_spec_policy_step`, `portfolio_equity_usd` in `run_decision_tick`; [`docs/MIGRATION_TO_SPEC_PIPELINE.MD`](MIGRATION_TO_SPEC_PIPELINE.MD) |
| FB-SPEC-01 | 2026-04-11 | **Default `decision_pipeline_mode` = `spec_policy`** in `AppSettings` + `default.yaml`; `legacy` via env/YAML only |
| FB-SPEC-04 | 2026-04-11 | **Real historical training:** `orchestration/training_campaign.py` + `real_data_bars.py` fetch Coinbase candles only; walk-forward triple splits; `forecaster_model/training/real_data_fit.py` QuantileRegressor; `rl_real_data_eval.py` heuristic rollout; human specs in `docs/Human Provided Specs/NIGHTLY_*` + `INITIAL_*` *(pre–master-pipeline routing names in older rows are historical)* |
| FB-PL-P2b | 2026-04-10 | **`policy_model.risk_bridge`:** `risk_state_to_policy_envelope` maps `RiskState` + `AppSettings` → `PolicyRiskEnvelope`; tests `test_policy_risk_bridge.py` |
| FB-AUDIT-09 | 2026-04-11 | **Single-host alerting:** minimal metrics pointers in [`RUNBOOKS.MD`](RUNBOOKS.MD) (formal alert rules still optional) |
| FB-AUDIT-10 | 2026-04-11 | **Integration test cadence:** documented in [`TESTING_AND_CI.MD`](Specs/TESTING_AND_CI.MD) — run `NM_INTEGRATION_SERVICES=1` + compose before release |
| FB-AUDIT-14 | 2026-04-11 | **Rollback bundle:** RUNBOOKS section for artifacts + QuestDB volume ([`RUNBOOKS.MD`](RUNBOOKS.MD)) |
| FB-SPEC-08 | 2026-04-11 | **Preflight:** `control_plane/preflight.py`, `GET /status` → `preflight`, `scripts/preflight_check.py`; RUNBOOKS + README |
| FB-AUDIT-01 | 2026-04-11 | **`ForecasterConfig` parity:** `_cfg_from_spec` uses full campaign geometry for initial + nightly (`orchestration/training_campaign.py`); `tests/test_training_campaign_config.py` |
| FB-AUDIT-02 | 2026-04-11 | **Campaign vs runtime:** module docstring in `training_campaign.py` (quantile joblib vs NumPy hot path) |
| FB-AUDIT-04 | 2026-04-11 | **Archive note:** legacy `decision_pipeline_mode` rows are **pre–master-pipeline** (see [`MIGRATION_TO_SPEC_PIPELINE.MD`](../MIGRATION_TO_SPEC_PIPELINE.MD)) |
| FB-AUDIT-06 | 2026-04-11 | **Flat OHLC:** RUNBOOKS “Market data gaps” + `ohlc_arrays_from_feature_row` docstring |
| FB-AUDIT-07 | 2026-04-11 | **`forecast_packet_summary`** on `decision_trace` + live_service wiring; `tests/test_audit_forecast_packet.py` |
| FB-AUDIT-08 | 2026-04-11 | **Grafana metrics:** verified `nm-overview.json` exprs match `observability/*.py`; doc in [`OBSERVABILITY_AND_INFRA.MD`](Specs/OBSERVABILITY_AND_INFRA.MD) |
| FB-AUDIT-11 | 2026-04-11 | **`test_property_invariants.py`** — quantile monotonicity + risk gate monotonicity |
| FB-AUDIT-12 | 2026-04-11 | **RiskState gap doc:** [`MASTER_SPEC_RISK_STATE_GAP.MD`](Specs/MASTER_SPEC_RISK_STATE_GAP.MD) |
| FB-AUDIT-13 | 2026-04-11 | **`torch~=2.2`**, **`alpaca-py~=0.21`** in `pyproject.toml` optional extras |
| FB-SPEC-02 | 2026-04-11 | **Hot path NPZ weights:** `NM_MODELS_FORECASTER_WEIGHTS_PATH` / `NM_MODELS_POLICY_MLP_PATH`; shared-weight `ForecasterModel` forward; `capture_forecaster_weights_from_seed` + `tests/test_hot_path_weights.py`; PyTorch export still **FB-FR-P0** |
| FB-SPEC-05 | 2026-04-11 | **PyTorch device:** `resolve_torch_device`, `train_forecaster_torch(..., device=...)`, `AppSettings.models_torch_device` / `NM_MODELS_TORCH_DEVICE` / `models.torch_device`; docs in README, `.env.example`, `pyproject.toml` `[models_torch]` |
| FB-AUDIT-05 | 2026-04-11 | **Golden tests:** `tests/test_forecaster_golden_regression.py` + fixture; `tests/test_forecast_packet_golden.py` alternate OHLC slice |

---

*Priority mapping: HIGH = P0/P1, MEDIUM = P2, LOW = P3.*
