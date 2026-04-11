# Issue log

**Purpose:** A single place for **things we need to fix** on **existing** functions and components — bugs, incorrect behavior, stubs that break contracts, missing wiring in code paths that already exist, and operational/debt items that are **fixes**, not new capabilities.

**Not here:** new features, greenfield work, or release gates — use **[`FEATURES_BACKLOG.MD`](FEATURES_BACKLOG.MD)** only. No duplicate line items between this file and the backlog.

---

## Contents

| § | Section |
|---|---------|
| [§1](#1-priority-levels) | Priority levels |
| [§1.5](#15-phases--categories-for-issues) | **Phases & categories** (open + resolved) |
| [§2](#2-conventions) | Conventions (ID, area, status) |
| [§3](#3-open-issues-by-priority) | Open issues (HIGH/MEDIUM/LOW) |
| [§4](#4-syncing-to-github-issues-optional) | GitHub sync |
| [§5](#5-resolved-recent) | Resolved (recent), by area / category |
| [§6](#6-related) | Related docs |

---

## 1. Priority levels

| Level | Meaning |
|-------|---------|
| **HIGH** | Blocks or severely misrepresents production behavior; safety, correctness, or core pipeline integrity. |
| **MEDIUM** | Important gaps in observability, ops, UX, or data quality that do not block basic paper/sandbox use. |
| **LOW** | Test coverage, polish, or minor verification gaps. |

---

## 1.5 Phases & categories (for issues)

**Phases** (fix urgency — use on **open** rows in §3; optional on resolved rows for analytics):

| Phase | Maps to priority | Meaning |
|-------|------------------|---------|
| **P1** | HIGH | Correctness, safety, or pipeline integrity — fix before trusting production paths. |
| **P2** | MEDIUM | Important gaps in observability, ops, or data quality for sandbox/paper. |
| **P3** | LOW | Tests, polish, minor verification. |

**Categories** (subsystem — stable codes; aligns with **Area**):

| Cat | Area | Typical paths |
|-----|------|----------------|
| **EX** | `execution` | `execution/adapters/`, venues |
| **RK** | `risk` | `risk_engine/`, signing |
| **DP** | `data` | `data_plane/ingest/`, WS normalizers, memory |
| **MD** | `models` | `forecaster_model/`, `models/`, serving |
| **RT** | `runtime` | `app/runtime/`, live loop, signals |
| **BT** | `backtest` | `backtesting/` |
| **CP** | `control_plane` | `control_plane/`, FastAPI, Streamlit |
| **OB** | `observability` | metrics, QuestDB traces, Grafana/Loki |
| **OP** | `ops` | orchestration, training drivers, runbooks |
| **CI** | `ci` | workflows, integration test harness |
| **CFG** | *(contracts/config)* | `.env.example`, `app/contracts/`, mapping constants |

*Resolved issues:* §5 subsections are labeled with **(Cat)**; **[§5.0](#50-resolved-index-by-category)** lists all IDs by category.

---

## 2. Conventions

| Field | Meaning |
|-------|---------|
| **ID** | `IL-###` (local); **IL-1xx** reserved for data/ingest/memory cluster |
| **Sub-IDs** | For one logical incident split across PRs, use **`IL-###-01`**, **`IL-###-02`** in the **Summary** (same §3 row until all slices ship, or separate rows if easier to close independently). Prefer **separate `IL-###` rows** when fixes are unrelated. |
| **Area** | `data`, `models`, `execution`, `risk`, `runtime`, `backtest`, `control_plane`, `observability`, `ops`, `ci` — each maps to a **Cat** code in [§1.5](#15-phases--categories-for-issues). |
| **Cat** | Category code (**EX**, **DP**, …) — set from Area; use **CFG** when the fix is primarily config/contracts. |
| **Phase** | **P1** / **P2** / **P3** — same meaning as in [§1.5](#15-phases--categories-for-issues). |
| **Status** | Open / In progress / Done |

When you close an item: set **Status** to **Done**, add **Resolved** date + short note in **§5** under the right **Area** subsection (or **CFG** for config/contract fixes), update **[§5.0](#50-resolved-index-by-category)** if you maintain it, or remove the row from open tables.

**Granularity rule:** one **open** row should describe a **single verifiable fix** (e.g. “return 422 on invalid payload” not “improve API”). If a fix needs design + code + docs, either one row with **checklist sub-bullets** in the summary or **IL-###-01 / -02** sub-IDs.

---

## 3. Open issues (by priority)

*Template columns: **Phase** = P1|P2|P3, **Cat** = category code, **Area** = machine area string.*

### HIGH priority (open) — Phase P1

| ID | Phase | Cat | Area | Summary |
|----|-------|-----|------|---------|
| — | — | — | — | *No open HIGH items — see §5.* |

### MEDIUM priority (open) — Phase P2

| ID | Phase | Cat | Area | Summary |
|----|-------|-----|------|---------|
| — | — | — | — | *No open MEDIUM items — see §5.* |

### LOW priority (open) — Phase P3

| ID | Phase | Cat | Area | Summary |
|----|-------|-----|------|---------|
| — | — | — | — | *No open LOW items — see §5.* |

---

## 4. Syncing to GitHub Issues (optional)

Embedded issue titles and bodies for `gh issue create` live in **`scripts/create_github_issues.sh`**. To push them to GitHub:

1. Install and auth: `gh auth login` (repo scope).
2. From repo root: `bash scripts/create_github_issues.sh`  
   Or: `REPO=owner/repo bash scripts/create_github_issues.sh`
3. Edit the script if titles/bodies drift from this file; prefer **updating this file** for operational fixes, then mirroring to the script if you still use bulk create.

**Note:** GitHub issues are optional; this markdown file remains the **local** source of truth for fix tracking.

---

## 5. Resolved (recent)

*Grouped by **Area** (with **Cat** code). Within each subsection, roughly **newest last** (append new resolutions at the bottom of the subsection).*

### 5.0 Resolved index by category

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

### 5.1 execution **(EX)**

| ID | Phase | Resolved | Summary |
|----|-------|----------|---------|
| IL-001 | P1 | 2026-04-08 | Coinbase live: CDP JWT REST for `submit_order` / `cancel_order` / `fetch_positions` via `execution/coinbase_advanced_http.py` (no synthetic `pending_implementation` ack). |

### 5.2 models **(MD)**

| ID | Phase | Resolved | Summary |
|----|-------|----------|---------|
| IL-002 | P3 | 2026-04-08 | *(Superseded 2026-04-11.)* Historical: HMM regime + `NM_MODELS_REGIME_PATH` — **removed** with master pipeline; regime now derived from `ForecastPacket` soft vector. |
| IL-003 | P3 | 2026-04-08 | *(Superseded 2026-04-11.)* Historical: Ridge `TemporalFusionForecaster` + `NM_MODELS_FORECAST_PATH` — **removed**; forecaster is `forecaster_model` / `ForecastPacket` path only. |
| IL-103 | P2 | 2026-04-11 | **Serving mode:** README + RUNBOOKS; `DecisionPipeline` logs `decision pipeline serving mode:` once per process (`decision_engine/pipeline.py`). |
| IL-106 | P3 | 2026-04-11 | **Doc hygiene:** IL-002 / IL-003 summaries rewritten (superseded-by-master-pipeline note). |
| IL-107 | P2 | 2026-04-11 | **Conformal load:** WARNING log when `conformal_state_path` set but `load_conformal_state` fails (`forecaster_model/inference/build_from_ohlc.py`). |

### 5.3 data **(DP)**

| ID | Phase | Resolved | Summary |
|----|-------|----------|---------|
| IL-004 | P2 | 2026-04-08 | Memory: deterministic `feature_dict_to_embedding` for Qdrant queries; per-symbol 60s loop; payload `schema_version: 1`. |
| IL-005 | P2 | 2026-04-10 | Sentiment: RSS/Atom fetch + dedup (`data_plane/ingest/rss_news.py`, `news_ingest.py`); `aggregate_sentiment_for_symbols_async` wired in `live_service`; optional FinBERT unchanged. |
| IL-101 | P2 | 2026-04-10 | `news_ingest`: live RSS pipeline + dedup (see IL-005). |
| IL-102 | P3 | 2026-04-10 | WS normalizer fixtures expanded: `trade_tick.json`, `l2_snapshot.json` + tests in `test_normalizer_fixtures.py`. |

### 5.4 observability **(OB)**

| ID | Phase | Resolved | Summary |
|----|-------|----------|---------|
| IL-006 | P2 | 2026-04-10 | QuestDB: batched `decision_traces` writes (`questdb_batch_max_rows`, periodic `flush_interval_seconds`); runbooks in `docs/RUNBOOKS.MD`. |
| IL-007 | P2 | 2026-04-10 | Observability: Promtail + Grafana provisioning already in compose; Streamlit links to Loki/Grafana URLs (`control_plane/streamlit_util.py`). |

### 5.5 control_plane **(CP)**

| ID | Phase | Resolved | Summary |
|----|-------|----------|---------|
| IL-009 | P2 | 2026-04-10 | Streamlit: shared API helpers + Home/Live/Routes/Models/Emergency/Logs/Regimes bound to control plane or observability URLs. |
| IL-105 | P2 | 2026-04-11 | **Preflight:** `preflight_report` + `GET /status` → **`preflight`** (adapter names, signing policy), **`production_preflight`** (signing + venue keys vs mode); `scripts/preflight_check.py`; RUNBOOKS + README — complements **FB-SPEC-08** |
| IL-110 | P2 | 2026-04-11 | **Bind address:** RUNBOOKS section for `NM_CONTROL_PLANE_HOST` / API key on untrusted LAN ([`RUNBOOKS.MD`](RUNBOOKS.MD)). |

### 5.6 ci **(CI)**

| ID | Phase | Resolved | Summary |
|----|-------|----------|---------|
| IL-010 | P2 | 2026-04-10 | CI: `.github/workflows/ci.yml` (ruff, pytest, spec scripts); optional integration workflow + `tests/test_integration_optional_services.py`. |

### 5.7 ops **(OP)**

| ID | Phase | Resolved | Summary |
|----|-------|----------|---------|
| IL-008 | P2 | 2026-04-08 | Orchestration: `run_train_evaluate_log` + optional Prefect `@flow` in `orchestration/nightly_retrain.py`; MLflow params/metrics (no auto-promotion). |
| IL-104 | P1 | 2026-04-11 | **Training pipeline:** removed `_synthetic_training_data` / Ridge-only `run_train_evaluate_log`; `orchestration/nightly_retrain.py` drives **real Coinbase candle** fetch + walk-forward quantile forecaster + real-data heuristic RL eval (`orchestration/training_campaign.py`). |

### 5.8 runtime **(RT)**

| ID | Phase | Resolved | Summary |
|----|-------|----------|---------|
| IL-011 | P2 | 2026-04-10 | `NotImplementedError` in `main()` signal handlers: extracted `register_shutdown_signals`; test `tests/test_live_service_signals.py`. |

### 5.9 config / contracts **(CFG)**

| ID | Phase | Resolved | Summary |
|----|-------|----------|---------|
| IL-108 | P2 | 2026-04-11 | **`.env.example`:** aligned to `NM_*` variables + pointer to `AppSettings` (`.env.example`). |
| IL-109 | P2 | 2026-04-11 | **`SPEC_POLICY_DELTA_TO_SIZE_FRACTION_SCALE`** + docstring in `decision_engine/spec_policy_proposal.py` for delta→`size_fraction` mapping. |

---

## 6. Related

- [`FEATURES_BACKLOG.MD`](FEATURES_BACKLOG.MD) — planned work; **FB-** items cover capabilities (no duplicate fix rows here). **FB-** rows may use **-01 / -02** sub-IDs for shippable slices (e.g. **FB-DASH-03-01**); **IL-** issues stay fix-shaped and should not mirror those epics line-for-line.
- [`RISK_PRECEDENCE.MD`](RISK_PRECEDENCE.MD) · [`BACKTESTING_SIMULATOR.MD`](BACKTESTING_SIMULATOR.MD)
