# Queue

**Single source of truth** for work items: **fixes**, **changes**, **new features**, **hardening**, **platform**, and **completed history**.

**References:** [`RISK_PRECEDENCE.MD`](RISK_PRECEDENCE.MD) · [`QUESTDB_TRACES.MD`](QUESTDB_TRACES.MD) · [`GRACEFUL_SHUTDOWN.MD`](GRACEFUL_SHUTDOWN.MD) · [`COINBASE_GRANULARITY.MD`](COINBASE_GRANULARITY.MD) · [`BACKTESTING_SIMULATOR.MD`](BACKTESTING_SIMULATOR.MD) · [`RUNBOOKS.MD`](RUNBOOKS.MD) · [`MICROSERVICES_SPLIT_PLAN.MD`](MICROSERVICES_SPLIT_PLAN.MD) · [`BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD`](BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD) (OCI deployment brainstorm; queue **FB-CONT-P0**) · [`BRAINSTORM/BS-004_MULTI_TENANT_AUTH_ACCOUNT_UI.MD`](BRAINSTORM/BS-004_MULTI_TENANT_AUTH_ACCOUNT_UI.MD) (multi-tenant auth + Account UI; queue **FB-UX-P0**) · [`Human Provided Specs/`](Human%20Provided%20Specs/README.MD)

---

## Contents

