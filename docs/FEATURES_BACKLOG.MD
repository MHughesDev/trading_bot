# Features backlog

Structured backlog for **all planned work**: new capabilities, production hardening, reliability, docs, and spikes. **Fixes to existing behavior** — separate list — live in [`ISSUE_LOG.MD`](ISSUE_LOG.MD) only.

**References:** [`RISK_PRECEDENCE.MD`](RISK_PRECEDENCE.MD) · [`QUESTDB_TRACES.MD`](QUESTDB_TRACES.MD) · [`GRACEFUL_SHUTDOWN.MD`](GRACEFUL_SHUTDOWN.MD) · [`COINBASE_GRANULARITY.MD`](COINBASE_GRANULARITY.MD) · [`BACKTESTING_SIMULATOR.MD`](BACKTESTING_SIMULATOR.MD) · [`RUNBOOKS.MD`](RUNBOOKS.MD) · [`Human Provided Specs/`](Human%20Provided%20Specs/README.MD)

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
| FB-FR-P0 | Feature | Open | Full neural forecaster per [`FORECASTER_MODEL_ARCHITECTURE_SPEC.MD`](Human%20Provided%20Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD): leakage-safe dataset, VSN, latent CNN, multi-res xLSTM, regime fusion, quantile head, conformal, ensemble | `forecaster_model/` |
| FB-FR-PG2 | Improvement | Open | **PyTorch forecaster training stack:** trainer, validation split, early stopping, export checkpoints → MLflow or disk registry (manual promotion per CI policy); prerequisite for closing FB-FR-P0 in production | `forecaster_model/training/`, `models/` |

### Backlog items (open) — policy production readiness

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| FB-PL-PG2 | Improvement | Open | **RL training runtime:** replay buffer (or on-policy rollouts), actor–critic updates, walk-forward schedules, save/load policy weights + obs normalizer state | `policy_model/training/` |
| FB-PL-PG3 | Hardening | Open | **Policy shadow / canary:** compute and log RL policy actions (and deltas vs baseline) without sending orders until sign-off | `policy_model/`, `observability/`, optional `NM_*` flag |

**Open vs human spec:** **`FB-FR-P0`** / **`FB-PL-P0`** remain the **umbrella** end-state from the human specs. **`FB-FR-PG*`** / **`FB-PL-PG*`** decompose **production-grade** delivery (single-machine scope: no multi-tenant auth or API rate limits). **Module layout and reference forward paths** align to [`FORECASTER_MODEL_ARCHITECTURE_SPEC.MD`](Human%20Provided%20Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD) / [`POLICY_MODEL_ARCHITECTURE_SPEC.MD`](Human%20Provided%20Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD); **xLSTM** uses **LSTM** forward per forecaster spec §12.7 until **`FB-FR-PG7`** resolves native xLSTM vs signed-off substitute.

**Recommended sequencing (dependencies):** **FB-FR-PG4** + **FB-FR-PG2** (data + train) → **FB-FR-PG3** + **FB-FR-PG5** (calibration + ensemble) → **FB-PL-PG1** → **FB-PL-PG2** → **FB-PL-PG3** → **FB-PL-PG6** (nightly automation). Run **FB-FR-PG6**, **FB-PL-PG5**, **FB-PL-PG7** in parallel once a candidate model exists. (**FB-FR-PG1** done: `models.decision_forecast_routing_source` = `ridge` \| `packet`.)

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
| FB-FR-PG4 | Improvement | Open | **Frozen training dataset builder:** versioned manifests (raw bars → windows → log-return targets); leakage checks; reproducible train/val splits | `forecaster_model/data/`, `data_plane/` |
| FB-FR-PG5 | Feature | Open | **Forecast ensemble (production):** multiple trained members, aggregation, `ensemble_variance` and diagnostics end-to-end in `ForecastPacket` | `forecaster_model/models/`, `app/contracts/forecast_packet.py` |
| FB-FR-PG6 | Reliability | Open | **Inference robustness:** timeouts, NaN/Inf guards, abstention metrics; Prometheus counters for latency and fallback rate | `forecaster_model/inference/`, `observability/` |
| FB-FR-PG7 | Spike | Open | **xLSTM decision:** ship native xLSTM / third-party cell, or formal **LSTM baseline** with ablation sign-off per spec §27 | `forecaster_model/models/xlstm_cell.py` |
| FB-PL-PG4 | Improvement | Open | **Reward and transaction-cost calibration:** tune decomposition weights; replay-based evaluation; regime stress harness | `policy_model/env/reward.py`, `backtesting/` |
| FB-PL-PG5 | Hardening | Open | **Execution parity tests:** `ExecutionPlanner` + risk gate outputs vs Alpaca/Coinbase adapter constraints (min notional, step size, reduce-only) | `policy_model/execution/`, `execution/adapters/`, `tests/` |
| FB-PL-PG6 | Platform | Open | **Scheduled training jobs:** nightly or off-hours forecaster + RL train/eval (cron, systemd timer, or Prefect); write artifacts + metrics; no auto-promotion | `orchestration/`, `docs/RUNBOOKS.MD` |
| FB-PL-PG7 | Reliability | Open | **Model-aware dashboards/metrics:** version tags, calibration error proxy, target vs actual exposure drift | `observability/`, Grafana, `control_plane/` |

