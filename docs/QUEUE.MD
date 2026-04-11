# Queue

**Single source of truth** for work items: **fixes**, **changes**, **new features**, **hardening**, **platform**, and **completed history**.

**References:** [`RISK_PRECEDENCE.MD`](RISK_PRECEDENCE.MD) · [`QUESTDB_TRACES.MD`](QUESTDB_TRACES.MD) · [`GRACEFUL_SHUTDOWN.MD`](GRACEFUL_SHUTDOWN.MD) · [`COINBASE_GRANULARITY.MD`](COINBASE_GRANULARITY.MD) · [`BACKTESTING_SIMULATOR.MD`](BACKTESTING_SIMULATOR.MD) · [`RUNBOOKS.MD`](RUNBOOKS.MD) · [`MICROSERVICES_SPLIT_PLAN.MD`](MICROSERVICES_SPLIT_PLAN.MD) · [`Human Provided Specs/`](Human%20Provided%20Specs/README.MD)

---

## Contents

| § | Section |
|---|---------|
| [§1](#1-conventions) | Conventions (Kind, IDs, priority, phase, category) |
| [§2](#2-open-queue) | **Open** queue (roadmap + deferred) |
| [§3](#3-resolved-fixes-il) | Resolved **fixes** (`IL-*`) |
| [§4](#4-completed-compliance-gates-hg) | Completed **gates** (`HG-*`) |
| [§5](#5-completed-archive-fb--themes) | Completed **archive** (`FB-*`) |
| [§6](#6-what-is-already-in-good-shape) | Healthy areas |
| [§7](#7-github-sync-optional) | GitHub sync |
| [§8](#8-how-to-add-an-entry) | How to add an entry |

---

## 1. Conventions

### 1.1 Kind (work type)

Every row uses exactly one **Kind**.

| Kind | Meaning |
|------|---------|
| **fix** | Wrong behavior, bug, contract break, or missing wiring in **existing** code (`IL-*`). |
| **feature** | **New** user- or operator-visible capability. |
| **change** | **Improvement** to existing behavior — not a bug, not a whole new surface. |
| **hardening** | Production readiness: security, signing, guardrails. |
| **platform** | Infra, CI, compose, packaging. |
| **reliability** | Ops: runbooks, smokes, distribution notes. |
| **spike** | Time-boxed research. |
| **tech_debt** | Refactors. |
| **compliance** | Release **gates** (`HG-*`). |
| **deferred** | Out of scope for now (`FB-N*`). |

Legacy **Type** column in §2 tables maps to Kind: Feature→**feature**, Hardening→**hardening**, Platform→**platform**, Reliability→**reliability**, Spike→**spike**.

### 1.2 ID prefixes

| Prefix | Scope |
|--------|--------|
| **FB-** | Roadmap, epics, deferred |
| **IL-** | Fixes only (**fix** kind) |
| **HG-** | Compliance gates |

### 1.3 Priority, phase, category, status

**Priority:** HIGH / MEDIUM / LOW (maps to P0–P3).

**Phase (for `FB-*` open work):** **A** — core trading intelligence; **B** — operator product; **C** — distributed platform; **D** — deferred.

**Category:** **FR**, **PF**, **UI**, **DS**, **MS**, **XX** (roadmap); **EX**, **MD**, **DP**, **RT**, **CP**, **OB**, **OP**, **CI**, **CFG** (fixes / §3).

**Fix urgency (`IL-*`):** **P1** / **P2** / **P3**.

**Status:** Open / In progress / Done.

---

## 2. Open queue

*Open roadmap and deferred rows. Legacy column **Type** maps to [Kind](#11-kind-work-type) (Feature→feature, Platform→platform, …).*

### HIGH priority

*P0 / P1 — release-critical and core architecture.*

### Compliance & release gates (open)

| ID | Type | Status | Summary | Verify / pointer |
|----|------|--------|---------|------------------|
| — | — | — | *No open HIGH gates — completed gates are listed in [§4](#4-completed-compliance-gates-hg).* | — |

### Backlog items (open) — forecaster epic **FB-FR-P0**

| ID | Phase | Cat | Type | Status | Summary | Pointer |
|----|-------|-----|------|--------|---------|---------|
| FB-FR-P0-01 | A | FR | Feature | Open | **Data:** leakage-safe windows, `DatasetManifest` / walk-forward alignment with master spec | `forecaster_model/data/`, `forecaster_model/training/` |
| FB-FR-P0-02 | A | FR | Feature | Open | **Graph:** VSN → latent CNN → multi-res xLSTM → regime fusion → quantile head (PyTorch), spec geometry | `forecaster_model/models/` |
| FB-FR-P0-03 | A | FR | Feature | Open | **Train:** end-to-end PyTorch training loop, checkpoints, optional `[models_torch]` / `NM_MODELS_TORCH_DEVICE` | `forecaster_model/training/torch_trainer.py` |
| FB-FR-P0-04 | A | FR | Feature | Open | **Calibrate:** conformal + ensemble integration on trained outputs (not RNG reference only) | `forecaster_model/calibration/`, `forecaster_model/models/ensemble.py` |
| FB-FR-P0-05 | A | FR | Feature | Open | **Serve:** export path from trained checkpoint to hot path (extends NPZ reference / replaces RNG-only) | `decision_engine/pipeline.py`, `forecaster_model/inference/` |
| FB-SPEC-07 | A | PF | Feature | Open | **Multi-asset portfolio coordination** (parent — use sub-rows below) | `risk_engine/`, `decision_engine/`, [`Specs/MULTI_ASSET_PORTFOLIO.MD`](Specs/MULTI_ASSET_PORTFOLIO.MD) |
| FB-SPEC-07-01 | A | PF | Feature | Open | **Exposure limits:** enforce or document max aggregate exposure across `market_data_symbols` (per-tier caps, USD notional) | `risk_engine/` |
| FB-SPEC-07-02 | A | PF | Feature | Open | **Cross-symbol rules:** correlation-aware / risk-parity style sizing *or* explicit “independent per-symbol” contract when many symbols run | `risk_engine/`, `decision_engine/` |
| FB-SPEC-07-03 | A | PF | Hardening | Open | **Doc ↔ code alignment:** [`Specs/MULTI_ASSET_PORTFOLIO.MD`](Specs/MULTI_ASSET_PORTFOLIO.MD) matches implemented limits and operator-facing warnings | `docs/` |

*Parent epic:* full neural forecaster per [`FORECASTER_MODEL_ARCHITECTURE_SPEC.MD`](Human%20Provided%20Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD). **Close epic** when **FB-FR-P0-01 … FB-FR-P0-05** are Done or explicitly waived.

**Stretch vs delivered core:** NumPy forecaster + policy integration shipped under archive ids **`FB-FR-CORE`** / **`FB-PL-CORE`** (see [§5](#5-completed-archive-fb--themes)). Open **FB-FR-P0-01 … FB-FR-P0-05** track **full architecture-spec** depth (leakage-safe datasets, native xLSTM on hot path where applicable).

**Blocked (not started):** PyTorch/xLSTM stack + large training loop — unsafe to claim done without full model + data pipeline validation.

### MEDIUM priority

*P2 — important after initial launch.*

### Compliance & gates (open)

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| — | — | — | *No open items — see [§5](#5-completed-archive-fb--themes) Completed archive.* | — |

### Backlog items (open) — policy epic **FB-PL-P0**

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| — | — | — | *No open sub-rows — see [§5](#5-completed-archive-fb--themes).* | — |

### Backlog items (open) — operator UI / Windows

*Shorthand: **FB-UI** = launcher/shell; **FB-DASH** = dashboard features. Sub-IDs are independently closable slices.*

#### FB-UI — Launcher & desktop shell

| ID | Phase | Cat | Type | Status | Summary | Pointer |
|----|-------|-----|------|--------|---------|---------|
| FB-UI-02-01 | B | UI | Spike | Open | **Shell choice:** time-boxed comparison Electron vs Tauri vs PyInstaller-webview for loading `http://127.0.0.1:8501` | [`WINDOWS_OPERATOR_UI.MD`](WINDOWS_OPERATOR_UI.MD) |
| FB-UI-02-02 | B | UI | Platform | Open | **Desktop shell MVP:** single-window host opening the Streamlit URL (no React rewrite) | `docs/`, optional `packaging/` |

#### FB-DASH — Dashboard & operator UX

| ID | Phase | Cat | Type | Status | Summary | Pointer |
|----|-------|-----|------|--------|---------|---------|
| FB-DASH-04-01 | B | DS | Feature | Open | **Mark price source:** define Kraken mid vs venue quote precedence per mode | `data_plane/`, `execution/` |
| FB-DASH-04-02 | B | DS | Feature | Open | **Unrealized P&L:** extend position DTO + green/red in UI when avg entry + mark available | `execution/adapters/`, Streamlit |
| FB-DASH-05-01 | B | DS | Feature | Open | **Ledger design:** choose source of truth for fills (QuestDB tables vs Alpaca activity API vs hybrid) | `data_plane/storage/`, `docs/` |
| FB-DASH-05-02 | B | DS | Feature | Open | **`GET /pnl/summary?range=`** (hour, day, month, year, all) with realized/unrealized split | `control_plane/api.py` |
| FB-DASH-05-03 | B | DS | Feature | Open | **Streamlit:** timeframe selector + aggregate P&L panel | `control_plane/` |
| FB-DASH-06-01 | B | DS | Hardening | Open | **UI guardrail:** modal + optional “type LIVE” before applying live execution profile | `control_plane/` |
| FB-DASH-06-02 | B | DS | Reliability | Open | **RUNBOOKS:** live trading checklist mirrors UI guardrails | `RUNBOOKS.MD` |

#### FB-MS — Microservices extraction (remaining production split)

*Scaffold + Redis chains are in place; these rows close gaps vs [`MICROSERVICES_SPLIT_PLAN.MD`](MICROSERVICES_SPLIT_PLAN.MD) “next step”.*

| ID | Phase | Cat | Type | Status | Summary | Pointer |
|----|-------|-----|------|--------|---------|---------|
| FB-MS-01 | C | MS | Feature | Open | **Market data service:** Kraken WS (and REST fallback) in process — publish real `market.tick.normalized.v1` / heartbeats; retire scaffold-only HTTP as default path | `services/market_data_service/`, `data_plane/ingest/` |
| FB-MS-02 | C | MS | Feature | Open | **Feature service:** wire `FeaturePipeline` + rolling bars (parity with `live_service` tick path), not scaffold tick→row | `services/feature_service/`, `data_plane/features/` |
| FB-MS-03 | C | MS | Feature | Open | **Observability writer:** persist subscribed events to QuestDB (or chosen store); bounded memory as cache only | `services/observability_writer_service/`, `data_plane/storage/` |
| FB-MS-04 | C | MS | Platform | Open | **Compose live_runtime:** document + validate `NM_LIVE_SERVICE_APP_START_LOOP=true` with `.env` (Kraken + venue); health/readiness for long-running loop | `infra/docker-compose.microservices.yml`, `RUNBOOKS.MD` |
| FB-MS-05 | C | MS | Reliability | Open | **Optional smoke:** documented (or CI-optional) **real Alpaca paper** path through gateway with test keys — no secrets in repo | `tests/`, `scripts/` |
| FB-MS-06 | C | MS | Hardening | Open | **Split acceptance:** checklist item — “program §11” / production split criteria written and checked off in `MICROSERVICES_SPLIT_PLAN.MD` | `docs/MICROSERVICES_SPLIT_PLAN.MD` |

*Parent epic:* RL policy per [`POLICY_MODEL_ARCHITECTURE_SPEC.MD`](Human%20Provided%20Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD). **Sub-rows FB-PL-P0-01 … FB-PL-P0-04 Done 2026-04-11** (NumPy stubs + offline env; full PyTorch RL remains future hardening).

*See also **HIGH** priority above (**FB-FR-P0-*** and **FB-SPEC-07** / **FB-SPEC-07-01…03**). **FB-SPEC-06** is in [§5](#5-completed-archive-fb--themes).*

---

### LOW priority

*P3 — optional experiments and polish.*

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| — | — | — | *No open LOW items — see [§5](#5-completed-archive-fb--themes).* | — |

### Out of scope (not prioritized)

| ID | Kind | Phase | Cat | Pri | Status | Summary |
|----|------|-------|-----|-----|--------|---------|
| FB-N1 | deferred | D | XX | LOW | Open | Multi-exchange execution |
| FB-N3 | deferred | D | XX | LOW | Open | Portfolio optimization (generic portfolio optimization libs) |

---

## 3. Resolved fixes (`IL-*`)

*Grouped by **Area** (with **Cat** code). Within each subsection, roughly **newest last** (append new resolutions at the bottom of the subsection).*

### 3.0 Resolved index by category

| Cat | IDs |
|-----|-----|
| **EX** | IL-001 |
| **MD** | IL-002, IL-003, IL-103, IL-106, IL-107 |
| **DP** | IL-004, IL-005, IL-101, IL-102 |
| **OB** | IL-006, IL-007 |
| **CP** | IL-009, IL-105, IL-110 |
| **CI** | IL-010 |
| **OP** | IL-008, IL-104 |
| **RT** | IL-011 |
| **CFG** | IL-108, IL-109 |

### 3.1 execution **(EX)**

| ID | Kind | Phase | Resolved | Summary |
|----|------|-------|----------|---------|
| IL-001 | fix | P1 | 2026-04-08 | Coinbase live: CDP JWT REST for `submit_order` / `cancel_order` / `fetch_positions` via `execution/coinbase_advanced_http.py` (no synthetic `pending_implementation` ack). |

### 3.2 models **(MD)**

| ID | Kind | Phase | Resolved | Summary |
|----|------|-------|----------|---------|
| IL-002 | fix | P3 | 2026-04-08 | *(Superseded 2026-04-11.)* Historical: HMM regime + `NM_MODELS_REGIME_PATH` — **removed** with master pipeline; regime now derived from `ForecastPacket` soft vector. |
| IL-003 | fix | P3 | 2026-04-08 | *(Superseded 2026-04-11.)* Historical: Ridge `TemporalFusionForecaster` + `NM_MODELS_FORECAST_PATH` — **removed**; forecaster is `forecaster_model` / `ForecastPacket` path only. |
| IL-103 | fix | P2 | 2026-04-11 | **Serving mode:** README + RUNBOOKS; `DecisionPipeline` logs `decision pipeline serving mode:` once per process (`decision_engine/pipeline.py`). |
| IL-106 | fix | P3 | 2026-04-11 | **Doc hygiene:** IL-002 / IL-003 summaries rewritten (superseded-by-master-pipeline note). |
| IL-107 | fix | P2 | 2026-04-11 | **Conformal load:** WARNING log when `conformal_state_path` set but `load_conformal_state` fails (`forecaster_model/inference/build_from_ohlc.py`). |

### 3.3 data **(DP)**

| ID | Kind | Phase | Resolved | Summary |
|----|------|-------|----------|---------|
| IL-004 | fix | P2 | 2026-04-08 | Memory: deterministic `feature_dict_to_embedding` for Qdrant queries; per-symbol 60s loop; payload `schema_version: 1`. |
| IL-005 | fix | P2 | 2026-04-10 | Sentiment: RSS/Atom fetch + dedup (`data_plane/ingest/rss_news.py`, `news_ingest.py`); `aggregate_sentiment_for_symbols_async` wired in `live_service`; optional FinBERT unchanged. |
| IL-101 | fix | P2 | 2026-04-10 | `news_ingest`: live RSS pipeline + dedup (see IL-005). |
| IL-102 | fix | P3 | 2026-04-10 | WS normalizer fixtures expanded: `trade_tick.json`, `l2_snapshot.json` + tests in `test_normalizer_fixtures.py`. |

### 3.4 observability **(OB)**

| ID | Kind | Phase | Resolved | Summary |
|----|------|-------|----------|---------|
| IL-006 | fix | P2 | 2026-04-10 | QuestDB: batched `decision_traces` writes (`questdb_batch_max_rows`, periodic `flush_interval_seconds`); runbooks in `docs/RUNBOOKS.MD`. |
| IL-007 | fix | P2 | 2026-04-10 | Observability: Promtail + Grafana provisioning already in compose; Streamlit links to Loki/Grafana URLs (`control_plane/streamlit_util.py`). |

### 3.5 control_plane **(CP)**

| ID | Kind | Phase | Resolved | Summary |
|----|------|-------|----------|---------|
| IL-009 | fix | P2 | 2026-04-10 | Streamlit: shared API helpers + Home/Live/Routes/Models/Emergency/Logs/Regimes bound to control plane or observability URLs. |
| IL-105 | fix | P2 | 2026-04-11 | **Preflight:** `preflight_report` + `GET /status` → **`preflight`** (adapter names, signing policy), **`production_preflight`** (signing + venue keys vs mode); `scripts/preflight_check.py`; RUNBOOKS + README — complements **FB-SPEC-08** |
| IL-110 | fix | P2 | 2026-04-11 | **Bind address:** RUNBOOKS section for `NM_CONTROL_PLANE_HOST` / API key on untrusted LAN ([`RUNBOOKS.MD`](RUNBOOKS.MD)). |

### 3.6 ci **(CI)**

| ID | Kind | Phase | Resolved | Summary |
|----|------|-------|----------|---------|
| IL-010 | fix | P2 | 2026-04-10 | CI: `.github/workflows/ci.yml` (ruff, pytest, spec scripts); optional integration workflow + `tests/test_integration_optional_services.py`. |

### 3.7 ops **(OP)**

| ID | Kind | Phase | Resolved | Summary |
|----|------|-------|----------|---------|
| IL-008 | fix | P2 | 2026-04-08 | Orchestration: `run_train_evaluate_log` + optional Prefect `@flow` in `orchestration/nightly_retrain.py`; MLflow params/metrics (no auto-promotion). |
| IL-104 | fix | P1 | 2026-04-11 | **Training pipeline:** removed `_synthetic_training_data` / Ridge-only `run_train_evaluate_log`; `orchestration/nightly_retrain.py` drives **real Coinbase candle** fetch + walk-forward quantile forecaster + real-data heuristic RL eval (`orchestration/training_campaign.py`). |

### 3.8 runtime **(RT)**

| ID | Kind | Phase | Resolved | Summary |
|----|------|-------|----------|---------|
| IL-011 | fix | P2 | 2026-04-10 | `NotImplementedError` in `main()` signal handlers: extracted `register_shutdown_signals`; test `tests/test_live_service_signals.py`. |

### 3.9 config / contracts **(CFG)**

| ID | Kind | Phase | Resolved | Summary |
|----|------|-------|----------|---------|
| IL-108 | fix | P2 | 2026-04-11 | **`.env.example`:** aligned to `NM_*` variables + pointer to `AppSettings` (`.env.example`). |
| IL-109 | fix | P2 | 2026-04-11 | **`SPEC_POLICY_DELTA_TO_SIZE_FRACTION_SCALE`** + docstring in `decision_engine/spec_policy_proposal.py` for delta→`size_fraction` mapping. |

---

## 4. Completed compliance gates (`HG-*`, reference)

### Completed gates (reference)

*These predate the Phase column; treat as cross-cutting **HG** compliance work.*

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

## 5. Completed archive (`FB-*` & themes)

Grouped by theme for quicker lookup. *Chronological merge history is preserved in git; this table is operator-oriented.*

### 5.1 Execution, data, memory

| ID | Completed | Notes |
|----|-------------|-------|
| FB-X1, FB-X2 | 2026-04-08 | Coinbase live JWT REST orders, cancel, positions from accounts |
| FB-X3 | 2026-04-10 | `create_execution_adapter` in `execution/router.py` (used by `ExecutionService`) |
| FB-F3 | 2026-04-10 | `test_coinbase_rest_candles_multi.py` (BTC/ETH/SOL Exchange fallback path) |
| FB-M1, FB-M2 | 2026-04-08 | `feature_dict_to_embedding`; live `run_memory_retrieval_loop` per symbol |
| FB-M3 | 2026-04-10 | Qdrant `schema_version: 1` in payload; CI integration test in `test_integration_optional_services.py` |
| FB-F2 | 2026-04-10 | RSS fetch + dedup (`rss_news.py`, `news_ingest.py`); `NM_NEWS_RSS_FEEDS` via YAML/env |
| FB-F4 | 2026-04-10 | Strict `BarEvent` OHLC validation (`app/contracts/events.py`) |

### 5.2 Models, training, orchestration (summary)

| ID | Completed | Notes |
|----|-------------|-------|
| FB-ML1–ML4 | 2026-04-08 | HMM / Ridge / MLflow / Prefect orchestration *(historical paths; master pipeline superseded HMM/Ridge runtime)* |
| FB-FR-P0 | 2026-04-11 | **Partial deliverable** (superseded by §2 **FB-FR-P0-01…**): NumPy `ForecasterModel` + ensemble + optional `train_forecaster_torch` |
| FB-FR-P1 | 2026-04-11 | Walk-forward splits, pinball, JSON checkpoints (`forecaster_model/training/`) |
| FB-PL-P0 | 2026-04-11 | Epic parent: `run_stub_episode`; sub-rows **FB-PL-P0-01…04** completed 2026-04-11 — §5.5 |
| FB-PL-P0-01 … FB-PL-P0-04 | 2026-04-11 | Protocol runtime check, offline `ForecastPacket` env, buffer/trainer + reward tests — `tests/test_policy_pl_p0.py` |

### 5.3 Spec pipeline, master path, ops (summary)

| ID | Completed | Notes |
|----|-------------|-------|
| FB-SPEC-00 | 2026-04-11 | Spec pipeline wiring (`decision_pipeline_mode` — **removed** later; see [`MIGRATION_TO_SPEC_PIPELINE.MD`](MIGRATION_TO_SPEC_PIPELINE.MD)) |
| FB-SPEC-01 | 2026-04-11 | Default `spec_policy` *(historical row)* |
| FB-SPEC-02–08 | 2026-04-11 | NPZ hot path, artifact contract, torch device, registry, multi-asset doc, preflight |
| FB-SPEC-04 | 2026-04-11 | Real historical training (`training_campaign`, Kraken OHLC/trades) |
| FB-HS-01–04 | 2026-04-10 / 11 | Human specs, methodology, layout |
| FB-PL-P2b | 2026-04-10 | `policy_model.risk_bridge` |
| HG-12, FB-I3 | 2026-04-10 | Runbooks [`RUNBOOKS.MD`](RUNBOOKS.MD) |

### 5.4 UI, observability, CI, backtesting (summary)

| ID | Completed | Notes |
|----|-------------|-------|
| FB-O1, FB-O2 | 2026-04-08 | Decision latency metric; Promtail + Grafana |
| FB-S1 | 2026-04-10 | QuestDB batch flush + runbooks |
| FB-C1 | 2026-04-10 | Streamlit + `streamlit_util` |
| FB-UI-01-01, FB-UI-01-02 | 2026-04-11 | **`packaging/windows/`** — PyInstaller **`nm_operator_launcher`**, **`build_operator_bundle.bat`**; **[`WINDOWS_OPERATOR_UI.MD`](WINDOWS_OPERATOR_UI.MD)** § distribution |
| FB-DASH-02-01 … FB-DASH-02-03 | 2026-04-11 | **`GET/POST /system/execution-profile`**, `execution_profile` on **`GET /status`**; Streamlit sidebar; RUNBOOKS + README; `data/execution_profile.json` pending intent |
| FB-DASH-03-01 … FB-DASH-03-03 | 2026-04-11 | **`PortfolioPositionRow`**, **`GET /portfolio/positions`**, Streamlit **Open positions** expander + refresh |
| FB-DASH-01-01, FB-DASH-01-02 | 2026-04-11 | Operator **[`READY_TO_RUN.MD`](READY_TO_RUN.MD)** checklist; README + RUNBOOKS + `.env.example` cross-links |
| FB-I1, FB-I2, HG-13 | 2026-04-10 | README orchestration note; CI workflow + optional integration |
| FB-B1 | 2026-04-10 | `available_cash_usd` / replay cash |
| FB-T1, FB-T2 | 2026-04-10 | Alpaca smoke; shutdown signals |

### 5.5 Complete archive (one row per ID)

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
| FB-UI-01-01 | platform | 2026-04-11 | **`packaging/windows/nm_operator_launcher.spec`** + **`nm_operator_launcher.py`** + **`build_operator_bundle.bat`** — mirrors **`run.bat`** |
| FB-UI-01-02 | docs | 2026-04-11 | **[`WINDOWS_OPERATOR_UI.MD`](WINDOWS_OPERATOR_UI.MD)** — § **Windows distribution (PyInstaller)** |
| FB-DASH-02-01 | control_plane | 2026-04-11 | **`GET/POST /system/execution-profile`**, **`execution_profile`** on **`GET /status`**, patch `default.yaml` + `.env` |
| FB-DASH-02-02 | control_plane | 2026-04-11 | Streamlit **Execution mode** sidebar + Apply → POST |
| FB-DASH-02-03 | ops | 2026-04-11 | **[`RUNBOOKS.MD`](RUNBOOKS.MD)** — execution mode section (restart after change) |
| FB-DASH-03-01 | contracts | 2026-04-11 | **`app/contracts/portfolio.py`** — `PortfolioPositionRow` + `position_snapshot_to_row` |
| FB-DASH-03-02 | control_plane | 2026-04-11 | **`GET /portfolio/positions`** — `execution/portfolio_positions.fetch_portfolio_positions` |
| FB-DASH-03-03 | control_plane | 2026-04-11 | **`control_plane/positions_panel.py`** — collapsible sidebar, optional 30s refresh |
| FB-DASH-01-01 | docs | 2026-04-11 | **[`READY_TO_RUN.MD`](READY_TO_RUN.MD)** — single operator checklist (venv, `.env`, Docker, preflight, venues, `run.bat`) |
| FB-DASH-01-02 | docs | 2026-04-11 | Cross-links: [`README.md`](../README.md) quick start + doc map, [`RUNBOOKS.MD`](RUNBOOKS.MD) intro, [`.env.example`](../.env.example) header → `READY_TO_RUN.MD` |
| FB-F1 | data | 2026-04-08 | FinBERT optional + sentiment aggregates |
| FB-F2 | data | 2026-04-10 | RSS + dedup; `NM_NEWS_RSS_FEEDS` |
| FB-F3 | data | 2026-04-10 | Multi-symbol market data tests *(historical: Coinbase REST; current: Kraken)* |
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
| FB-SPEC-06 | pipeline | 2026-04-11 | **Done:** filesystem `active_model_set.json` + drift; `GET /status` → `model_artifacts`; optional `MLflowModelRegistry` params/metrics (no auto-promote); `POST /models/version` → `nm_model_version_info` (`tests/test_model_artifacts.py`). Champion/candidate compare remains **FB-AUDIT-03**. |
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

## 6. What is already in good shape

| Area | Notes |
|------|--------|
| **Decision** | Shared **`run_decision_tick`**; **`replay_decisions`** / **`replay_multi_asset_decisions`**. |
| **Execution** | **Alpaca paper** adapter; **Coinbase live** execution adapter (orders only). **Market data:** Kraken REST + WebSocket. |
| **Features** | **Feature pipeline** (RSI, MACD, ATR, ADX, VWAP, microstructure). |
| **Risk** | **Risk** + HMAC + CI guards. |
| **Control / observability** | **FastAPI** + **Prometheus**; **Compose** core services. |

---

## 7. GitHub sync (optional)

Embedded issue titles and bodies for `gh issue create` live in **`scripts/create_github_issues.sh`**. To push them to GitHub:

1. Install and auth: `gh auth login` (repo scope).
2. From repo root: `bash scripts/create_github_issues.sh`  
   Or: `REPO=owner/repo bash scripts/create_github_issues.sh`
3. Edit the script if titles/bodies drift from `QUEUE.MD`; prefer **updating `QUEUE.MD`** for operational fixes, then mirroring to the script if you still use bulk create.

**Note:** GitHub issues are optional; this markdown file remains the **local** source of truth for fix tracking.

---

## 8. How to add an entry

1. Pick a **Kind** from [§1.1](#11-kind-work-type).
2. Assign **HIGH** / **MEDIUM** / **LOW** (maps to P0–P3).  
3. Set **Phase** (**A** / **B** / **C** / **D**) and **Category** per [§1.3](#13-priority-phase-category-status).  
4. Add a row in [§2](#2-open-queue) (roadmap), [§3](#3-resolved-fixes-il) (fixes), or note completed gates in [§4](#4-completed-compliance-gates-hg). Use the next **FB-** or **HG-** id; for large epics prefer **-01, -02** sub-ids.  
5. Link **code paths** or tickets.

---