| § | Section |
|---|---------|
| [§1](#1-conventions) | Conventions (Kind, IDs, priority, phase, category) |
| [§2](#2-open-queue) | **Open** queue (roadmap + deferred) |
| [§2.2](#22-per-asset-product-epic-fb-ap-p0) | **FB-AP-P0** — per-asset product (registry, UI, data, scheduling) |
| [§2.3](#23-container--oci-cloud-deployment-fb-cont-p0) | **FB-CONT-P0** — container / OCI cloud deployment (images, compose, CI, TLS, runbooks) |
| [§2.4](#24-multi-tenant-web-operator-ux-fb-ux-p0) | **FB-UX-P0** — multi-tenant auth, Account page, per-user PnL, dashboard UI extras |
| [§3](#3-resolved-fixes-il) | Resolved **fixes** (`IL-*`) |
| [§4](#4-completed-compliance-gates-hg) | Completed **gates** (`HG-*`) |
| [§5](#5-completed-archive-fb--themes) | Completed **archive** (`FB-*`) |
| [§6](#6-what-is-already-in-good-shape) | Healthy areas |
| [§7](#7-github-sync-optional) | GitHub sync |
| [§8](#8-how-to-add-an-entry) | How to add an entry |
| — | [`AUTOMATION_QUEUE_SLICE_PROMPT.MD`](AUTOMATION_QUEUE_SLICE_PROMPT.MD) — agent prompt: slice → tests → PR → merge → prune |
| — | Cursor skill [`.cursor/skills/add-to-queue`](../.cursor/skills/add-to-queue/SKILL.md) — **Add to Queue** workflow for agents |

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

**Batch (for `FB-AP-*` and similar epics):** Short **theme code** so multiple rows can belong to the same initiative without being duplicates. Batch is **not** a sprint calendar; it is a **grouping key** for related line items.

| Batch | Theme (FB-AP-P0) |
|-------|-------------------|
| **AP-ALL** | Epic umbrella / cross-cutting |
| **AP-PAM** | Per-asset manifest + registry + tests |
| **AP-ENF** | Enforcement guards + active watch loop |
| **AP-STA** | Per-asset state machine |
| **AP-INI** | Initialization pipeline (orchestrator + steps + register) |
| **AP-DAT** | Canonical storage, rollups, dedup, ADR |
| **AP-WS** | WebSocket → canonical bars |
| **AP-BF** | Startup gap detection + Kraken backfill |
| **AP-UNI** | Alpaca/Coinbase universe + cross-ref + search API |
| **AP-API** | Symbol-scoped HTTP (chart, fills, docs) |
| **AP-UI-D** | Dashboard |
| **AP-UI-A** | Asset Page + chrome (routes, chart, markers, buttons, menu) |
| **AP-EX** | Execution (flatten on stop, adapters) |
| **AP-RT** | Realtime push to UI |
| **AP-SCH** | Schedulers (nightly, app-as-master) |
| **AP-REM** | Removal of global power / global live-paper |

**Batch (for `FB-UX-P0` — multi-tenant web operator UX; traceability [`BS-004`](../BRAINSTORM/BS-004_MULTI_TENANT_AUTH_ACCOUNT_UI.MD)):**

| Batch | Theme |
|-------|--------|
| **UX-ALL** | Epic umbrella / cross-cutting |
| **AP-UX-AUTH** | Users, passwords, sessions, API auth dependencies |
| **AP-UX-ACC** | Sign-in / Account / gear / route protection |
| **AP-UX-DAT** | Per-user PnL, positions, registry, execution credentials |
| **AP-UX-UI** | Dashboard & Asset UI extras (preflight strip, init monitor, export, etc.) |

**Batch (for `FB-CONT-P0` — container / OCI cloud deployment):**

| Batch | Theme |
|-------|--------|
| **CONT-ALL** | Epic umbrella / optional cross-cutting (spikes, managed-service evaluation) |
| **CONT-IMG** | App OCI image (Dockerfile, `.dockerignore`, entrypoints) |
| **CONT-CMP** | Compose: app service wired to QuestDB, Redis, Qdrant, observability |
| **CONT-CI** | CI: build image, optional **hadolint** / **Trivy** (no secrets in layers) |
| **CONT-EDGE** | TLS reverse proxy (**Caddy** or **Nginx**) + optional public vs bind-local |
| **CONT-BK** | Backup / restore / DR for stateful volumes |
| **CONT-CLOUD** | Cloud deployment runbook (single VM + Compose or managed containers) |
| **CONT-DOC** | Operator docs: web + OCI north star vs Windows `.bat` path |

**Category:** **FR**, **PF**, **UI**, **DS**, **MS**, **XX** (roadmap); **EX**, **MD**, **DP**, **RT**, **CP**, **OB**, **OP**, **CI**, **CFG** (fixes / §3).

**Affected files (open-queue tables):** Pipe-separated **primary** paths or globs the implementer expects to touch; **non-exhaustive** — use for navigation, not as a complete dependency closure.

**Fix urgency (`IL-*`):** **P1** / **P2** / **P3**.

**Status:** Open / In progress / Done.

---

## 2. Open queue

*Open roadmap and deferred rows. Legacy column **Type** maps to [Kind](#11-kind-work-type) (Feature→feature, Platform→platform, …).*

**Status (2026-04-12):** open **HIGH** backlog — **§2.2 [FB-AP-P0](#22-per-asset-product-epic-fb-ap-p0)** (per-asset product: strict model separation, two-page UI, universe search, canonical bars, scheduling). **§2.4 [FB-UX-P0](#24-multi-tenant-web-operator-ux-fb-ux-p0)** — multi-tenant users, sign-in + Account page (venue keys), per-user PnL, dashboard UI extras (**BS-004**). **§2.3 [FB-CONT-P0](#23-container--oci-cloud-deployment-fb-cont-p0)** — OCI/Linux container deployment and cloud portability (**BS-001**). Still deferred: **§2.1** (**FB-N1**, **FB-N3**) and optional GitHub sync **§7**.

### HIGH priority

*P0 / P1 — release-critical and core architecture.*

### 2.2 Per-asset product epic (**FB-AP-P0**)

**Brief:** Deliver **strict per-asset model ownership** (dedicated Forecaster and RL **lifecycle and artifacts** per symbol, **no shared ambiguous model state**), an **asset initialization pipeline** separate from live loops (Kraken REST historical → validate → features → initial forecaster + RL train/init → registered ready state), **tradable-universe** sync from **Alpaca** and **Coinbase** with cross-reference and **in-app search**, **symbol-scoped** data load and APIs (no global preload of other symbols’ series), **unified canonical bar storage** for REST backfill and WS-derived updates (seconds-resolution canonical table per asset as the single source of truth; higher intervals derived or materialized for charts and training), **startup backfill** of gaps for initialized assets, **two primary pages only** — **Dashboard** (holdings list + **PnL-only** chart with timeframe dropdown; **no** global hard start/stop) and **Asset Page** (one symbol: OHLC/candlestick chart with intervals **second / minute / hour / day / week / month**, buy/sell markers, per-asset **Paper/Live** dropdown, top-right **Initialize / Start / Stop** reflecting real state, **Stop** halts active watch and **flattens position**), **application running** as master for nightly jobs (when app is off, nothing runs), **nightly forecaster retrain** for every **initialized** asset, **RL retrain only when** that asset had **trading activity** in the prior period (skip RL on no-trade days; still run forecaster), **left slide-in menu** listing **active** (watching) assets with navigation to Asset Page, **low-latency chart updates** via push (WebSocket/SSE) scoped to the selected symbol, **remove** application-wide live/paper toggle and **remove** global system power hard stop. **Enforcement:** runtime and config must **enumerate** symbol → model instance and refuse cross-symbol reuse without explicit binding.

**Table columns:** **Batch** = theme group; **Phase** = [§1.3](#13-priority-phase-category-status); **Affected files** = primary touch list.

| ID | Batch | Phase | Cat | Kind | Pri | Status | Summary | Affected files |
|----|-------|-------|-----|------|-----|--------|---------|----------------|
| FB-AP-P0 | AP-ALL | B | XX | feature | HIGH | Open | **(Epic parent)** Owns end-to-end delivery of the per-asset operator product described in the **Brief** above: registry and enforcement for one Forecaster and one RL stack per traded asset; initialization pipeline; universe discovery; canonical historical storage and backfill; two-page UI; per-asset execution mode and lifecycle buttons; scheduling rules; removal of legacy global controls. Sub-rows **FB-AP-001**… are independently closable slices; implement in batch order where dependencies exist (registry before runtime wiring, storage contract before UI chart queries). | `docs/QUEUE.MD`, `app/`, `control_plane/`, `data_plane/`, `decision_engine/`, `execution/`, `forecaster_model/`, `models/`, `orchestration/`, `risk_engine/`, `tests/` |
| FB-AP-001 | AP-PAM | B | PF | feature | HIGH | Done | **Per-asset model manifest schema:** Define a versioned contract (JSON or DB row) that binds **exactly one canonical symbol** to **forecaster artifact paths** (weights, conformal, config), **RL/policy artifact paths**, optional **runtime instance id**, and **last trained** metadata. No global single `NM_MODELS_*` path may remain the only source of truth for multi-symbol operation; manifest must be the operator-visible enumeration of which model files belong to which asset. Include validation on load: symbol in manifest must match request symbol. **Done 2026-04-12:** `app/contracts/asset_model_manifest.py` (`AssetModelManifest`). | `app/contracts/asset_model_manifest.py`, `docs/PER_ASSET_OPERATOR.MD` |
| FB-AP-002 | AP-PAM | B | PF | feature | HIGH | Done | **Registry persistence and CRUD:** Implement durable storage for the per-asset manifest (filesystem with atomic writes or SQLite/QuestDB table — pick one in design). Support create/update on successful initialization, read on runtime startup, and listing for UI (all initialized assets). Must survive process restarts and support concurrent read during trading. **Done 2026-04-12:** `app/runtime/asset_model_registry.py` (JSON under `data/asset_model_registry/manifests/`, override `NM_ASSET_MODEL_REGISTRY_DIR`); **`GET/PUT/DELETE /assets/models/...`**, **`GET /assets/models`**; **`GET /status` → `asset_model_registry`**; tests `tests/test_asset_model_registry.py`, `tests/test_asset_model_manifest_api.py`. | `app/runtime/asset_model_registry.py`, `control_plane/api.py`, `tests/` |
| FB-AP-003 | AP-ENF | B | FR | hardening | HIGH | Done | **Runtime guard — Forecaster binding:** In `DecisionPipeline` (or equivalent), loading a forecaster for a tick must assert `manifest.symbol == decision_symbol`; refuse or abstain if a global fallback path would mix symbols. Log explicit error with symbol and manifest id. Prevents silent reuse of one checkpoint across multiple assets. **Done 2026-04-12:** `resolve_serving_paths` + per-tick loads; multi-symbol + global `NM_MODELS_*` files require manifest or abstain. | `decision_engine/pipeline.py`, `forecaster_model/inference/`, `app/contracts/` |
| FB-AP-004 | AP-ENF | B | MD | hardening | HIGH | Done | **Runtime guard — RL / policy binding:** Policy inference and training artifact resolution must be keyed by the same per-asset manifest; no shared policy weights applied to a different symbol without an explicit multi-head design (out of scope — forbidden for V1 of this epic). **Done 2026-04-12:** same resolution as FB-AP-003; policy MLP from manifest-only paths when multi-symbol + global files. | `models/policy/`, `decision_engine/`, `orchestration/` |
| FB-AP-005 | AP-STA | B | RT | feature | HIGH | Done | **Per-asset state machine:** Define states `uninitialized`, `initialized_not_active`, `active` (watching: inference + decision loop on schedule for this symbol only). Transitions: user **Initialize** runs init pipeline → `initialized_not_active`; **Start** → `active`; **Stop** from `active` → `initialized_not_active` and triggers flatten rule. Persist state per asset; expose via API for UI button labels (**Initialize** / **Start** / **Stop**). **Done 2026-04-12:** `app/runtime/asset_lifecycle_state.py` (JSON under `data/asset_lifecycle_state/`, **`NM_ASSET_LIFECYCLE_STATE_DIR`**); **`GET /assets/lifecycle/{symbol}`**, **`POST .../start`**, **`POST .../stop`** (mutate key); init success sets **`initialized_not_active`**; **`GET /status` → `asset_lifecycle`**; manifest delete clears lifecycle file; flatten still **FB-AP-032**. | `app/runtime/` (new module), `control_plane/api.py`, `tests/` |
| FB-AP-006 | AP-INI | B | DS | feature | HIGH | Done | **Init pipeline orchestrator:** Background thread in `orchestration/asset_init_pipeline.py` — **one global job at a time**; **API:** `POST /assets/init/{symbol}`, `GET /assets/init/jobs/{job_id}`. **Done 2026-04-12.** | `orchestration/asset_init_pipeline.py`, `control_plane/api.py`, `tests/test_asset_init_pipeline.py` |
| FB-AP-007 | AP-INI | B | DP | feature | HIGH | Done | **Init step — Kraken historical pull:** `orchestration/init_kraken_historical.py` — window from **`init_pipeline`** / **`NM_ASSET_INIT_BOOTSTRAP_*`**, **`fetch_symbol_bars_sync`**. **Done 2026-04-12.** | `orchestration/init_kraken_historical.py`, `app/config/settings.py`, `app/config/default.yaml`, `tests/test_init_kraken_historical.py` |
| FB-AP-008 | AP-INI | B | DP | feature | HIGH | Done | **Init step — clean and validate:** `data_plane/bootstrap_bars.py` — schema, dedupe timestamps, gap stats, invalid row drops. **Done 2026-04-12.** | `data_plane/bootstrap_bars.py`, `tests/test_bootstrap_bars.py` |
| FB-AP-009 | AP-INI | B | FR | feature | HIGH | Done | **Init step — features:** `FeaturePipeline.enrich_bars` on validated bootstrap bars; Parquet + `feature_manifest.json` under **`NM_ASSET_INIT_ARTIFACTS_DIR`** (`data/asset_init/<symbol>/init_<job_id>/`). **Done 2026-04-12.** | `orchestration/init_feature_artifacts.py`, `data_plane/features/pipeline.py`, `tests/test_init_feature_artifacts.py` |
| FB-AP-010 | AP-INI | B | FR | feature | HIGH | Done | **Init step — initial Forecaster training:** `orchestration/init_forecaster_distill.py` calls `train_distilled_mlp_forecaster` into **`<init_run>/forecaster/`** (synthetic teacher distillation; same trainer as `tb-train-forecaster-distill`). **Epochs:** `init_pipeline.forecaster_distill_epochs` / **`NM_ASSET_INIT_FORECASTER_DISTILL_EPOCHS`**. Skips with **skipped** status if `torch` missing. **Done 2026-04-12.** | `orchestration/init_forecaster_distill.py`, `orchestration/asset_init_pipeline.py`, `forecaster_model/training/distill_mlp.py`, `tests/test_init_forecaster_distill.py` |
| FB-AP-011 | AP-INI | B | MD | feature | HIGH | Done | **Init step — RL train or initialize:** `orchestration/init_policy_artifacts.py` materializes `MultiBranchMLPPolicy` → `<run_dir>/policy/policy_mlp.npz` (seed from symbol+job_id). **Done 2026-04-12.** | `orchestration/init_policy_artifacts.py`, `policy_model/policy/mlp_actor.py`, `tests/test_init_policy_register.py` |
| FB-AP-012 | AP-INI | B | PF | feature | HIGH | Done | **Init completion — register manifest:** `orchestration/init_register_manifest.py` writes `AssetModelManifest` with `forecaster_torch_path` / `policy_mlp_path` when files exist (atomic `save_manifest`). Per-asset lifecycle state (`initialized_not_active`) remains **FB-AP-005**. **Done 2026-04-12.** | `orchestration/init_register_manifest.py`, `app/runtime/asset_model_registry.py`, `tests/test_init_policy_register.py` |
| FB-AP-013 | AP-DAT | B | DS | spike | HIGH | Done | **Storage evaluation spike:** Document current bar storage (files, QuestDB, Parquet, etc.) vs requirements: long retention, fast range reads for charts, sequential reads for training, incremental append, deduplication. **Decision:** keep extend vs replace; if replace, migration plan for initialized assets. Deliver short **ADR** in `docs/`. **Done 2026-04-12:** [`ADR_CANONICAL_BAR_STORAGE.MD`](ADR_CANONICAL_BAR_STORAGE.MD) — extend QuestDB + contracts; Parquet for init/training artifacts; Redis latest-bar only. | `data_plane/storage/`, `docs/*.MD` |
| FB-AP-014 | AP-DAT | B | DS | feature | HIGH | Done | **Canonical per-asset bar table (seconds resolution):** Implement or standardize **one** logical schema per symbol for OHLCV at **1-second** (or sub-second bucket if product chooses; document) bars as the **authoritative** series. REST history and WS-derived aggregates **must** land in the same schema to avoid dual histories. **Done 2026-04-12:** `BarEvent.interval_seconds`; `data_plane/storage/canonical_bars.py`; QuestDB **`canonical_bars`** + `insert_bar`; bootstrap + init Parquet include **`interval_seconds`**; see [`ADR_CANONICAL_BAR_STORAGE.MD`](ADR_CANONICAL_BAR_STORAGE.MD). | `data_plane/`, `data_plane/storage/schemas.py`, QuestDB DDL or Parquet layout |
| FB-AP-015 | AP-DAT | B | DP | feature | HIGH | Done | **Idempotent merge and dedup:** Upsert or merge logic so backfill replays and live inserts never create duplicate keys `(symbol, ts)`; support **last-write-wins** or explicit conflict rules for OHLC. **Done 2026-04-12:** key ``(symbol, ts, interval_seconds)``; Polars `merge_canonical_bars` / `dedupe_canonical_bars_last_wins`; `QuestDBWriter.insert_bar` delete-then-insert; `query_bars` reads `canonical_bars`. | `data_plane/storage/`, `tests/` |
| FB-AP-016 | AP-WS | B | DP | feature | HIGH | Done | **WebSocket → canonical bars:** Ingest Kraken WS ticks/trades per symbol, aggregate **in process** into **`NM_MARKET_DATA_BAR_INTERVAL_SECONDS`** OHLC buckets **before** persist, matching REST bar format (same columns, UTC bucket start). **Done 2026-04-12:** `RollingBars.on_tick` returns completed bucket; `run_live_loop` maps Kraken pair → canonical symbol (`canonical_symbol_from_kraken_pair`); optional **`NM_QUESTDB_PERSIST_CANONICAL_BARS`** → `QuestDBWriter.insert_bar` → `canonical_bars`. Partial bar: only **completed** buckets are persisted; in-progress bucket feeds features via `bars_frame_with_partial`. | `data_plane/bars/rolling.py`, `data_plane/ingest/kraken_symbols.py`, `app/runtime/live_service.py`, `app/config/` |
| FB-AP-017 | AP-DAT | B | DS | feature | MEDIUM | Open | **Derived intervals for chart and training:** Materialized views or on-the-fly resample: **minute, hour, day, week, month** from canonical seconds (and **second** as native). Ensure chart API and training use these consistently; no duplicate conflicting rollups in different modules. | `data_plane/bars/`, API layer |
| FB-AP-018 | AP-BF | B | DP | feature | HIGH | Done | **Startup gap detection:** On app launch, for each **initialized** asset, read **max stored timestamp**; if behind wall-clock or last close, compute gap range. **Done 2026-04-12:** `QuestDBWriter.max_canonical_bar_timestamp`; `data_plane/storage/startup_gap_detection.py` (`detect_canonical_bar_gaps`, `last_closed_bucket_start_utc`); optional **`NM_QUESTDB_STARTUP_GAP_DETECTION`** — live loop logs `canonical_bar_gap` / `canonical_bar_gap_ok` for symbols with manifests (interval = `NM_MARKET_DATA_BAR_INTERVAL_SECONDS`). Backfill in **FB-AP-019**. | `data_plane/storage/`, `app/runtime/live_service.py`, `app/config/` |
| FB-AP-019 | AP-BF | B | DP | feature | HIGH | Done | **Startup Kraken backfill:** For each gap, fetch missing history from Kraken REST, validate, merge via FB-AP-015, update **current through** timestamp in registry or sidecar. **Done 2026-04-12:** `orchestration/startup_canonical_backfill.py` — optional **`NM_QUESTDB_STARTUP_KRAKEN_BACKFILL`** (YAML: `questdb.startup_kraken_backfill`); after FB-AP-018 gap list, fetches OHLC via `fetch_symbol_bars_async`, `validate_and_clean_init_bootstrap_bars`, `QuestDBWriter.insert_bar`; watermark JSON under **`NM_CANONICAL_BAR_WATERMARK_DIR`** (`data/canonical_bar_watermarks/<symbol>.json`). Empty QuestDB uses **`questdb.backfill_max_lookback_days`** (default 14) to cap REST window. Wired in `run_live_loop` before WS loop. | `orchestration/`, `app/runtime/`, `data_plane/` |
| FB-AP-020 | AP-UNI | B | EX | feature | MEDIUM | Open | **Alpaca tradable universe sync:** Periodic job lists **tradable** assets from Alpaca API, persists to local store (SQLite/JSON), refresh on schedule and on demand. Stores metadata needed for search (symbol, name, class). | `execution/adapters/` (Alpaca client), `data/` or `data_plane/`, `control_plane/api.py` |
| FB-AP-021 | AP-UNI | B | EX | feature | MEDIUM | Open | **Coinbase tradable universe sync:** Same pattern for Coinbase list products; persist separately with sync timestamps. | `execution/adapters/coinbase_live.py`, `execution/coinbase_advanced_http.py`, `data/` |
| FB-AP-022 | AP-UNI | B | PF | feature | MEDIUM | Open | **Cross-reference and supported set:** Define rule (intersection vs union vs “either lists as tradable”) to produce **platform_supported_symbols**; version the rule in docs. Build filtered list used **only** for search and eligibility — not for Kraken pair mapping (Kraken remains data path). | `docs/`, `app/config/`, registry module |
| FB-AP-023 | AP-UNI | B | UI | feature | MEDIUM | Open | **Search API:** HTTP endpoint returning paginated/filtered symbols from **FB-AP-022** set; no loading of OHLC for all symbols — metadata only until user selects one. | `control_plane/api.py`, `tests/` |
| FB-AP-024 | AP-API | B | CP | feature | HIGH | Done | **Symbol-scoped chart API:** GET bars for **one** symbol and interval + time range; reject or ignore requests that omit symbol. No endpoint returns all symbols’ series in one call for chart use. **Done 2026-04-12:** **`GET /assets/chart/bars`** — query params **`symbol`** (required), **`start`**, **`end`** (ISO-8601), optional **`interval_seconds`** (default **`NM_MARKET_DATA_BAR_INTERVAL_SECONDS`**), **`limit`** (≤50k); reads QuestDB **`canonical_bars`** via `control_plane/chart_bars.py`. | `control_plane/api.py`, `control_plane/chart_bars.py`, `data_plane/storage/questdb.py`, `tests/test_asset_chart_bars_api.py` |
| FB-AP-025 | AP-API | B | CP | feature | HIGH | Done | **Symbol-scoped fills / trade markers:** API for buy/sell points to overlay on Asset Page chart (from paper/live fills or intents — define source of truth). **Done 2026-04-12:** **Source of truth** — append-only **`data/trade_markers.jsonl`** (gitignored); **`GET /assets/chart/trade-markers`** with required **`symbol`**, **`start`**, **`end`**, optional **`limit`**; **`live_service`** appends a marker on **successful** `submit_order` (**`source=intent_submit`**, **`execution_mode`**, **`correlation_id`** from decision trace). Venue fills are a future extension. | `execution/trade_markers.py`, `control_plane/api.py`, `app/runtime/live_service.py`, `tests/` |
| FB-AP-026 | AP-UI-D | B | UI | change | HIGH | Done | **Dashboard redesign:** Single **PnL** chart with **timeframe dropdown** (align with existing PnL ledger time buckets); list **current holdings**; **remove** global hard start/stop controls and **remove** application-wide live vs paper toggle. **Done 2026-04-12:** **`GET /pnl/series`** — cumulative realized series from JSONL (`realized_bucket_series`); **`control_plane/pnl_panel.py`** — main-area holdings table + PnL line chart + summary; **`Home.py`** — removed sidebar power + execution-profile controls (read-only execution mode caption); per-asset controls remain **FB-AP-031** / **FB-AP-040**. | `execution/pnl_ledger.py`, `execution/pnl_summary.py`, `control_plane/api.py`, `control_plane/Home.py`, `control_plane/pnl_panel.py`, `tests/` |
| FB-AP-027 | AP-UI-A | B | UI | feature | HIGH | Done | **Asset Page — routing:** Only two primary routes: **Dashboard** and **Asset Page** (`/asset/<symbol>` or equivalent); deep links from search and slide menu. **Done 2026-04-12:** Streamlit multipage **`pages/Asset.py`** → route **`/Asset`**; query param **`?symbol=BTC-USD`**; **`control_plane/asset_page.py`** + **`asset_page_helpers.py`**; sidebar **`st.page_link`** from **Home** to Asset and per-**`GET /status`** symbols. | `control_plane/pages/Asset.py`, `control_plane/Home.py`, `control_plane/asset_page.py`, `tests/test_asset_page_routing.py` |
| FB-AP-028 | AP-UI-A | B | UI | feature | HIGH | Done | **Asset Page — candlestick chart:** OHLC visualization; interval selector: **second, minute, hour, day, week, month** (week/month from derived rollups). Lazy-fetch data **only for active symbol** when page loads or interval changes. **Done 2026-04-12:** Asset page **`fetch_bars_for_preset`** → **`GET /assets/chart/bars`**; Plotly candlestick when **`pip install -e ".[dashboard]"`**; week/month = Polars **`group_by_dynamic`** on daily bars; minute/hour optional resample from finer native rows if missing. | `control_plane/asset_page.py`, `control_plane/asset_chart_data.py`, `control_plane/asset_chart_fetch.py`, `pyproject.toml` (`plotly` in `[dashboard]`), `tests/test_asset_chart_data.py` |
| FB-AP-029 | AP-UI-A | B | UI | feature | HIGH | Done | **Asset Page — buy/sell markers:** Overlay entry/exit points from FB-AP-025 on chart; legend and tooltip. **Done 2026-04-12:** Same UTC window as OHLC preset; **`fetch_trade_markers_for_window`** → **`GET /assets/chart/trade-markers`**; Plotly **Buy** / **Sell** scatter (triangle up/down) with hover (side, qty, source, correlation id); **y** = last bar close ≤ marker time. | `control_plane/asset_chart_fetch.py`, `control_plane/asset_chart_markers.py`, `control_plane/asset_page.py`, `tests/test_asset_chart_markers.py` |
| FB-AP-030 | AP-UI-A | B | UI | feature | HIGH | Done | **Per-asset Paper/Live dropdown:** Persists per symbol (manifest or sidecar); execution adapter selection for **that symbol’s** orders only; remove reliance on single global `NM_EXECUTION_MODE` for UI-driven mode (env may remain for defaults). **Done 2026-04-12:** Sidecar **`data/asset_execution_mode/<symbol>.json`** (**`NM_ASSET_EXECUTION_MODE_DIR`**); **`GET`/`PUT`/`DELETE /assets/execution-mode/{symbol}`**; **`ExecutionService`** picks Alpaca vs Coinbase per **`OrderIntent.symbol`**; Asset page dropdown; **`GET /status`** → **`asset_execution_mode`**. | `app/runtime/asset_execution_mode.py`, `execution/service.py`, `control_plane/api.py`, `control_plane/asset_page.py`, `control_plane/streamlit_util.py`, `tests/` |
| FB-AP-031 | AP-UI-A | B | UI | feature | HIGH | Done | **Initialize / Start / Stop button:** Top-right control bound to **FB-AP-005** state: show **Initialize** if uninitialized, **Start** if initialized but not active, **Stop** if active (watching). Disable illegal transitions; show loading during init job. **Done 2026-04-12:** Asset page header column — **Initialize** → **`POST /assets/init/{symbol}`** + poll **`GET /assets/init/jobs/{job_id}`** with spinner; **Start** / **Stop** → lifecycle POSTs; illegal API errors surfaced via session toast. Helpers: **`control_plane/asset_lifecycle_actions.py`**. | `control_plane/asset_page.py`, `control_plane/asset_lifecycle_actions.py`, `tests/test_asset_lifecycle_actions.py` |
| FB-AP-032 | AP-EX | B | EX | feature | HIGH | Done | **Stop behavior — flatten:** When **Stop** from active state, cancel/stop watch loop **immediately**, submit **sell/close** for open position per venue rules (paper vs live), handle partial fills; update state to `initialized_not_active`. Document order type (market vs limit) in runbook. **Done 2026-04-12:** **`POST /assets/lifecycle/{symbol}/stop`** calls **`execution.flatten_stop.flatten_symbol_position_sync`** (market close via **`RiskEngine`** + **`FLATTEN_ALL`**, per-symbol adapter); response includes **`flatten`**; **502** if venue fetch/submit fails (lifecycle stays **active**). Watch-loop cancel remains runtime follow-up. | `execution/flatten_stop.py`, `execution/service.py`, `control_plane/api.py`, `tests/` |
| FB-AP-033 | AP-UI-A | B | UI | feature | MEDIUM | Open | **Left slide-in menu:** Lists **active** assets (symbols in `active` state); click navigates to Asset Page; available from both Dashboard and Asset Page; static chrome requirement. | `control_plane/` (session state / components) |
| FB-AP-034 | AP-RT | B | CP | feature | MEDIUM | Open | **Low-latency chart push:** WebSocket or SSE subscription **per open Asset Page symbol** streaming new bars or last-price updates from canonical store or ingest path; unsubscribe on navigation away. | `control_plane/api.py` (WS route), `uvicorn`, frontend loop |
| FB-AP-035 | AP-SCH | B | OP | feature | HIGH | Done | **App runtime as master:** Background schedulers (nightly train, universe refresh) register only when **application process** is running; no training when `.exe` closed. Document behavior. **Done 2026-04-12:** FastAPI **lifespan** starts/stops **`orchestration.app_scheduler`** daemon thread; optional **`NM_SCHEDULER_NIGHTLY_ENABLED`** + **`NM_SCHEDULER_NIGHTLY_INTERVAL_SECONDS`** (YAML `scheduler.*`); **`run_nightly_training_job`** wraps nightly campaign; **`GET /status` → `app_scheduler`**. Universe refresh still future. | `orchestration/app_scheduler.py`, `orchestration/nightly_retrain.py`, `control_plane/api.py`, `app/config/`, `tests/test_app_scheduler.py` |
| FB-AP-036 | AP-SCH | B | OP | feature | HIGH | Done | **Nightly forecaster retrain:** For every **initialized** asset (including those not **active**), run forecaster retrain on schedule while app is up. Per-asset artifact paths from manifest. **Done 2026-04-12:** **`run_nightly_training_job`** when **`NM_SCHEDULER_NIGHTLY_PER_ASSET_FORECASTER=true`** (default) runs **`run_training_campaign`** once per **`list_manifest_symbols()`** with **`artifact_dir=<NM_TRAINING_ARTIFACT_DIR>/nightly/<symbol>/`**; empty registry → skip. Torch manifest paths unchanged; quantile sklearn joblib still written per run dir. | `orchestration/nightly_retrain.py`, `app/config/settings.py`, `tests/test_nightly_per_asset_forecaster.py` |
| FB-AP-037 | AP-SCH | B | MD | feature | HIGH | Open | **Nightly RL conditional:** Run RL retrain **only** if that asset had **at least one trade** (or defined activity signal) in the prior day/session; **skip** RL on **zero-trade** days; **still** run forecaster nightly. Persist criterion in config. | `orchestration/nightly_retrain.py`, `data/` trade log |
| FB-AP-038 | AP-ENF | B | RT | feature | HIGH | Open | **Active watch loop — per symbol:** When `active`, run inference/decision/risk on **this symbol only** on configured cadence (e.g. per minute); no global loop iterating all symbols unless isolated tasks per symbol. | `app/runtime/live_service.py` or successor, task scheduler |
| FB-AP-039 | AP-REM | B | UI | change | HIGH | Open | **Remove global system power / hard stop:** Delete or hide UI and API for **system-wide** power off that halted all trading/inference; replace mental model with **app open = schedulers allowed**, **per-asset Stop** for trading watch. Align `GET /system/power` or remove if obsolete. | `control_plane/`, `control_plane/api.py`, `app/runtime/power_supervisor.py` |
| FB-AP-040 | AP-REM | B | CFG | change | HIGH | Open | **Remove app-wide live/paper switch:** Eliminate Streamlit/API control that sets **single** execution mode for whole system; execution mode comes from **per-asset** setting (FB-AP-030) with migration note for existing `.env` users. | `control_plane/Home.py`, `app/config/settings.py`, `docs/` |
| FB-AP-041 | AP-API | B | CP | reliability | MEDIUM | Done | **Operator docs and RUNBOOKS:** Update `README.md`, `docs/RUNBOOKS.MD`, and new **`docs/PER_ASSET_OPERATOR.MD`** (or similar) describing states, init pipeline, backfill, nightly rules, storage ADR, per-asset paper/live, Stop+flatten. **Partial Done 2026-04-12:** new **[`PER_ASSET_OPERATOR.MD`](PER_ASSET_OPERATOR.MD)** (manifest schema + API + env); **`README.md`** link; **`RUNBOOKS.MD`** §Per-asset model registry. Remaining FB-AP-P0 topics (states, init pipeline, backfill, per-asset UI) still open in other rows. | `docs/PER_ASSET_OPERATOR.MD`, `README.md`, `docs/RUNBOOKS.MD` |
| FB-AP-042 | AP-PAM | B | CI | feature | MEDIUM | Open | **Integration tests:** Cover manifest round-trip, state transitions, guard rails rejecting wrong symbol, chart API scoped query, merge idempotency for duplicate timestamps. | `tests/` |

### 2.3 Container / OCI cloud deployment (**FB-CONT-P0**)

**Brief:** Deliver a **browser-first**, **Linux OCI–based** deployment story so the same **images** run on a **developer laptop**, a **homelab server**, or **cloud** without rewriting the architecture. **Host OS is irrelevant** as long as a **Linux container runtime** exists (Docker Engine, Docker Desktop, Podman, etc.). **Stateful services** — **QuestDB**, **Redis**, **Qdrant**, **Prometheus**, **Grafana**, **Loki** — stay as **containers** with **named volumes** (or cloud block storage); the **application tier** (**FastAPI**, **Streamlit**, **`live_service`**, **decision/risk path**, **PyTorch / model inference**) ships in **one or more app images** with **secrets injected at runtime** (never baked into layers). **Edge:** optional **Caddy** or **Nginx** for **TLS**; optional **Tailscale** / SSH tunnel for private access. **Alignment:** [`AGENTS.md`](../AGENTS.md) — **Kraken-only market data**; **risk/signing** unchanged; no secrets in git. **Traceability:** brainstorm [`BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD`](BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD). **Complements** [`WINDOWS_OPERATOR_UI.MD`](WINDOWS_OPERATOR_UI.MD) (desktop `.bat` / launcher path).

**Table columns:** **Batch** = theme group; **Phase** = [§1.3](#13-priority-phase-category-status); **Affected files** = primary touch list.

| ID | Batch | Phase | Cat | Kind | Pri | Status | Summary | Affected files |
|----|-------|-------|-----|------|-----|--------|---------|----------------|
| FB-CONT-P0 | CONT-ALL | C | XX | feature | MEDIUM | Open | **(Epic parent)** Owns the **OCI / container** deployment track described in the **Brief** above: reproducible **Linux images** for the trading app + documented **Compose** wiring to existing **`infra/docker-compose.yml`** data plane, **CI** that builds (and optionally scans) images, **TLS** edge, **backup/restore** for stateful volumes, **cloud** runbook, and **README** operator guidance distinguishing **web + containers** from **Windows** convenience scripts. Sub-rows **FB-CONT-001**… are independently closable; implement **FB-CONT-001** before **FB-CONT-002** (image before compose references it). Link **BS-001** in PRs touching deployment docs. | `docs/QUEUE.MD`, `docs/BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD`, `infra/`, `Dockerfile`, `.dockerignore`, `.github/workflows/`, `README.md`, `docs/RUNBOOKS.MD`, `control_plane/`, `app/` |
| FB-CONT-001 | CONT-IMG | C | CP | platform | MEDIUM | Open | **App OCI image (Dockerfile):** Add a **multi-stage** `Dockerfile` at repo root (or `infra/docker/Dockerfile`) that installs Python **≥3.11**, `pip install -e ".[dev]"` or a **production** extra set (document which extras ship in prod vs dev), copies application code, sets **`PYTHONUNBUFFERED`**, and defines a clear **entrypoint** (shell script or `CMD`) that can start **uvicorn** for `control_plane.api:app`, **Streamlit** `control_plane/Home.py`, and **`live_service` / supervisor** as documented — **or** documents that operators run **one process per container** with **Compose** services if we split. Include **`.dockerignore`** to exclude `.venv`, `tests` artifacts, and secrets. **Acceptance:** `docker build` succeeds on a clean clone; image runs **`python -c "import app"`** (or equivalent smoke); **no API keys** in image layers. **Models:** PyTorch and native deps either in image or documented optional GPU tag — align with `pyproject.toml` extras. | `Dockerfile`, `.dockerignore`, `pyproject.toml`, `README.md` |
| FB-CONT-002 | CONT-CMP | C | CP | platform | MEDIUM | Open | **Compose: `app` service:** Extend **[`infra/docker-compose.yml`](../infra/docker-compose.yml)** (or add **`infra/docker-compose.app.yml`** overlay) with an **`app`** (or `trading-bot`) service that joins the existing **`trading-bot`** network, **`depends_on`** **questdb** / **redis** / **qdrant** (with health where applicable), mounts **read-only config** as needed, injects **`env_file: .env`** (document that **`.env` is gitignored**), and publishes **control plane** / **Streamlit** / **metrics** ports explicitly. Document **volume mounts** for `data/`, model artifacts, and registry paths used by **`NM_*`**. **Acceptance:** `docker compose up` brings up **app + data stack**; **`GET /status`** responds from the app container; **Kraken / execution** still configured only via env — no behavior change to risk/signing. | `infra/docker-compose.yml`, `infra/docker-compose.app.yml` (if split), `.env.example`, `README.md`, `docs/RUNBOOKS.MD` |
| FB-CONT-003 | CONT-CI | C | CP | platform | MEDIUM | Open | **CI: build (and scan) app image:** Add a workflow (e.g. **GitHub Actions**) on **PR** and/or **`main`** that builds the **FB-CONT-001** image, fails on build error, and optionally runs **Trivy** (filesystem or image) and/or **hadolint** on the Dockerfile. **No secrets** in CI — use dummy `NM_*` only if tests require env. **Acceptance:** failing Dockerfile breaks CI; document how operators mirror the same build locally. | `.github/workflows/`, `Dockerfile`, `docs/` |
| FB-CONT-004 | CONT-EDGE | C | CP | platform | MEDIUM | Open | **TLS reverse proxy (optional compose service):** Provide **`infra/docker-compose.edge.yml`** (or equivalent) adding **Caddy** or **Nginx** that fronts **Streamlit** + **FastAPI** with **HTTPS**, **upstream** to app container ports, and configurable **server name** / **ACME** (Let's Encrypt) or **mounted certs** for private CA. Document **localhost-only** vs **public** deployment. **Acceptance:** operator can hit **`https://`** to the UI when enabled; default **insecure HTTP** path still documented for LAN dev. | `infra/`, `docs/RUNBOOKS.MD`, `README.md` |
| FB-CONT-005 | CONT-BK | C | OP | reliability | MEDIUM | Open | **Backup and restore runbook — stateful volumes:** Document and optionally script **offline/consistent backup** for **QuestDB**, **Redis** (AOF), and **Qdrant** Docker volumes: **snapshot** or **vendor export** to **S3-compatible object storage**; **restore drill** steps (order of startup, volume attach). Include **retention** and **test restore** checklist. **Acceptance:** new section in [`RUNBOOKS.MD`](../RUNBOOKS.MD) with copy-paste commands; no production secrets in examples. | `docs/RUNBOOKS.MD`, `scripts/` (optional), `infra/` |
| FB-CONT-006 | CONT-CLOUD | C | CP | reliability | MEDIUM | Open | **Cloud deployment runbook:** One **opinionated** path (e.g. **single Linux VM + Docker Compose + systemd** *or* **AWS Fargate + EFS** — pick one primary) for moving the **same images** from homelab to cloud: **minimum instance size**, **networking** (security groups / firewall), **secrets** from **cloud secret manager** → runtime env, **persistent disk** mapping to Compose volumes, **egress** notes. **Second path** may be sketched as a bullet list. **Acceptance:** new subsection in **`README.md`** or **`docs/DEPLOY_CLOUD.MD`** linked from **`README.md`**; **BS-001** / **FB-CONT-P0** referenced. | `README.md`, `docs/*.MD`, `infra/` |
| FB-CONT-007 | CONT-DOC | C | CP | change | MEDIUM | Open | **README — deployment north star:** Add a short **“Deployment models”** subsection: **(A)** Windows **`.bat`** / optional launcher (**[`WINDOWS_OPERATOR_UI.MD`](WINDOWS_OPERATOR_UI.MD)**) vs **(B)** **Linux OCI / Compose / cloud** (**BS-001**, **FB-CONT-P0**) — browser-only, same stack. Link **QuestDB / Redis / Qdrant** roles in one sentence each. **Acceptance:** new users understand which path they are on; no duplicate of long brainstorm text. | `README.md`, `docs/BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD` |
| FB-CONT-008 | CONT-ALL | C | XX | spike | LOW | Open | **Spike — managed data services vs self-hosted:** Time-boxed comparison for **cloud** deployments: **managed Redis**, **QuestDB Cloud** or **managed TSDB**, **Qdrant Cloud** vs **self-hosted containers** on one VM — **cost**, **ops load**, **network latency** to app region, **migration** from current Compose defaults. Deliver **≤2 page ADR** in **`docs/`** with **recommendation** and **follow-up queue rows** only if we adopt a managed swap. **Traceability:** **BS-001** open questions. | `docs/*.MD`, `docs/QUEUE.MD` |

### 2.4 Multi-tenant web operator UX (**FB-UX-P0**)

**Brief:** Add **registered users** with **standard authentication** (sign-up, sign-in, sign-out, password hashing). Each user has **isolated trading context**: **PnL ledger**, **positions**, **per-asset model registry / manifests**, and **Alpaca + Coinbase API credentials** entered on an **Account** page. **Kraken** remains the **deployment-wide** market-data provider (quotes/bars) per [`AGENTS.md`](../AGENTS.md) — **not** per-user venue keys for data ingestion. **Web UX:** a **sign-in screen** is the default for unauthenticated visitors; **deep links** (e.g. Dashboard, Asset) **redirect to sign-in** if not logged in. **Chrome:** **settings gear** opens **Account** (keys, profile). **Dashboard/Asset extras** (from **BS-003**, promoted here): preflight/health strip, init job monitor, manifest/model card on Asset Page, CSV export, scheduler status panel, toasts, watchlist. **Traceability:** [`BRAINSTORM/BS-004_MULTI_TENANT_AUTH_ACCOUNT_UI.MD`](BRAINSTORM/BS-004_MULTI_TENANT_AUTH_ACCOUNT_UI.MD).

**Table columns:** **Batch** = theme group; **Phase** = [§1.3](#13-priority-phase-category-status); **Affected files** = primary touch list.

| ID | Batch | Phase | Cat | Kind | Pri | Status | Summary | Affected files |
|----|-------|-------|-----|------|-----|--------|---------|----------------|
| FB-UX-P0 | UX-ALL | B | XX | feature | HIGH | Open | **(Epic parent)** End-to-end **multi-tenant** web operator product: auth, per-user data and venue keys, gated navigation, Account + gear, integration with **FB-AP-P0** per-asset features per user. Sub-rows **FB-UX-001**… are independently closable; implement **FB-UX-001**–**002** before protected routes and per-user data (**FB-UX-003**–**007**). | `docs/QUEUE.MD`, `app/`, `control_plane/`, `execution/`, `docs/BRAINSTORM/BS-004_MULTI_TENANT_AUTH_ACCOUNT_UI.MD` |
| FB-UX-001 | AP-UX-AUTH | B | PF | feature | HIGH | Open | **User store and passwords:** Persistent **users** table (e.g. SQLite/Postgres) with **email**, **password hash** (argon2/bcrypt), **created_at**; no plaintext passwords. Registration API and validation. **Depends on:** choose DB alongside or instead of file-only stores for other per-user data. | `app/`, `control_plane/api.py`, `tests/` |
| FB-UX-002 | AP-UX-AUTH | B | PF | hardening | HIGH | Open | **Sessions / tokens:** HTTP-only **session cookies** or signed **JWT**; secure flags; logout invalidates session. FastAPI dependencies: **`get_current_user`**, **`require_user`** for mutating routes. | `app/`, `control_plane/api.py`, `tests/` |
| FB-UX-003 | AP-UX-ACC | B | UI | feature | HIGH | Open | **Sign-in / sign-up / sign-out UI:** Standard screens (email + password); success redirects to **Dashboard**. Wire to auth APIs. | `control_plane/`, `control_plane/api.py` |
| FB-UX-004 | AP-UX-ACC | B | UI | feature | HIGH | Open | **Route protection:** If user opens **Dashboard**, **Asset Page**, or any app URL **without** a valid session, **redirect to sign-in** (Streamlit: gate in entry or `st.query_params` / session state; document limitations). | `control_plane/Home.py`, `control_plane/pages/`, `docs/` |
| FB-UX-005 | AP-UX-ACC | B | UI | feature | HIGH | Open | **Settings gear:** Persistent **gear** control in app chrome linking to **Account** page. | `control_plane/` |
| FB-UX-006 | AP-UX-ACC | B | UI | feature | HIGH | Open | **Account page — venue keys:** Form to save **Alpaca** (paper) and **Coinbase** (live) **API keys and secrets** per user; **encrypt at rest**; never echo full secrets in UI after save; validate format optionally. **Kraken** market-data keys remain **`NM_*`** deployment env — surface read-only note on page. | `control_plane/`, `app/`, `execution/`, `tests/` |
| FB-UX-007 | AP-UX-DAT | B | PF | feature | HIGH | Open | **Per-user isolation:** Scope **PnL ledger**, **positions**, **asset model registry**, init artifacts, and **execution** adapter credentials by **`user_id`**. Migration plan for existing single-tenant `data/` layout. **Kraken** data plane unchanged (shared). | `app/`, `execution/`, `data_plane/` (paths), `control_plane/api.py`, `tests/` |
| FB-UX-008 | AP-UX-UI | B | UI | change | MEDIUM | Open | **Dashboard preflight / health strip:** Surface `GET /status` → `preflight`, `production_preflight`, execution profile in one glanceable row. | `control_plane/`, `control_plane/pnl_panel.py` |
| FB-UX-009 | AP-UX-UI | B | UI | feature | MEDIUM | Open | **Init pipeline monitor UI:** Panel for running **asset init** job (**symbol**, **job_id**, poll **`GET /assets/init/jobs/{id}`**). | `control_plane/` |
| FB-UX-010 | AP-UX-UI | B | UI | feature | MEDIUM | Open | **Asset Page — manifest / model card:** Read-only block: **forecaster/policy paths**, **last trained**, from **`GET /assets/models/{symbol}`** per user. | `control_plane/` |
| FB-UX-011 | AP-UX-UI | B | UI | change | MEDIUM | Open | **CSV export:** Download **P&L summary** and **positions** as CSV (client-side from API JSON). | `control_plane/` |
| FB-UX-012 | AP-UX-UI | B | UI | feature | MEDIUM | Open | **Scheduler / nightly status panel:** Show next/last run and last error for nightly jobs (**FB-AP-035**–**037**), backed by a small status API or agreed source. | `control_plane/`, `orchestration/`, `control_plane/api.py` |
| FB-UX-013 | AP-UX-UI | B | UI | feature | LOW | Open | **In-app notifications:** Toasts for init complete, train failure, flatten (**st.toast** + polling or events). | `control_plane/` |
| FB-UX-014 | AP-UX-UI | B | UI | feature | LOW | Open | **Watchlist / favorites:** Pin symbols for quick navigation to Asset Page (session or per-user prefs). | `control_plane/` |

### Compliance & release gates (open)

| ID | Type | Status | Summary | Verify / pointer |
|----|------|--------|---------|------------------|
| — | — | — | *No open HIGH gates — completed gates are listed in [§4](#4-completed-compliance-gates-hg).* | — |

### Backlog items (open) — forecaster epic **FB-FR-P0**

*No open **FB-FR-P0** items — closed **2026-04-12** (see [§5.5](#55-complete-archive-one-row-per-id): **FB-FR-P0-CLOSE**, **FB-FR-P0-WAIVE**).*

*Stretch:* full **human-spec** PyTorch VSN/xLSTM **training** stack remains **optional future work** (documented under **FB-FR-P0-WAIVE**); the repo ships **NumPy `ForecasterModel`** + **MLP distill** + **`NM_MODELS_FORECASTER_TORCH_PATH`** hot path.

#### Reference — optional full-spec PyTorch decomposition (waived; not open queue)

*Epic alignment:* [`FORECASTER_MODEL_ARCHITECTURE_SPEC.MD`](Human%20Provided%20Specs/FORECASTER_MODEL_ARCHITECTURE_SPEC.MD). **ID scheme:** **FB-FR-P0-0X-YY**. **Status:** all rows **Waived** with **FB-FR-P0-WAIVE** (2026-04-12) — kept only if you revisit native PyTorch training for every submodule.

#### FB-FR-P0-02 — **Graph** (PyTorch module stack)

| ID | Phase | Cat | Type | Status | Summary | Pointer |
|----|-------|-----|------|--------|---------|---------|
| FB-FR-P0-02 | A | FR | Feature | Waived | **(Epic)** VSN → latent CNN → multi-res xLSTM → regime fusion → quantile head; spec geometry | `forecaster_model/models/`, spec §tensor contracts |
| FB-FR-P0-02-01 | A | FR | Hardening | Waived | **Tensor / config contracts:** `ForecasterConfig` + batch shapes documented and asserted at module boundaries | `forecaster_model/config/schema.py`, `forecaster_model/schemas.py` |
| FB-FR-P0-02-02 | A | FR | Feature | Waived | **VSN:** variable selection block matches spec; unit tests on masked weights / dims | `forecaster_model/models/vsn.py` |
| FB-FR-P0-02-03 | A | FR | Feature | Waived | **Latent encoder:** CNN / patch encoder outputs align to downstream LSTM dims | `forecaster_model/models/latent_encoder.py` |
| FB-FR-P0-02-04 | A | FR | Feature | Waived | **Multi-res xLSTM:** stack + `xLSTMCell` integration; no training yet — forward-only | `forecaster_model/models/multi_resolution_xlstm.py`, `forecaster_model/models/xlstm_cell.py` |
| FB-FR-P0-02-05 | A | FR | Feature | Waived | **Regime fusion:** soft regime + fusion block wired to backbone output | `forecaster_model/models/fusion.py`, `forecaster_model/regime/` |
| FB-FR-P0-02-06 | A | FR | Feature | Waived | **Decoder + quantile head:** output ranks match `losses/quantile` and `ForecastPacket` consumers | `forecaster_model/models/decoder.py`, `forecaster_model/losses/quantile.py` |
| FB-FR-P0-02-07 | A | FR | Feature | Waived | **`TorchForecasterNet` (or equivalent):** single module composing 02-02…06; `forward()` test on synthetic batch | `forecaster_model/models/torch_forecaster_net.py` |
| FB-FR-P0-02-08 | A | FR | Platform | Waived | **Smoke:** export/import `state_dict` round-trip; dtype/device (`cpu`/`cuda`) forward | `tests/` (new) |

#### FB-FR-P0-03 — **Train** (loop, checkpoints, device)

| ID | Phase | Cat | Type | Status | Summary | Pointer |
|----|-------|-----|------|--------|---------|---------|
| FB-FR-P0-03 | A | FR | Feature | Waived | **(Epic)** End-to-end training loop, checkpoints, `[models_torch]` / `NM_MODELS_TORCH_DEVICE` | `forecaster_model/training/torch_trainer.py` |
| FB-FR-P0-03-01 | A | FR | Feature | Waived | **DataModule:** OHLC / manifest → `(batch, seq, feat)` batches; collate + pad; train/val split respects manifest | `forecaster_model/data/`, `forecaster_model/features/ohlc.py` |
| FB-FR-P0-03-02 | A | FR | Feature | Waived | **Train step:** quantile loss, backward, grad clip, optimizer; logging hooks | `forecaster_model/training/torch_trainer.py`, `forecaster_model/losses/` |
| FB-FR-P0-03-03 | A | FR | Feature | Waived | **Checkpointing:** save/load compatible with `training/checkpoint.py`; resume from step | `forecaster_model/training/checkpoint.py` |
| FB-FR-P0-03-04 | A | FR | Feature | Waived | **Walk-forward trainer:** fold driver using `walk_forward_indices` + no leakage between folds | `forecaster_model/training/walkforward.py`, `forecaster_model/training/torch_trainer.py` |
| FB-FR-P0-03-05 | A | FR | Platform | Waived | **Metrics + CLI:** `training/metrics.py` wired; minimal `python -m forecaster_model.training...` or script entry | `forecaster_model/training/metrics.py`, `pyproject.toml` `[project.scripts]` optional |
| FB-FR-P0-03-06 | A | FR | Platform | Waived | **Extras / device:** `[models_torch]` in `pyproject.toml`; `NM_MODELS_TORCH_DEVICE` + `training/device.py` used in trainer | `pyproject.toml`, `forecaster_model/training/device.py` |

#### FB-FR-P0-04 — **Calibrate** (conformal + ensemble on trained outputs)

| ID | Phase | Cat | Type | Status | Summary | Pointer |
|----|-------|-----|------|--------|---------|---------|
| FB-FR-P0-04 | A | FR | Feature | Waived | **(Epic)** Conformal + ensemble on **trained** forwards (not RNG reference) | `forecaster_model/calibration/`, `forecaster_model/models/ensemble.py` |
| FB-FR-P0-04-01 | A | FR | Feature | Waived | **Residuals → conformal state:** fit/save JSON compatible with hot-path loader | `forecaster_model/calibration/conformal.py` |
| FB-FR-P0-04-02 | A | FR | Feature | Waived | **Ensemble:** multiple members + aggregation; deterministic forward for tests | `forecaster_model/models/ensemble.py` |
| FB-FR-P0-04-03 | A | FR | Test | Waived | **Calibration eval:** coverage / pinball on held-out fold (small synthetic or real slice) | `tests/` |

#### FB-FR-P0-05 — **Serve** (checkpoint → decision hot path)

| ID | Phase | Cat | Type | Status | Summary | Pointer |
|----|-------|-----|------|--------|---------|---------|
| FB-FR-P0-05 | A | FR | Feature | Waived | **(Epic)** Export path from checkpoint to `DecisionPipeline` / `ForecastPacket` | `decision_engine/pipeline.py`, `forecaster_model/inference/` |
| FB-FR-P0-05-01 | A | FR | Feature | Waived | **Export bundle:** `.pt` + manifest JSON (schema version, horizons, normalization) | `forecaster_model/inference/` or `scripts/` |
| FB-FR-P0-05-02 | A | FR | Feature | Waived | **Inference API:** `torch_infer.py` / `build_from_ohlc.py` match training normalization | `forecaster_model/inference/torch_infer.py`, `build_from_ohlc.py` |
| FB-FR-P0-05-03 | A | FR | Feature | Waived | **`DecisionPipeline` integration:** `NM_MODELS_FORECASTER_TORCH_PATH` loads torch bundle; precedence vs NPZ / RNG documented | `decision_engine/pipeline.py` |
| FB-FR-P0-05-04 | A | FR | Hardening | Waived | **Guards + fallback:** `inference/guards.py` abstain behavior; README / RUNBOOKS operator table (torch vs NPZ vs RNG) | `forecaster_model/inference/guards.py`, `README.md`, `docs/RUNBOOKS.MD` |
| FB-FR-P0-05-05 | A | FR | Test | Waived | **Integration test:** load stub checkpoint → `ForecastPacket` or equivalent contract in pipeline | `tests/` |

*Note:* **Superseded** in part by shipped **`train_distilled_mlp_forecaster`**, **`tb-train-forecaster-distill`**, **`describe_walk_forward_folds`**, conformal tests, and **`tests/test_forecaster_torch_pipeline.py`** — see **FB-FR-P0-CLOSE** in [§5.5](#55-complete-archive-one-row-per-id).

### Backlog items (open) — multi-asset **FB-SPEC-07**

*No open **FB-SPEC-07** items — **FB-SPEC-07** + **07-01…03** completed **2026-04-12** (see [§5.5](#55-complete-archive-one-row-per-id)).*

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

*No open **FB-UI** items — completed rows are in [§5.4](#54-ui-observability-ci-backtesting-summary) / [§5.5](#55-complete-archive-one-row-per-id).*

#### FB-DASH — Dashboard & operator UX

*No open **FB-DASH** items — completed rows are in [§5.4](#54-ui-observability-ci-backtesting-summary) / [§5.5](#55-complete-archive-one-row-per-id).*

#### FB-MS — Microservices extraction (remaining production split)

*No open **FB-MS** items — slice **FB-MS-01 … FB-MS-06** completed **2026-04-11** (see [§5.5](#55-complete-archive-one-row-per-id)).*

*Parent epic:* RL policy per [`POLICY_MODEL_ARCHITECTURE_SPEC.MD`](Human%20Provided%20Specs/POLICY_MODEL_ARCHITECTURE_SPEC.MD). **Sub-rows FB-PL-P0-01 … FB-PL-P0-04 Done 2026-04-11** (NumPy stubs + offline env; full PyTorch RL remains future hardening).

*See also **HIGH** priority above (forecaster closeout). **FB-SPEC-06** is in [§5](#5-completed-archive-fb--themes).*

---

### LOW priority

*P3 — optional experiments and polish.*

| ID | Type | Status | Summary | Pointer |
|----|------|--------|---------|---------|
| — | — | — | *No open LOW items — see [§5](#5-completed-archive-fb--themes).* | — |

### Out of scope (not prioritized)

| ID | Kind | Phase | Cat | Pri | Status | Summary |
|----|------|-------|-----|-----|--------|---------|
| FB-N1 | deferred | D | XX | LOW | Open | Multi-exchange execution — **first slice Done 2026-04-13:** [`execution/adapter_registry.py`](../execution/adapter_registry.py), **`GET /status` → `execution_adapters`**, [`DEFERRED_ROADMAP.MD`](DEFERRED_ROADMAP.MD) §FB-N1 |
| FB-N3 | deferred | D | XX | LOW | Open | Portfolio optimization — **first slice Done 2026-04-13:** [`risk_engine/equal_weight.py`](../risk_engine/equal_weight.py), tests, [`DEFERRED_ROADMAP.MD`](DEFERRED_ROADMAP.MD) §FB-N3 |

*Full multi-venue / optimizer work remains deferred; see [`DEFERRED_ROADMAP.MD`](DEFERRED_ROADMAP.MD).*

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
| FB-UI-01-01, FB-UI-01-02 | 2026-04-11 | **`packaging/windows/`** — PyInstaller **`trading_bot_operator_launcher`**, **`build_operator_bundle.bat`**; **[`WINDOWS_OPERATOR_UI.MD`](WINDOWS_OPERATOR_UI.MD)** §1.1 distribution |
| FB-UI-02-01, FB-UI-02-02 | 2026-04-11 | **[`WINDOWS_OPERATOR_UI.MD`](WINDOWS_OPERATOR_UI.MD)** §2.2 shell comparison; **`operator_packaging/desktop_shell`**, **`run_desktop_dashboard.bat`**, **`pywebview`** in **`[dashboard]`** |
| FB-DASH-02-01 … FB-DASH-02-03 | 2026-04-11 | **`GET/POST /system/execution-profile`**, `execution_profile` on **`GET /status`**; Streamlit sidebar; RUNBOOKS + README; `data/execution_profile.json` pending intent |
| FB-DASH-03-01 … FB-DASH-03-03 | 2026-04-11 | **`PortfolioPositionRow`**, **`GET /portfolio/positions`**, Streamlit **Open positions** expander + refresh |
| FB-DASH-04-01 … FB-DASH-04-02 | 2026-04-11 | **Mark policy** (`kraken_mid` paper / `venue_only` live), Kraken **`/Ticker`** mid; **`mark_price`** + **`mark_price_policy`** on API; green/red uPnL in Streamlit |
| FB-DASH-05-01 … FB-DASH-05-03 | 2026-04-11 | **[`PNL_LEDGER.MD`](PNL_LEDGER.MD)** hybrid ledger; **`GET /pnl/summary`**, **`data/pnl_ledger.jsonl`**; Home **Aggregate P&L** panel |
| FB-DASH-06-01 … FB-DASH-06-02 | 2026-04-11 | Live **`st.dialog`** + type **LIVE**; **[`RUNBOOKS.MD`](RUNBOOKS.MD)** live checklist |
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
| FB-UI-01-01 | platform | 2026-04-11 | **`packaging/windows/trading_bot_operator_launcher.spec`** + **`trading_bot_operator_launcher.py`** + **`build_operator_bundle.bat`** — mirrors **`run.bat`** |
| FB-UI-01-02 | docs | 2026-04-11 | **[`WINDOWS_OPERATOR_UI.MD`](WINDOWS_OPERATOR_UI.MD)** — § **1.1 Windows distribution (PyInstaller)** |
| FB-UI-02-01 | docs | 2026-04-11 | **[`WINDOWS_OPERATOR_UI.MD`](WINDOWS_OPERATOR_UI.MD)** — § **2.2 Desktop shell options** (pywebview vs Electron vs Tauri) |
| FB-UI-02-02 | platform | 2026-04-11 | **`operator_packaging/desktop_shell/streamlit_desktop_host.py`**, **`run_desktop_dashboard.bat`**, **`pywebview`** in **`[dashboard]`** |
| FB-DASH-02-01 | control_plane | 2026-04-11 | **`GET/POST /system/execution-profile`**, **`execution_profile`** on **`GET /status`**, patch `default.yaml` + `.env` |
| FB-DASH-02-02 | control_plane | 2026-04-11 | Streamlit **Execution mode** sidebar + Apply → POST |
| FB-DASH-02-03 | ops | 2026-04-11 | **[`RUNBOOKS.MD`](RUNBOOKS.MD)** — execution mode section (restart after change) |
| FB-DASH-03-01 | contracts | 2026-04-11 | **`app/contracts/portfolio.py`** — `PortfolioPositionRow` + `position_snapshot_to_row` |
| FB-DASH-03-02 | control_plane | 2026-04-11 | **`GET /portfolio/positions`** — `execution/portfolio_positions.fetch_portfolio_positions` |
| FB-DASH-03-03 | control_plane | 2026-04-11 | **`control_plane/positions_panel.py`** — collapsible sidebar, optional 30s refresh |
| FB-DASH-04-01 | data_plane | 2026-04-11 | **`KrakenRESTClient.ticker_mid`**, settings **`portfolio_mark_price_source_*`**, **`execution/mark_price.py`** |
| FB-DASH-04-02 | control_plane | 2026-04-11 | **`enrich_row_with_mark`**, **`mark_price`** / **`mark_price_policy`** on **`GET /portfolio/positions`**; Streamlit green/red uPnL |
| FB-DASH-05-01 | docs | 2026-04-11 | **[`PNL_LEDGER.MD`](PNL_LEDGER.MD)** — hybrid JSONL + adapter positions; QuestDB/Alpaca future |
| FB-DASH-05-02 | control_plane | 2026-04-11 | **`GET /pnl/summary?range=`** — **`execution/pnl_ledger.py`** + **`execution/pnl_summary.py`** |
| FB-DASH-05-03 | control_plane | 2026-04-11 | **`control_plane/pnl_panel.py`** — timeframe selector + realized/unrealized on Home |
| FB-DASH-06-01 | control_plane | 2026-04-11 | **`control_plane/live_confirm.py`** + **`st.dialog`** — **`NM_STREAMLIT_LIVE_CONFIRM`** |
| FB-DASH-06-02 | ops | 2026-04-11 | **[`RUNBOOKS.MD`](RUNBOOKS.MD)** — **Live trading checklist** |
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
| FB-FR-P0-01 | models | 2026-04-11 | Leakage guard + walk-forward ordering tests (`DatasetManifest.assert_no_leakage`, `walk_forward_indices` disjoint ranges — `tests/test_dataset_manifest.py`) |
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
| FB-MS-01 | platform | 2026-04-11 | Optional Kraken WS → **`market.tick.normalized.v1`** + **`market.heartbeat.v1`** (`NM_MARKET_DATA_KRAKEN_WS`, `services/market_data_service/`) |
| FB-MS-02 | platform | 2026-04-11 | **`RollingBars` + `FeaturePipeline`** default in feature_service (`NM_FEATURE_SERVICE_USE_PIPELINE`, `services/feature_service/feature_pipeline_integration.py`) |
| FB-MS-03 | observability | 2026-04-11 | Optional QuestDB **`microservice_events`** (`NM_QUESTDB_PERSIST_MICROSERVICE_EVENTS`, `data_plane/storage/schemas.py`) |
| FB-MS-04 | ops | 2026-04-11 | Compose + **RUNBOOKS** — `NM_LIVE_SERVICE_APP_START_LOOP`, Kraken WS env on market_data |
| FB-MS-05 | reliability | 2026-04-11 | **RUNBOOKS** — real Alpaca paper smoke steps (keys in `.env` only); CI uses **`mock_alpaca_paper`** |
| FB-MS-06 | docs | 2026-04-11 | **`MICROSERVICES_SPLIT_PLAN.MD`** §11.1 progress checklist (program DoD tracking) |
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
| FB-SPEC-06 | pipeline | 2026-04-11 | **Done:** filesystem `active_model_set.json` + drift; `GET /status` → `model_artifacts`; optional `MLflowModelRegistry` params/metrics (no auto-promote); `POST /models/version` → `tb_model_version_info` (`tests/test_model_artifacts.py`). Champion/candidate compare remains **FB-AUDIT-03**. |
| FB-SPEC-07 | pipeline | 2026-04-12 | Parent + **07-01…03**: exposure/cross-symbol/operator doc — [`Specs/MULTI_ASSET_PORTFOLIO.MD`](Specs/MULTI_ASSET_PORTFOLIO.MD) |
| FB-SPEC-07-01 | risk | 2026-04-12 | **`risk_max_total_exposure_usd`** / **`risk_max_per_symbol_usd`** + **`NM_RISK_MAX_*`** vs `market_data_symbols` |
| FB-SPEC-07-02 | risk | 2026-04-12 | **Independent per-symbol** contract (no correlation/risk-parity hot path) |
| FB-SPEC-07-03 | docs | 2026-04-12 | Doc ↔ `RiskEngine.evaluate` + operator warnings |
| FB-FR-P0-CLOSE | models | 2026-04-12 | **FB-FR-P0** queue closed: **01** `DatasetManifest` + walk-forward tests; **02–06** NumPy `ForecasterModel` (VSN→…→quantiles) + **`build_torch_forecaster`** MLP; **03** `train_distilled_mlp_forecaster`, CLI **`tb-train-forecaster-distill`**, `describe_walk_forward_folds`; **04** conformal + ensemble + `tests/test_conformal_calibration_eval.py`; **05** `NM_MODELS_FORECASTER_TORCH_PATH` + `tests/test_forecaster_torch_pipeline.py` |
| FB-FR-P0-WAIVE | models | 2026-04-12 | **Waived (optional):** human-spec **full PyTorch** VSN/xLSTM *trainer* — not required for hot path; use **MLP distill** + NumPy reference |
| FB-MS-01 … FB-MS-06 | platform | 2026-04-11 | Microservices slices (Kraken WS path, FeaturePipeline, QuestDB audit, compose/runbooks, Alpaca smoke doc, §11 checklist) |
| FB-N1-S1 | execution | 2026-04-13 | **`execution/adapter_registry`**, **`GET /status` → `execution_adapters`**, [`DEFERRED_ROADMAP.MD`](DEFERRED_ROADMAP.MD) |
| FB-N3-S1 | risk | 2026-04-13 | **`risk_engine/equal_weight`**, `tests/test_equal_weight.py`, roadmap doc |
| FB-SPEC-08 | pipeline | 2026-04-11 | Preflight script + API |
| FB-T1 | test | 2026-04-10 | `smoke_credentials.py` |
| FB-T2 | runtime | 2026-04-10 | Shutdown signals test |
| FB-UI-01-02 | docs | 2026-04-11 | Windows distribution notes (AV, signing, bundle size) in `WINDOWS_OPERATOR_UI.MD` |
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
4. For multi-row epics (e.g. **FB-AP-P0**), assign a **Batch** code ([§1.3](#13-priority-phase-category-status)) and add **Affected files** (pipe-separated primary paths).
5. Add a row in [§2](#2-open-queue) (roadmap), [§3](#3-resolved-fixes-il) (fixes), or note completed gates in [§4](#4-completed-compliance-gates-hg). Use the next **FB-** or **HG-** id; for large epics prefer **-01, -02** sub-ids.  
6. Link **code paths** or tickets.

**Cursor Agent:** For a structured checklist aligned with §1–§2, use the project skill **Add to Queue** at [`.cursor/skills/add-to-queue/SKILL.md`](../.cursor/skills/add-to-queue/SKILL.md) (invoke with `/add-to-queue` or attach `@add-to-queue` when available).

---