---

## 4. LOW priority

*P3 — optional experiments and polish.*

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| FB-FR-PG8 | Improvement | Open | **Drift monitoring (features/targets):** statistical checks vs training reference; optional auto-abstention hooks to `ForecastGuard` | `forecaster_model/inference/guards.py`, batch or streaming stats |
| FB-FR-PG9 | Hardening | Open | **Fault-injection tests:** simulated model failure mid-tick → verify fallback to surrogate and no silent trade | `tests/` |
| FB-PL-PG8 | Spike | Open | **Conservative offline RL:** behavior cloning or pessimistic value if training purely on historical replay before live shadow | `policy_model/training/` |

### Out of scope (not prioritized)

| ID | Summary |
|----|---------|
| FB-N1 | Multi-exchange execution |
| FB-N3 | Portfolio optimization (generic portfolio optimization libs) |

*Note:* **Forecast-conditioned RL** is **in scope** via [`POLICY_MODEL_ARCHITECTURE_SPEC.MD`](Human%20Provided%20Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD) (track **`FB-PL-P0`**).

---

## 5. How to add a request

1. Assign **HIGH** / **MEDIUM** / **LOW** using §Priority levels (and P0–P3 mapping).  
2. Add a row in §2, §3, or §4. Use the next **FB-** or **HG-** id.  
3. Link **code paths** or tickets.

**Forecaster / policy production tracks:** **`FB-FR-PG*`** (forecaster production-grade) and **`FB-PL-PG*`** (policy production-grade) complement **`FB-FR-P0`** and **`FB-PL-P0`** (full human-spec stacks). Close PG items incrementally; P0 may remain open until PG + architecture work complete.

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
| FB-HS-04 | 2026-04-10 | **Literal spec layout:** `forecaster_model/models/{vsn,latent_encoder,xlstm_cell,multi_resolution_xlstm,fusion,decoder,forecaster_model}.py`, `config/{schema,defaults.yaml}`, `losses/quantile.py`; `policy_model/objects/*` per §8, `RiskGate`/`PolicySystem` names per §28–29, `env/rewards.py`, `config/defaults.yaml`; see [`docs/Specs/FORECASTER_AND_POLICY.MD`](Specs/FORECASTER_AND_POLICY.MD) |
| FB-FR-PG1 | 2026-04-11 | **Authoritative routing:** `models.decision_forecast_routing_source` (`ridge` default, `packet` uses `forecast_packet_to_forecast_output`); packet built when diagnostics enabled or routing is `packet`; tests `test_forecast_packet_adapter.py`, `test_pipeline_forecast_packet.py` |
| FB-PL-PG9 | 2026-04-11 | **Runbook:** model artifact paths + `decision_forecast_routing_source` + packet diagnostics — [`docs/RUNBOOKS.MD`](RUNBOOKS.MD) § Model artifacts and forecast routing |
| FB-FR-PG3 | 2026-04-11 | **Conformal lifecycle:** `MultiHorizonConformal`, `save_conformal_state` / `load_conformal_state`; `ForecasterConfig` gains `conformal_alpha` / `conformal_window_size`; `build_forecast_packet_methodology(..., conformal_bundle=, conformal_state_path=)` when `calibration_enabled`; tests `tests/test_conformal_lifecycle.py` |
| FB-PL-PG1 | 2026-04-11 | **`ReplayPolicyEnvironment`** (`policy_model/env/replay_env.py`): `reset` / `step` over close series, `DecisionPipeline` + `ForecastPacket`, reward from `one_step_reward` + settings bps; test `tests/test_replay_policy_env.py` |

---

*Priority mapping: HIGH = P0/P1, MEDIUM = P2, LOW = P3.*
