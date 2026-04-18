# Queue archive (historical tables)

**Documentation last reviewed:** **2026-04-18**.

**Purpose:** Full backlog tables, resolved fixes, and completed archive moved from the slim [`QUEUE.MD`](QUEUE.MD) for token-efficient agent workflows. **Agents implementing work:** use [`QUEUE_STACK.csv`](QUEUE_STACK.csv) only — each row is self-contained.

**Queue system:** This file is part of the **[queue system](QUEUE_SCHEMA.md)** (see [`QUEUE_SCHEMA.md`](QUEUE_SCHEMA.md) for all machinery files).

**Parent:** [`QUEUE.MD`](QUEUE.MD) — conventions and how to add items.

---


## 2. Open queue

*Open roadmap and deferred rows. Legacy column **Type** maps to [Kind](#11-kind-work-type) (Feature→feature, Platform→platform, …).*

**Status (2026-04-18):** **Canonical replacement program** — [**§2.7**](#27-canonical-replacement-program-fb-can) **FB-CAN-001**…**FB-CAN-038** are **Done**. **FB-CAN-039**…**FB-CAN-078** remain **Open** for the hard repo-to-canonical migration + reconfiguration + deletion track, except **FB-CAN-059** (runtime cutover guard), **FB-CAN-060** (canonical synthesis shim prune), **FB-CAN-061** (canonical config metadata validation), and **FB-CAN-062** (canonical contract conventions) **Done**. Legacy roadmap epics **FB-AP-P0**, **FB-UX-P0**, **FB-CONT-P0** remain **Done**; audit/setup/doc tracks remain **Done**. **Open queue is active**; sentinel **`_QUEUE_EMPTY_`** remains terminal-only for true-empty backlog states.

### 2.1 Queue snapshot (open by priority)

| Priority | Open count | IDs |
|----------|------------|-----|
| **HIGH** | **3** | FB-CAN-041, FB-CAN-045, FB-CAN-047 |
| **MEDIUM** | **21** | FB-CAN-039, FB-CAN-042..044, FB-CAN-046, FB-CAN-048..052, FB-CAN-054..056, FB-CAN-063, FB-CAN-066, FB-CAN-070..072, FB-CAN-074, FB-CAN-076, FB-CAN-078 |
| **LOW** | **11** | FB-CAN-040, FB-CAN-053, FB-CAN-057..058, FB-CAN-064..065, FB-CAN-067..069, FB-CAN-075, FB-CAN-077 |

*Update this table when **§2** Open rows change so “next **HIGH** open item” scans stay obvious (all three priority bands).*

*Deferred **FB-N1** / **FB-N3** are **Done** (closed for now); see [**Out of scope**](#out-of-scope-not-prioritized).*

### HIGH priority

*P0 / P1 — release-critical and core architecture.*

*No **Open** rows — audit backlog [**§2.5**](#25-post-audit-hardening-fb-aud) **Done**; stack empty in **[`QUEUE_STACK.csv`](QUEUE_STACK.csv)**.*

### 2.2 Per-asset product epic (**FB-AP-P0**)

**Brief:** Deliver **strict per-asset model ownership** (dedicated Forecaster and RL **lifecycle and artifacts** per symbol, **no shared ambiguous model state**), an **asset initialization pipeline** separate from live loops (Kraken REST historical → validate → features → initial forecaster + RL train/init → registered ready state), **tradable-universe** sync from **Alpaca** and **Coinbase** with cross-reference and **in-app search**, **symbol-scoped** data load and APIs (no global preload of other symbols’ series), **unified canonical bar storage** for REST backfill and WS-derived updates (seconds-resolution canonical table per asset as the single source of truth; higher intervals derived or materialized for charts and training), **startup backfill** of gaps for initialized assets, **two primary pages only** — **Dashboard** (holdings list + **PnL-only** chart with timeframe dropdown; **no** global hard start/stop) and **Asset Page** (one symbol: OHLC/candlestick chart with intervals **second / minute / hour / day / week / month**, buy/sell markers, per-asset **Paper/Live** dropdown, top-right **Initialize / Start / Stop** reflecting real state, **Stop** halts active watch and **flattens position**), **application running** as master for nightly jobs (when app is off, nothing runs), **nightly forecaster retrain** for every **initialized** asset, **RL retrain only when** that asset had **trading activity** in the prior period (skip RL on no-trade days; still run forecaster), **left slide-in menu** listing **active** (watching) assets with navigation to Asset Page, **low-latency chart updates** via push (WebSocket/SSE) scoped to the selected symbol, **remove** application-wide live/paper toggle and **remove** global system power hard stop. **Enforcement:** runtime and config must **enumerate** symbol → model instance and refuse cross-symbol reuse without explicit binding.

**Table columns:** **Batch** = theme group; **Phase** = [§1.3](#13-priority-phase-category-status); **Affected files** = primary touch list.

| ID | Batch | Phase | Cat | Kind | Pri | Status | Summary | Affected files |
|----|-------|-------|-----|------|-----|--------|---------|----------------|
| FB-AP-P0 | AP-ALL | B | XX | feature | HIGH | Done | **(Epic parent)** Owns end-to-end delivery of the per-asset operator product described in the **Brief** above: registry and enforcement for one Forecaster and one RL stack per traded asset; initialization pipeline; universe discovery; canonical historical storage and backfill; two-page UI; per-asset execution mode and lifecycle buttons; scheduling rules; removal of legacy global controls. Sub-rows **FB-AP-001**… are independently closable slices; implement in batch order where dependencies exist (registry before runtime wiring, storage contract before UI chart queries). **Done 2026-04-12:** all **FB-AP-001**…**FB-AP-042** rows **Done** — per-asset product track complete. | `docs/QUEUE.MD`, `app/`, `control_plane/`, `data_plane/`, `decision_engine/`, `execution/`, `forecaster_model/`, `models/`, `orchestration/`, `risk_engine/`, `tests/` |
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
| FB-AP-017 | AP-DAT | B | DS | feature | MEDIUM | Done | **Derived intervals for chart and training:** Materialized views or on-the-fly resample: **minute, hour, day, week, month** from canonical seconds (and **second** as native). Ensure chart API and training use these consistently; no duplicate conflicting rollups in different modules. **Done 2026-04-12:** **`data_plane/bars/derived_intervals.py`** — single Polars OHLC rollup; **week/month** charts build **daily** from **1s** canonical rows then **`NM_TRAINING_DATA_GRANULARITY_SECONDS`** step before week/month when 1s data exists; **`real_data_bars._resample_ohlc`** delegates to the same helper; **`control_plane/asset_chart_data`** wrappers. | `data_plane/bars/`, API layer |
| FB-AP-018 | AP-BF | B | DP | feature | HIGH | Done | **Startup gap detection:** On app launch, for each **initialized** asset, read **max stored timestamp**; if behind wall-clock or last close, compute gap range. **Done 2026-04-12:** `QuestDBWriter.max_canonical_bar_timestamp`; `data_plane/storage/startup_gap_detection.py` (`detect_canonical_bar_gaps`, `last_closed_bucket_start_utc`); optional **`NM_QUESTDB_STARTUP_GAP_DETECTION`** — live loop logs `canonical_bar_gap` / `canonical_bar_gap_ok` for symbols with manifests (interval = `NM_MARKET_DATA_BAR_INTERVAL_SECONDS`). Backfill in **FB-AP-019**. | `data_plane/storage/`, `app/runtime/live_service.py`, `app/config/` |
| FB-AP-019 | AP-BF | B | DP | feature | HIGH | Done | **Startup Kraken backfill:** For each gap, fetch missing history from Kraken REST, validate, merge via FB-AP-015, update **current through** timestamp in registry or sidecar. **Done 2026-04-12:** `orchestration/startup_canonical_backfill.py` — optional **`NM_QUESTDB_STARTUP_KRAKEN_BACKFILL`** (YAML: `questdb.startup_kraken_backfill`); after FB-AP-018 gap list, fetches OHLC via `fetch_symbol_bars_async`, `validate_and_clean_init_bootstrap_bars`, `QuestDBWriter.insert_bar`; watermark JSON under **`NM_CANONICAL_BAR_WATERMARK_DIR`** (`data/canonical_bar_watermarks/<symbol>.json`). Empty QuestDB uses **`questdb.backfill_max_lookback_days`** (default 14) to cap REST window. Wired in `run_live_loop` before WS loop. | `orchestration/`, `app/runtime/`, `data_plane/` |
| FB-AP-020 | AP-UNI | B | EX | feature | MEDIUM | Done | **Alpaca tradable universe sync:** Periodic job lists **tradable** assets from Alpaca API, persists to local store (SQLite/JSON), refresh on schedule and on demand. Stores metadata needed for search (symbol, name, class). **Done 2026-04-12:** SQLite **`NM_ALPACA_UNIVERSE_DB_PATH`** (default **`data/alpaca_universe.sqlite`**); **`GET /universe/alpaca`**, **`POST /universe/alpaca/sync`** (mutate); optional **`NM_ALPACA_UNIVERSE_SYNC_ENABLED`** + **`NM_ALPACA_UNIVERSE_SYNC_INTERVAL_SECONDS`**; **`GET /status` → `alpaca_universe`**; **`orchestration/alpaca_universe_sync.py`** (``TradingClient.get_all_assets`` crypto **active** **tradable**). | `execution/adapters/` (Alpaca client), `data/` or `data_plane/`, `control_plane/api.py` |
| FB-AP-021 | AP-UNI | B | EX | feature | MEDIUM | Done | **Coinbase tradable universe sync:** Same pattern for Coinbase list products; persist separately with sync timestamps. **Done 2026-04-12:** SQLite **`NM_COINBASE_UNIVERSE_DB_PATH`**; **`GET /universe/coinbase`**, **`POST /universe/coinbase/sync`**; **`CoinbaseAdvancedHTTPClient.list_spot_products_paginated`** (GET `/products` **SPOT**, cursor); JWT path includes query string; **`GET /status` → `coinbase_universe`**. | `execution/adapters/coinbase_live.py`, `execution/coinbase_advanced_http.py`, `data/` |
| FB-AP-022 | AP-UNI | B | PF | feature | MEDIUM | Done | **Cross-reference and supported set:** Define rule (intersection vs union vs “either lists as tradable”) to produce **platform_supported_symbols**; version the rule in docs. Build filtered list used **only** for search and eligibility — not for Kraken pair mapping (Kraken remains data path). **Done 2026-04-12:** **`app/runtime/platform_supported_universe.py`** — rule **v1** default **intersection** (Alpaca `canonical_symbol` ∩ Coinbase `product_id`); optional **`union`** via **`NM_PLATFORM_SUPPORTED_UNIVERSE_MODE`** / YAML **`universe.platform_supported_mode`**; **`GET /universe/platform-supported`**, **`GET /status` → `platform_supported_universe`**. | `docs/`, `app/config/`, `app/runtime/platform_supported_universe.py` |
| FB-AP-023 | AP-UNI | B | UI | feature | MEDIUM | Done | **Search API:** HTTP endpoint returning paginated/filtered symbols from **FB-AP-022** set; no loading of OHLC for all symbols — metadata only until user selects one. **Done 2026-04-12:** **`GET /universe/search`** — paginated rows with Alpaca/Coinbase metadata + **`paper_tradable` / `live_tradable`** flags; **`universe_search_payload`** / **`list_platform_supported_search_rows`** in **`app/runtime/platform_supported_universe.py`**. | `control_plane/api.py`, `app/runtime/platform_supported_universe.py`, `tests/` |
| FB-AP-024 | AP-API | B | CP | feature | HIGH | Done | **Symbol-scoped chart API:** GET bars for **one** symbol and interval + time range; reject or ignore requests that omit symbol. No endpoint returns all symbols’ series in one call for chart use. **Done 2026-04-12:** **`GET /assets/chart/bars`** — query params **`symbol`** (required), **`start`**, **`end`** (ISO-8601), optional **`interval_seconds`** (default **`NM_MARKET_DATA_BAR_INTERVAL_SECONDS`**), **`limit`** (≤50k); reads QuestDB **`canonical_bars`** via `control_plane/chart_bars.py`. | `control_plane/api.py`, `control_plane/chart_bars.py`, `data_plane/storage/questdb.py`, `tests/test_asset_chart_bars_api.py` |
| FB-AP-025 | AP-API | B | CP | feature | HIGH | Done | **Symbol-scoped fills / trade markers:** API for buy/sell points to overlay on Asset Page chart (from paper/live fills or intents — define source of truth). **Done 2026-04-12:** **Source of truth** — append-only **`data/trade_markers.jsonl`** (gitignored); **`GET /assets/chart/trade-markers`** with required **`symbol`**, **`start`**, **`end`**, optional **`limit`**; **`live_service`** appends a marker on **successful** `submit_order` (**`source=intent_submit`**, **`execution_mode`**, **`correlation_id`** from decision trace). Venue fills are a future extension. | `execution/trade_markers.py`, `control_plane/api.py`, `app/runtime/live_service.py`, `tests/` |
| FB-AP-026 | AP-UI-D | B | UI | change | HIGH | Done | **Dashboard redesign:** Single **PnL** chart with **timeframe dropdown** (align with existing PnL ledger time buckets); list **current holdings**; **remove** global hard start/stop controls and **remove** application-wide live vs paper toggle. **Done 2026-04-12:** **`GET /pnl/series`** — cumulative realized series from JSONL (`realized_bucket_series`); **`control_plane/pnl_panel.py`** — main-area holdings table + PnL line chart + summary; **`Home.py`** — removed sidebar power + execution-profile controls (read-only execution mode caption); per-asset controls remain **FB-AP-031** / **FB-AP-040**. | `execution/pnl_ledger.py`, `execution/pnl_summary.py`, `control_plane/api.py`, `control_plane/Home.py`, `control_plane/pnl_panel.py`, `tests/` |
| FB-AP-027 | AP-UI-A | B | UI | feature | HIGH | Done | **Asset Page — routing:** Only two primary routes: **Dashboard** and **Asset Page** (`/asset/<symbol>` or equivalent); deep links from search and slide menu. **Done 2026-04-12:** Streamlit multipage **`pages/Asset.py`** → route **`/Asset`**; query param **`?symbol=BTC-USD`**; **`control_plane/asset_page.py`** + **`asset_page_helpers.py`**; sidebar **`st.page_link`** from **Home** to Asset and per-**`GET /status`** symbols. | `control_plane/pages/Asset.py`, `control_plane/Home.py`, `control_plane/asset_page.py`, `tests/test_asset_page_routing.py` |
| FB-AP-028 | AP-UI-A | B | UI | feature | HIGH | Done | **Asset Page — candlestick chart:** OHLC visualization; interval selector: **second, minute, hour, day, week, month** (week/month from derived rollups). Lazy-fetch data **only for active symbol** when page loads or interval changes. **Done 2026-04-12:** Asset page **`fetch_bars_for_preset`** → **`GET /assets/chart/bars`**; Plotly candlestick when **`pip install -e ".[dashboard]"`**; week/month = Polars **`group_by_dynamic`** on daily bars; minute/hour optional resample from finer native rows if missing. | `control_plane/asset_page.py`, `control_plane/asset_chart_data.py`, `control_plane/asset_chart_fetch.py`, `pyproject.toml` (`plotly` in `[dashboard]`), `tests/test_asset_chart_data.py` |
| FB-AP-029 | AP-UI-A | B | UI | feature | HIGH | Done | **Asset Page — buy/sell markers:** Overlay entry/exit points from FB-AP-025 on chart; legend and tooltip. **Done 2026-04-12:** Same UTC window as OHLC preset; **`fetch_trade_markers_for_window`** → **`GET /assets/chart/trade-markers`**; Plotly **Buy** / **Sell** scatter (triangle up/down) with hover (side, qty, source, correlation id); **y** = last bar close ≤ marker time. | `control_plane/asset_chart_fetch.py`, `control_plane/asset_chart_markers.py`, `control_plane/asset_page.py`, `tests/test_asset_chart_markers.py` |
| FB-AP-030 | AP-UI-A | B | UI | feature | HIGH | Done | **Per-asset Paper/Live dropdown:** Persists per symbol (manifest or sidecar); execution adapter selection for **that symbol’s** orders only; remove reliance on single global `NM_EXECUTION_MODE` for UI-driven mode (env may remain for defaults). **Done 2026-04-12:** Sidecar **`data/asset_execution_mode/<symbol>.json`** (**`NM_ASSET_EXECUTION_MODE_DIR`**); **`GET`/`PUT`/`DELETE /assets/execution-mode/{symbol}`**; **`ExecutionService`** picks Alpaca vs Coinbase per **`OrderIntent.symbol`**; Asset page dropdown; **`GET /status`** → **`asset_execution_mode`**. | `app/runtime/asset_execution_mode.py`, `execution/service.py`, `control_plane/api.py`, `control_plane/asset_page.py`, `control_plane/streamlit_util.py`, `tests/` |
| FB-AP-031 | AP-UI-A | B | UI | feature | HIGH | Done | **Initialize / Start / Stop button:** Top-right control bound to **FB-AP-005** state: show **Initialize** if uninitialized, **Start** if initialized but not active, **Stop** if active (watching). Disable illegal transitions; show loading during init job. **Done 2026-04-12:** Asset page header column — **Initialize** → **`POST /assets/init/{symbol}`** + poll **`GET /assets/init/jobs/{job_id}`** with spinner; **Start** / **Stop** → lifecycle POSTs; illegal API errors surfaced via session toast. Helpers: **`control_plane/asset_lifecycle_actions.py`**. | `control_plane/asset_page.py`, `control_plane/asset_lifecycle_actions.py`, `tests/test_asset_lifecycle_actions.py` |
| FB-AP-032 | AP-EX | B | EX | feature | HIGH | Done | **Stop behavior — flatten:** When **Stop** from active state, cancel/stop watch loop **immediately**, submit **sell/close** for open position per venue rules (paper vs live), handle partial fills; update state to `initialized_not_active`. Document order type (market vs limit) in runbook. **Done 2026-04-12:** **`POST /assets/lifecycle/{symbol}/stop`** calls **`execution.flatten_stop.flatten_symbol_position_sync`** (market close via **`RiskEngine`** + **`FLATTEN_ALL`**, per-symbol adapter); response includes **`flatten`**; **502** if venue fetch/submit fails (lifecycle stays **active**). Watch-loop cancel remains runtime follow-up. | `execution/flatten_stop.py`, `execution/service.py`, `control_plane/api.py`, `tests/` |
| FB-AP-033 | AP-UI-A | B | UI | feature | MEDIUM | Done | **Left slide-in menu:** Lists **active** assets (symbols in `active` state); click navigates to Asset Page; available from both Dashboard and Asset Page; static chrome requirement. **Done 2026-04-12:** **`control_plane/streamlit_chrome.py`** — **Active (watching)** section from **`GET /status` → `asset_lifecycle.states`**; **`_active_watching_symbols`**; tests **`tests/test_streamlit_chrome_active.py`**. | `control_plane/streamlit_chrome.py`, `tests/` |
| FB-AP-034 | AP-RT | B | CP | feature | MEDIUM | Done | **Low-latency chart push:** WebSocket or SSE subscription **per open Asset Page symbol** streaming new bars or last-price updates from canonical store or ingest path; unsubscribe on navigation away. **Done 2026-04-12:** **`GET /assets/chart/stream`** (SSE `text/event-stream`, polls QuestDB for latest bar per **`interval_seconds`**, emits when **`ts`** advances); **`GET /assets/chart/latest-bar`**; **`QuestDBWriter.query_latest_bar`**; CORS middleware for browser **`EventSource`** from Streamlit; Asset page caption with URLs; **`control_plane/chart_stream.py`**. | `control_plane/api.py`, `data_plane/storage/questdb.py`, `tests/` |
| FB-AP-035 | AP-SCH | B | OP | feature | HIGH | Done | **App runtime as master:** Background schedulers (nightly train, universe refresh) register only when **application process** is running; no training when `.exe` closed. Document behavior. **Done 2026-04-12:** FastAPI **lifespan** starts/stops **`orchestration.app_scheduler`** daemon thread; optional **`NM_SCHEDULER_NIGHTLY_ENABLED`** + **`NM_SCHEDULER_NIGHTLY_INTERVAL_SECONDS`** (YAML `scheduler.*`); **`run_nightly_training_job`** wraps nightly campaign; **`GET /status` → `app_scheduler`**. Universe refresh still future. | `orchestration/app_scheduler.py`, `orchestration/nightly_retrain.py`, `control_plane/api.py`, `app/config/`, `tests/test_app_scheduler.py` |
| FB-AP-036 | AP-SCH | B | OP | feature | HIGH | Done | **Nightly forecaster retrain:** For every **initialized** asset (including those not **active**), run forecaster retrain on schedule while app is up. Per-asset artifact paths from manifest. **Done 2026-04-12:** **`run_nightly_training_job`** when **`NM_SCHEDULER_NIGHTLY_PER_ASSET_FORECASTER=true`** (default) runs **`run_training_campaign`** once per **`list_manifest_symbols()`** with **`artifact_dir=<NM_TRAINING_ARTIFACT_DIR>/nightly/<symbol>/`**; empty registry → skip. Torch manifest paths unchanged; quantile sklearn joblib still written per run dir. | `orchestration/nightly_retrain.py`, `app/config/settings.py`, `tests/test_nightly_per_asset_forecaster.py` |
| FB-AP-037 | AP-SCH | B | MD | feature | HIGH | Done | **Nightly RL conditional:** Run RL retrain **only** if that asset had **at least one trade** (or defined activity signal) in the prior day/session; **skip** RL on **zero-trade** days; **still** run forecaster nightly. Persist criterion in config. **Done 2026-04-12:** **`run_training_campaign`** (nightly) skips heuristic RL when **`NM_SCHEDULER_NIGHTLY_RL_REQUIRES_TRADE=true`** and **`data/trade_markers.jsonl`** has no row for the symbol in **`[now - lookback, now]`** (lookback = **`NM_SCHEDULER_NIGHTLY_RL_TRADE_LOOKBACK_DAYS`** or campaign **`lookback_days`**). Report field **`rl_skipped`**. | `execution/trade_activity.py`, `orchestration/training_campaign.py`, `app/config/settings.py`, `tests/` |
| FB-AP-038 | AP-ENF | B | RT | feature | HIGH | Done | **Active watch loop — per symbol:** When `active`, run inference/decision/risk on **this symbol only** on configured cadence (e.g. per minute); no global loop iterating all symbols unless isolated tasks per symbol. **Done 2026-04-12:** Optional **`NM_LIVE_WATCH_LIFECYCLE_GATE`** — **`run_decision_tick`** only when **`effective_lifecycle_state(symbol)==active`**; optional **`NM_LIVE_DECISION_MIN_INTERVAL_SECONDS`** per-symbol throttle. Default **off** / **0** preserves prior behavior. | `app/runtime/live_service.py`, `app/runtime/live_watch_gate.py`, `app/config/settings.py`, `tests/test_live_watch_gate.py` |
| FB-AP-039 | AP-REM | B | UI | change | HIGH | Done | **Remove global system power / hard stop:** Delete or hide UI and API for **system-wide** power off that halted all trading/inference; replace mental model with **app open = schedulers allowed**, **per-asset Stop** for trading watch. Align `GET /system/power` or remove if obsolete. **Done 2026-04-12:** Default **`NM_SYSTEM_POWER_LEGACY_ENABLED=false`** — **`is_on()`** always **True**; **`POST /system/power`** → **410**; **`GET /status`** includes **`system_power_legacy_enabled`**; legacy restores file + POST behavior. | `app/runtime/system_power.py`, `control_plane/api.py`, `tests/` |
| FB-AP-040 | AP-REM | B | CFG | change | HIGH | Done | **Remove app-wide live/paper switch:** Eliminate Streamlit/API control that sets **single** execution mode for whole system; execution mode comes from **per-asset** setting (FB-AP-030) with migration note for existing `.env` users. **Done 2026-04-12:** **`GET/POST /system/execution-profile`** → **410** unless **`NM_EXECUTION_PROFILE_LEGACY_API=true`**; **`GET /status` → `execution_profile`** documents default + **`legacy_api_enabled`**; **Home** sidebar caption points to per-asset mode. | `control_plane/api.py`, `control_plane/execution_profile.py`, `control_plane/Home.py`, `tests/`, `docs/` |
| FB-AP-041 | AP-API | B | CP | reliability | MEDIUM | Done | **Operator docs and RUNBOOKS:** Update `README.md`, `docs/RUNBOOKS.MD`, and new **`docs/PER_ASSET_OPERATOR.MD`** (or similar) describing states, init pipeline, backfill, nightly rules, storage ADR, per-asset paper/live, Stop+flatten. **Partial Done 2026-04-12:** new **[`PER_ASSET_OPERATOR.MD`](PER_ASSET_OPERATOR.MD)** (manifest schema + API + env); **`README.md`** link; **`RUNBOOKS.MD`** §Per-asset model registry. Remaining FB-AP-P0 topics (states, init pipeline, backfill, per-asset UI) still open in other rows. | `docs/PER_ASSET_OPERATOR.MD`, `README.md`, `docs/RUNBOOKS.MD` |
| FB-AP-042 | AP-PAM | B | CI | feature | MEDIUM | Done | **Integration tests:** Cover manifest round-trip, state transitions, guard rails rejecting wrong symbol, chart API scoped query, merge idempotency for duplicate timestamps. **Done 2026-04-12:** **`tests/test_fb_ap_042_integration.py`** — manifest→lifecycle→chart bars (mocked QuestDB) + delete→uninitialized; multi-symbol **`resolve_serving_paths`** abstain without ETH manifest; **`merge_canonical_bars_frames`** triple-append last-wins on duplicate key. | `tests/test_fb_ap_042_integration.py` |

### 2.3 Container / OCI cloud deployment (**FB-CONT-P0**)

**Brief:** Deliver a **browser-first**, **Linux OCI–based** deployment story so the same **images** run on a **developer laptop**, a **homelab server**, or **cloud** without rewriting the architecture. **Host OS is irrelevant** as long as a **Linux container runtime** exists (Docker Engine, Docker Desktop, Podman, etc.). **Stateful services** — **QuestDB**, **Redis**, **Qdrant**, **Prometheus**, **Grafana**, **Loki** — stay as **containers** with **named volumes** (or cloud block storage); the **application tier** (**FastAPI**, **Streamlit**, **`live_service`**, **decision/risk path**, **PyTorch / model inference**) ships in **one or more app images** with **secrets injected at runtime** (never baked into layers). **Edge:** optional **Caddy** or **Nginx** for **TLS**; optional **Tailscale** / SSH tunnel for private access. **Alignment:** [`AGENTS.md`](../AGENTS.md) — **Kraken-only market data**; **risk/signing** unchanged; no secrets in git. **Traceability:** brainstorm [`BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD`](BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD). **Complements** [`WINDOWS_OPERATOR_UI.MD`](WINDOWS_OPERATOR_UI.MD) (desktop `.bat` / launcher path).

**Table columns:** **Batch** = theme group; **Phase** = [§1.3](#13-priority-phase-category-status); **Affected files** = primary touch list.

| ID | Batch | Phase | Cat | Kind | Pri | Status | Summary | Affected files |
|----|-------|-------|-----|------|-----|--------|---------|----------------|
| FB-CONT-P0 | CONT-ALL | C | XX | feature | MEDIUM | Done | **(Epic parent)** Owns the **OCI / container** deployment track described in the **Brief** above: reproducible **Linux images** for the trading app + documented **Compose** wiring to existing **`infra/docker-compose.yml`** data plane, **CI** that builds (and optionally scans) images, **TLS** edge, **backup/restore** for stateful volumes, **cloud** runbook, and **README** operator guidance distinguishing **web + containers** from **Windows** convenience scripts. Sub-rows **FB-CONT-001**… are independently closable; implement **FB-CONT-001** before **FB-CONT-002** (image before compose references it). Link **BS-001** in PRs touching deployment docs. **Done 2026-04-12:** **FB-CONT-001**…**FB-CONT-008** **Done** — container/OCI track complete. | `docs/QUEUE.MD`, `docs/BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD`, `infra/`, `Dockerfile`, `.dockerignore`, `.github/workflows/`, `README.md`, `docs/RUNBOOKS.MD`, `control_plane/`, `app/` |
| FB-CONT-001 | CONT-IMG | C | CP | platform | MEDIUM | Done | **App OCI image (Dockerfile):** Add a **multi-stage** `Dockerfile` at repo root (or `infra/docker/Dockerfile`) that installs Python **≥3.11**, `pip install -e ".[dev]"` or a **production** extra set (document which extras ship in prod vs dev), copies application code, sets **`PYTHONUNBUFFERED`**, and defines a clear **entrypoint** (shell script or `CMD`) that can start **uvicorn** for `control_plane.api:app`, **Streamlit** `control_plane/Home.py`, and **`live_service` / supervisor** as documented — **or** documents that operators run **one process per container** with **Compose** services if we split. Include **`.dockerignore`** to exclude `.venv`, `tests` artifacts, and secrets. **Acceptance:** `docker build` succeeds on a clean clone; image runs **`python -c "import app"`** (or equivalent smoke); **no API keys** in image layers. **Models:** PyTorch and native deps either in image or documented optional GPU tag — align with `pyproject.toml` extras. **Done 2026-04-12:** root **`Dockerfile`** (Python **3.12** slim), **`pip install -e ".[alpaca,dashboard]"`**, **`infra/docker/entrypoint.sh`** (`api` / `streamlit` / `live` / `shell`), **`.dockerignore`**, **`README.md`** build/run snippet. | `Dockerfile`, `.dockerignore`, `infra/docker/entrypoint.sh`, `README.md` |
| FB-CONT-002 | CONT-CMP | C | CP | platform | MEDIUM | Done | **Compose: `app` service:** Extend **[`infra/docker-compose.yml`](../infra/docker-compose.yml)** (or add **`infra/docker-compose.app.yml`** overlay) with an **`app`** (or `trading-bot`) service that joins the existing **`trading-bot`** network, **`depends_on`** **questdb** / **redis** / **qdrant** (with health where applicable), mounts **read-only config** as needed, injects **`env_file: .env`** (document that **`.env` is gitignored**), and publishes **control plane** / **Streamlit** / **metrics** ports explicitly. Document **volume mounts** for `data/`, model artifacts, and registry paths used by **`NM_*`**. **Acceptance:** `docker compose up` brings up **app + data stack**; **`GET /status`** responds from the app container; **Kraken / execution** still configured only via env — no behavior change to risk/signing. **Done 2026-04-12:** **`infra/docker-compose.app.yml`** — **`trading-bot-api`** (port **8000**), **`trading-bot-streamlit`** (**8501**, **`NM_CONTROL_PLANE_URL`** → API), optional **`trading-bot-live`** (`--profile live`); **`env_file`**, **`../data:/app/data`**, service DNS env for QuestDB/Redis/Qdrant; **`README.md`** + **`docs/RUNBOOKS.MD`** + **`.env.example`** note. | `infra/docker-compose.app.yml`, `README.md`, `docs/RUNBOOKS.MD`, `.env.example` |
| FB-CONT-003 | CONT-CI | C | CP | platform | MEDIUM | Done | **CI: build (and scan) app image:** Add a workflow (e.g. **GitHub Actions**) on **PR** and/or **`main`** that builds the **FB-CONT-001** image, fails on build error, and optionally runs **Trivy** (filesystem or image) and/or **hadolint** on the Dockerfile. **No secrets** in CI — use dummy `NM_*` only if tests require env. **Acceptance:** failing Dockerfile breaks CI; document how operators mirror the same build locally. **Done 2026-04-12:** **`docker-image`** job in **`.github/workflows/ci.yml`** — `docker build`, `docker run` import smoke, **hadolint** (`.hadolint.yaml`), **Trivy** fs scan (informational exit 0); **`README.md`** mirror note. | `.github/workflows/ci.yml`, `.hadolint.yaml`, `README.md` |
| FB-CONT-004 | CONT-EDGE | C | CP | platform | MEDIUM | Done | **TLS reverse proxy (optional compose service):** Provide **`infra/docker-compose.edge.yml`** (or equivalent) adding **Caddy** or **Nginx** that fronts **Streamlit** + **FastAPI** with **HTTPS**, **upstream** to app container ports, and configurable **server name** / **ACME** (Let's Encrypt) or **mounted certs** for private CA. Document **localhost-only** vs **public** deployment. **Acceptance:** operator can hit **`https://`** to the UI when enabled; default **insecure HTTP** path still documented for LAN dev. **Done 2026-04-12:** **`infra/docker-compose.edge.yml`** (Caddy **8443→443**), **`infra/caddy/Caddyfile`** (**api.localhost** / **ui.localhost**, **`tls internal`**); **`README.md`** + **`docs/RUNBOOKS.MD`**. | `infra/docker-compose.edge.yml`, `infra/caddy/Caddyfile`, `README.md`, `docs/RUNBOOKS.MD` |
| FB-CONT-005 | CONT-BK | C | OP | reliability | MEDIUM | Done | **Backup and restore runbook — stateful volumes:** Document and optionally script **offline/consistent backup** for **QuestDB**, **Redis** (AOF), and **Qdrant** Docker volumes: **snapshot** or **vendor export** to **S3-compatible object storage**; **restore drill** steps (order of startup, volume attach). Include **retention** and **test restore** checklist. **Acceptance:** new section in [`RUNBOOKS.MD`](../RUNBOOKS.MD) with copy-paste commands; no production secrets in examples. **Done 2026-04-12:** **`docs/RUNBOOKS.MD`** §**Stateful Docker volumes — backup & restore (FB-CONT-005)** — `COMPOSE_PROJECT_NAME`, cold `tar` via **alpine**, restore drill, S3 retention/testing, per-engine notes. | `docs/RUNBOOKS.MD` |
| FB-CONT-006 | CONT-CLOUD | C | CP | reliability | MEDIUM | Done | **Cloud deployment runbook:** One **opinionated** path (e.g. **single Linux VM + Docker Compose + systemd** *or* **AWS Fargate + EFS** — pick one primary) for moving the **same images** from homelab to cloud: **minimum instance size**, **networking** (security groups / firewall), **secrets** from **cloud secret manager** → runtime env, **persistent disk** mapping to Compose volumes, **egress** notes. **Second path** may be sketched as a bullet list. **Acceptance:** new subsection in **`README.md`** or **`docs/DEPLOY_CLOUD.MD`** linked from **`README.md`**; **BS-001** / **FB-CONT-P0** referenced. **Done 2026-04-12:** **[`docs/DEPLOY_CLOUD.MD`](DEPLOY_CLOUD.MD)** — Path A VM+Compose+systemd, Path B Fargate+EFS sketch; **`README.md`** subsection + doc table link. | `docs/DEPLOY_CLOUD.MD`, `README.md` |
| FB-CONT-007 | CONT-DOC | C | CP | change | MEDIUM | Done | **README — deployment north star:** Add a short **“Deployment models”** subsection: **(A)** Windows **`.bat`** / optional launcher (**[`WINDOWS_OPERATOR_UI.MD`](WINDOWS_OPERATOR_UI.MD)**) vs **(B)** **Linux OCI / Compose / cloud** (**BS-001**, **FB-CONT-P0**) — browser-only, same stack. Link **QuestDB / Redis / Qdrant** roles in one sentence each. **Acceptance:** new users understand which path they are on; no duplicate of long brainstorm text. **Done 2026-04-12:** **`README.md`** — **Deployment models (FB-CONT-007)** table + data-plane one-liner; **`BS-001`** “See also” to README + **`DEPLOY_CLOUD.MD`**. | `README.md`, `docs/BRAINSTORM/BS-001_CLOUD_OCI_WEB_DEPLOYMENT.MD` |
| FB-CONT-008 | CONT-ALL | C | XX | spike | LOW | Done | **Spike — managed data services vs self-hosted:** Time-boxed comparison for **cloud** deployments: **managed Redis**, **QuestDB Cloud** or **managed TSDB**, **Qdrant Cloud** vs **self-hosted containers** on one VM — **cost**, **ops load**, **network latency** to app region, **migration** from current Compose defaults. Deliver **≤2 page ADR** in **`docs/`** with **recommendation** and **follow-up queue rows** only if we adopt a managed swap. **Traceability:** **BS-001** open questions. **Done 2026-04-12:** **[`ADR_MANAGED_DATA_SERVICES.MD`](ADR_MANAGED_DATA_SERVICES.MD)** — default **self-hosted** on VM; managed when SLA/HA/compliance; TSDB swap out of scope without migration project. | `docs/ADR_MANAGED_DATA_SERVICES.MD` |
| FB-CONT-009 | CONT-CI | C | CP | fix | MEDIUM | Done | **CI: Trivy action pin** — **`aquasecurity/trivy-action`** bare tag **0.28.0** no longer resolves after upstream tag migration / supply-chain remediation; pin **`@v0.35.0`** in **`.github/workflows/ci.yml`**. **Done 2026-02-06:** comment + pin; see **`docs/CI_ROOT_CAUSE_ANALYSIS_PROMPT.md`**. | `.github/workflows/ci.yml`, `docs/CI_ROOT_CAUSE_ANALYSIS_PROMPT.md` |

### 2.4 Multi-tenant web operator UX (**FB-UX-P0**)

**Brief:** Add **registered users** with **standard authentication** (sign-up, sign-in, sign-out, password hashing). Each user has **isolated trading context**: **PnL ledger**, **positions**, **per-asset model registry / manifests**, and **Alpaca + Coinbase API credentials** entered on an **Account** page. **Kraken** remains the **deployment-wide** market-data provider (quotes/bars) per [`AGENTS.md`](../AGENTS.md) — **not** per-user venue keys for data ingestion. **Web UX:** a **sign-in screen** is the default for unauthenticated visitors; **deep links** (e.g. Dashboard, Asset) **redirect to sign-in** if not logged in. **Chrome:** **settings gear** opens **Account** (keys, profile). **Dashboard/Asset extras** (from **BS-003**, promoted here): preflight/health strip, init job monitor, manifest/model card on Asset Page, CSV export, scheduler status panel, toasts, watchlist. **Traceability:** [`BRAINSTORM/BS-004_MULTI_TENANT_AUTH_ACCOUNT_UI.MD`](BRAINSTORM/BS-004_MULTI_TENANT_AUTH_ACCOUNT_UI.MD).

**Table columns:** **Batch** = theme group; **Phase** = [§1.3](#13-priority-phase-category-status); **Affected files** = primary touch list.

| ID | Batch | Phase | Cat | Kind | Pri | Status | Summary | Affected files |
|----|-------|-------|-----|------|-----|--------|---------|----------------|
| FB-UX-P0 | UX-ALL | B | XX | feature | HIGH | Done | **(Epic parent)** End-to-end **multi-tenant** web operator product: auth, per-user data and venue keys, gated navigation, Account + gear, integration with **FB-AP-P0** per-asset features per user. Sub-rows **FB-UX-001**… are independently closable; implement **FB-UX-001**–**002** before protected routes and per-user data (**FB-UX-003**–**007**). **Done 2026-04-12:** **FB-UX-001**…**FB-UX-014** **Done** — multi-tenant UX track complete. | `docs/QUEUE.MD`, `app/`, `control_plane/`, `execution/`, `docs/BRAINSTORM/BS-004_MULTI_TENANT_AUTH_ACCOUNT_UI.MD` |
| FB-UX-001 | AP-UX-AUTH | B | PF | feature | HIGH | Done | **User store and passwords:** Persistent **users** table (e.g. SQLite/Postgres) with **email**, **password hash** (argon2/bcrypt), **created_at**; no plaintext passwords. Registration API and validation. **Done 2026-04-12:** `app/runtime/user_store.py` — SQLite **`NM_AUTH_USERS_DB_PATH`** (default `data/users.sqlite`), Argon2 hashes; **`POST /auth/register`** body `{email,password}`; **`GET /status` → `user_store`**; Pydantic `RegisterRequest`/`RegisterResponse`. **Depends on:** choose DB alongside or instead of file-only stores for other per-user data. | `app/`, `control_plane/api.py`, `tests/` |
| FB-UX-002 | AP-UX-AUTH | B | PF | hardening | HIGH | Done | **Sessions / tokens:** HTTP-only **session cookies** or signed **JWT**; secure flags; logout invalidates session. FastAPI dependencies: **`get_current_user`**, **`require_user`** for mutating routes. **Done 2026-04-12:** SQLite **`operator_sessions`** table (same DB as users); **`NM_AUTH_SESSION_ENABLED`** (default false); **`POST /auth/login`** / **`POST /auth/logout`** / **`GET /auth/me`**; **`require_mutate_operator`** — **`X-API-Key`** when set, else valid session cookie when sessions enabled; cookie **`NM_AUTH_SESSION_COOKIE_NAME`**, **`httponly`**, **`NM_AUTH_SESSION_COOKIE_SECURE`**, **`NM_AUTH_SESSION_COOKIE_SAMESITE`**, TTL **`NM_AUTH_SESSION_TTL_SECONDS`**. | `app/`, `control_plane/api.py`, `tests/` |
| FB-UX-003 | AP-UX-ACC | B | UI | feature | HIGH | Done | **Sign-in / sign-up / sign-out UI:** Standard screens (email + password); success redirects to **Dashboard**. Wire to auth APIs. **Done 2026-04-12:** Streamlit **`pages/0_Login.py`** — sign-in form (**`POST /auth/login`**) → **`st.switch_page("Home.py")`**; expander register (**`POST /auth/register`**); sidebar **Sign in / Register** link + **Sign out** (**`POST /auth/logout`**). **`streamlit_util`** stores opaque session token in **`st.session_state`** and sends **`Cookie`** + **`X-API-Key`** on API calls; requires **`NM_AUTH_SESSION_ENABLED=true`** for login. | `control_plane/`, `control_plane/api.py` |
| FB-UX-004 | AP-UX-ACC | B | UI | feature | HIGH | Done | **Route protection:** If user opens **Dashboard**, **Asset Page**, or any app URL **without** a valid session, **redirect to sign-in** (Streamlit: gate in entry or `st.query_params` / session state; document limitations). **Done 2026-04-12:** **`NM_STREAMLIT_ROUTE_GUARD_ENABLED=true`** (default false) — **`require_streamlit_app_access()`** on **Home**, **Asset**, and **pages/1–6**; redirects to **`pages/0_Login.py`** unless **`NM_CONTROL_PLANE_API_KEY`** is set in the Streamlit process (automation bypass) or **`GET /auth/me`** returns 200 with session cookie. **Limitation:** Streamlit cannot enforce HTTP-only cookies for deep links without a prior run; first visit may hit login. | `control_plane/Home.py`, `control_plane/pages/`, `docs/` |
| FB-UX-005 | AP-UX-ACC | B | UI | feature | HIGH | Done | **Settings gear:** Persistent **gear** control in app chrome linking to **Account** page. **Done 2026-04-12:** **`control_plane/streamlit_chrome.py`** — **`render_app_sidebar()`** (Material **settings** icon) with **`st.sidebar.page_link("pages/7_Account.py", …)`** on Home, Asset, pages 0–6; **`pages/7_Account.py`** Account placeholder + **`GET /auth/me`** when session exists. | `control_plane/` |
| FB-UX-006 | AP-UX-ACC | B | UI | feature | HIGH | Done | **Account page — venue keys:** Form to save **Alpaca** (paper) and **Coinbase** (live) **API keys and secrets** per user; **encrypt at rest**; never echo full secrets in UI after save; validate format optionally. **Kraken** market-data keys remain **`NM_*`** deployment env — surface read-only note on page. **Done 2026-04-12:** SQLite **`user_venue_credentials`** (Fernet via **`NM_AUTH_VENUE_CREDENTIALS_MASTER_SECRET`**); **`GET`/`PUT /auth/venue-credentials`** (session **`require_user`**); masked response; Account page form + clear buttons; **`cryptography`** dependency. Execution still uses env **`NM_*`** venue keys until **FB-UX-007**. | `control_plane/`, `app/`, `execution/`, `tests/` |
| FB-UX-007 | AP-UX-DAT | B | PF | feature | HIGH | Done | **Per-user isolation:** Scope **PnL ledger**, **positions**, **asset model registry**, init artifacts, and **execution** adapter credentials by **`user_id`**. Migration plan for existing single-tenant `data/` layout. **Kraken** data plane unchanged (shared). **Done 2026-04-12:** **`NM_MULTI_TENANT_DATA_SCOPING=true`** — session cookie → **`ContextVar`** user id (middleware); paths under **`data/users/<id>/...`** for registry, lifecycle, execution-mode sidecars, canonical watermarks, **`pnl_ledger.jsonl`**, **`trade_markers.jsonl`**, **`asset_init/`**; legacy **`data/...`** unchanged when flag off. **`NM_MULTI_TENANT_EXECUTION_CREDENTIALS=true`** + **`NM_AUTH_VENUE_CREDENTIALS_MASTER_SECRET`** — **`merge_settings_for_execution`** for **`/portfolio/positions`**, **`compute_pnl_summary`**, **flatten**; **`POST /assets/init`** passes **`user_id`** into pipeline thread. | `app/`, `execution/`, `data_plane/` (paths), `control_plane/api.py`, `tests/` |
| FB-UX-008 | AP-UX-UI | B | UI | change | MEDIUM | Done | **Dashboard preflight / health strip:** Surface `GET /status` → `preflight`, `production_preflight`, execution profile in one glanceable row. **Done 2026-04-12:** **`control_plane/health_strip.py`** — **`render_dashboard_health_strip()`** on **`Home.py`** (after sidebar); three columns + divider before holdings/PnL. | `control_plane/`, `control_plane/pnl_panel.py` |
| FB-UX-009 | AP-UX-UI | B | UI | feature | MEDIUM | Done | **Init pipeline monitor UI:** Panel for running **asset init** job (**symbol**, **job_id**, poll **`GET /assets/init/jobs/{id}`**). **Done 2026-04-12:** **`control_plane/init_monitor.py`** — **`render_init_pipeline_monitor()`** on **`Home.py`**; Asset page sets **`st.session_state["init_monitor_job_id"]`** when starting init. | `control_plane/` |
| FB-UX-010 | AP-UX-UI | B | UI | feature | MEDIUM | Done | **Asset Page — manifest / model card:** Read-only block: **forecaster/policy paths**, **last trained**, from **`GET /assets/models/{symbol}`** per user. **Done 2026-04-12:** **`control_plane/asset_manifest_card.py`** — **`manifest_model_rows()`**; Asset page **Model / manifest** table (Polars `st.dataframe` or fallback `st.table`). | `control_plane/` |
| FB-UX-011 | AP-UX-UI | B | UI | change | MEDIUM | Done | **CSV export:** Download **P&L summary** and **positions** as CSV (client-side from API JSON). **Done 2026-04-12:** **`control_plane/csv_export.py`** — **`positions_payload_to_csv_text`**, **`pnl_summary_to_csv_text`**; **`pnl_panel.py`** — **`st.download_button`** for **`GET /portfolio/positions`** and **`GET /pnl/summary`** payloads. | `control_plane/` |
| FB-UX-012 | AP-UX-UI | B | UI | feature | MEDIUM | Done | **Scheduler / nightly status panel:** Show next/last run and last error for nightly jobs (**FB-AP-035**–**037**), backed by a small status API or agreed source. **Done 2026-04-12:** **`GET /scheduler/nightly`** — full status + **`last_report`** + RL/forecaster flags; **`app_scheduler`** extended with **`last_run_finished_utc`**, **`next_run_after_utc`**; **`control_plane/scheduler_panel.py`** on **Home**. | `control_plane/`, `orchestration/`, `control_plane/api.py` |
| FB-UX-013 | AP-UX-UI | B | UI | feature | LOW | Done | **In-app notifications:** Toasts for init complete, train failure, flatten (**st.toast** + polling or events). **Done 2026-04-12:** **`control_plane/app_toasts.py`** — **`maybe_toast`** + **`NM_STREAMLIT_TOASTS_ENABLED`**; Asset page **Initialize** / **Start** / **Stop** (flatten-aware); **init monitor** + **scheduler panel** transitions; **`post_mutate_response`** long timeout for Stop. | `control_plane/` |
| FB-UX-014 | AP-UX-UI | B | UI | feature | LOW | Done | **Watchlist / favorites:** Pin symbols for quick navigation to Asset Page (session or per-user prefs). **Done 2026-04-12:** **`control_plane/watchlist.py`** — session **`st.session_state`** list (max 24); **Pin ★** / **Remove ★** on **Asset** page; sidebar links in **`streamlit_chrome.py`**. | `control_plane/` |
| FB-UX-015 | AP-UX-ONB | B | UI | feature | HIGH | Done | **Dedicated Sign up page + mandatory venue API key onboarding:** **`pages/99_Sign_up.py`** (link from **`0_Login`** + sidebar); **`pages/98_Setup_API_keys.py`** when **`NM_STREAMLIT_VENUE_KEYS_REQUIRED=true`** — **`require_streamlit_app_access`** redirects until Alpaca (paper) or Coinbase (live) pair saved per **`GET /status` → `execution_mode`**; **`/auth/me`**, **`/auth/login`**, **`/auth/register`** return **`venue_keys_required`** / **`venue_keys_complete`**. Kraken remains deployment **`NM_*`** only. **Done 2026-04-13.** | `control_plane/pages/`, `control_plane/streamlit_util.py`, `control_plane/api.py`, `app/runtime/auth_venue_status.py` |
| FB-UX-016 | UX-DOC | B | OP | change | MEDIUM | Done | **Document venue key onboarding in README + RUNBOOKS:** operator paragraph under *Control plane security* (**`README.md`**) — sessions, route guard, **`NM_STREAMLIT_VENUE_KEYS_REQUIRED`**, pages 98/99, links to **`PER_ASSET_OPERATOR`** / **`RUNBOOKS`**. **`RUNBOOKS.MD`** — new § *Streamlit venue key onboarding (FB-UX-015)* — prerequisites (**`NM_AUTH_VENUE_CREDENTIALS_MASTER_SECRET`**), behavior, Kraken note. **Done 2026-04-13.** | `README.md`, `docs/RUNBOOKS.MD` |

### 2.5 Post-audit hardening (**FB-AUD-***)

**Brief:** Action items from the **[`AUDIT_CODE_REVIEW.MD`](AUDIT_CODE_REVIEW.MD)** code review (2026-04-17 — pass 3): production **control plane** hardening (API key, TLS, CORS), **Streamlit** route-guard documentation, **ops** warnings and optional **rate limiting**, **quality** passes on broad exception handlers, small **code** clarity in **`intent_gate`**, **CI** check for **`QUEUE_STACK.csv`** vs **§2**, **security** (CORS config, session cookies, **`pip-audit`**, **httpx** review), **tests** for **`require_mutate_operator`**, **tech-debt** inventory, **data plane** observability, **multi-tenant** path review, **`legacy/`** import hygiene.

**Traceability:** Audit IDs **DOC-A1**…**LEGACY-A1** map to **FB-AUD-001**…**017** (same order as [`AUDIT_CODE_REVIEW.MD`](AUDIT_CODE_REVIEW.MD) granular table). **Promoted to queue 2026-04-18.**

**Table columns:** **Batch** = **AUD-*** theme; **Phase** = **B** (operator product) unless noted.

| ID | Batch | Phase | Cat | Kind | Pri | Status | Summary | Affected files |
|----|-------|-------|-----|------|-----|--------|---------|----------------|
| FB-AUD-001 | AUD-DOC | B | OP | change | MEDIUM | Done | **Production API hardening (docs):** TLS + firewall + **`NM_CONTROL_PLANE_API_KEY`** / session auth; **restrict CORS** when API is not localhost-only; link from **`README.md`** / **`RUNBOOKS.MD`**. *(Audit DOC-A1.)* **Done 2026-04-19:** **`README.md`** — subsection *Production control plane hardening*; **`RUNBOOKS.MD`** — *Production / network exposure*. | `README.md`, `docs/RUNBOOKS.MD`, `docs/AUDIT_CODE_REVIEW.MD` |
| FB-AUD-002 | AUD-DOC | B | OP | change | MEDIUM | Done | **Streamlit route guard (docs):** when to set **`NM_STREAMLIT_ROUTE_GUARD_ENABLED`**, session env vars, and **`NM_CONTROL_PLANE_API_KEY`** bypass for automation. *(Audit DOC-A2.)* **Done 2026-04-19:** **`README.md`** — *Streamlit route guard*; **`RUNBOOKS.MD`** — *Streamlit route guard*; **`streamlit_util.py`** docstrings. | `README.md`, `docs/RUNBOOKS.MD`, `control_plane/streamlit_util.py` |
| FB-AUD-003 | AUD-OPS | B | OP | platform | MEDIUM | Done | **Startup warning:** log or warn when **`control_plane_api_key`** unset **and** bind address is non-loopback (misconfiguration risk). *(Audit OPS-A1.)* **Done 2026-02-06:** **`lifespan`** warning in **`control_plane/api.py`**; **RUNBOOKS** link. | `control_plane/api.py`, `docs/RUNBOOKS.MD` |
| FB-AUD-004 | AUD-OPS | B | OP | platform | MEDIUM | Done | **Optional rate limiting** for **`control_plane`** when exposed publicly (middleware or reverse-proxy notes). *(Audit OPS-A2.)* **Done 2026-02-06:** **`ControlPlaneRateLimitMiddleware`** + env flags; **RUNBOOKS**. | `control_plane/api.py`, `docs/RUNBOOKS.MD`, `infra/` |
| FB-AUD-005 | AUD-QA | B | OP | tech_debt | LOW | Done | **Exception logging pass:** **`app/runtime/live_service.py`** — ensure **`except Exception`** paths log with **symbol** / correlation context. *(Audit QA-A1.)* **Done 2026-02-06:** symbol context on reconcile/sentiment logs. | `app/runtime/live_service.py` |
| FB-AUD-006 | AUD-QA | B | UI | tech_debt | LOW | Done | **Exception logging pass:** **`control_plane/asset_page.py`**, **`streamlit_util.py`** — user-visible error surfaces. *(Audit QA-A2.)* **Done 2026-02-06:** logging on API caption / logout failures. | `control_plane/asset_page.py`, `control_plane/streamlit_util.py` |
| FB-AUD-007 | AUD-CODE | B | CP | change | MEDIUM | Done | **Comment / docstring** in **`execution/intent_gate.py`**: clarify “no **`risk_signing_secret`** → gate open” vs **`preflight`** live checks. *(Audit CODE-A1.)* **Done 2026-02-06:** docstring on **`execution_allowed`**. | `execution/intent_gate.py` |
| FB-AUD-008 | AUD-CI | B | CP | platform | MEDIUM | Done | **Optional CI script:** validate **`QUEUE_STACK.csv`** **Open** rows vs **`QUEUE_ARCHIVE.MD`** §2 (catch drift). *(Audit CI-A1.)* **Done 2026-02-06:** **`scripts/ci_queue_consistency.py`** + **lint-test** job step. | `scripts/ci_queue_consistency.py`, `docs/QUEUE_STACK.csv`, `docs/QUEUE_ARCHIVE.MD` |
| FB-AUD-009 | AUD-SEC | B | CP | hardening | MEDIUM | Done | **CORS:** env-configurable **`allow_origins`**; default **`*`** only in dev — document production allowlist. *(Audit SEC-A1.)* **Done 2026-02-06:** **`NM_CONTROL_PLANE_CORS_ALLOW_ORIGINS`**, README/RUNBOOKS/`.env.example`. | `control_plane/api.py`, `.env.example`, `README.md` |
| FB-AUD-010 | AUD-SEC | B | OP | change | MEDIUM | Done | **Document** **`NM_AUTH_SESSION_COOKIE_SECURE`** (and related) for HTTPS vs local HTTP. *(Audit SEC-A2.)* **Done 2026-02-06:** README + RUNBOOKS + `.env.example`. | `README.md`, `docs/RUNBOOKS.MD`, `.env.example` |
| FB-AUD-011 | AUD-SEC | B | CP | platform | MEDIUM | Done | **`pip-audit`** (or equivalent) in **dev docs** / optional **CI** job for supply chain. *(Audit SEC-A3.)* **Done 2026-02-06:** CI step (continue-on-error) + README. | `README.md`, `.github/workflows/ci.yml` |
| FB-AUD-012 | AUD-SEC | B | CP | hardening | MEDIUM | Done | **Inventory **`httpx`** calls** to external APIs: timeouts, TLS, error handling. *(Audit SEC-A4.)* **Done 2026-02-06:** **RUNBOOKS** table. | `docs/RUNBOOKS.MD` |
| FB-AUD-013 | AUD-TEST | B | CP | platform | MEDIUM | Done | **Tests:** **`require_mutate_operator`** rejects wrong/missing key when **`NM_CONTROL_PLANE_API_KEY`** is set. *(Audit TEST-A1.)* **Done 2026-02-06:** **`tests/test_control_plane_api_key_mutate.py`**. | `tests/test_control_plane_api_key_mutate.py` |
| FB-AUD-014 | AUD-DEBT | B | OP | tech_debt | LOW | Done | **Inventory** silent **`except: pass`** / bare **`pass`** after exceptions; replace with logging where appropriate. *(Audit DEBT-A1.)* **Done 2026-02-06:** RUNBOOKS inventory note (incremental refactors deferred). | `docs/RUNBOOKS.MD` |
| FB-AUD-015 | AUD-DATA | B | DP | reliability | LOW | Done | **Observability:** QuestDB writer / ingest errors surfaced (metrics or structured logs) for operators. *(Audit DATA-A1.)* **Done 2026-02-06:** **`tb_questdb_write_fail_total`** + RUNBOOKS. | `data_plane/storage/questdb.py`, `observability/metrics.py`, `docs/RUNBOOKS.MD` |
| FB-AUD-016 | AUD-MT | B | PF | hardening | LOW | Done | **Multi-tenant review:** paths respect tenant prefix when **`NM_MULTI_TENANT_DATA_SCOPING=true`**. *(Audit MT-A1.)* **Done 2026-02-06:** RUNBOOKS checklist. | `docs/RUNBOOKS.MD` |
| FB-AUD-017 | AUD-LEG | B | OP | tech_debt | LOW | Done | **`legacy/`** remains isolated — no new imports from **`legacy/`** into main V3 tree (align with **`AGENTS.md`**). *(Audit LEGACY-A1.)* **Done 2026-02-06:** **`scripts/ci_spec_compliance.sh`** grep for **`from legacy.`** / **`import legacy.`**. | `scripts/ci_spec_compliance.sh`, `legacy/` |

### 2.6 Full audit 2026-04-13 (**FB-AUD-018**…**020**)

**Traceability:** [`docs/FULL_AUDIT.md`](FULL_AUDIT.md) §2 / §5 · [`docs/reports/AUDIT_REPORT_2026-04-13_full.md`](reports/AUDIT_REPORT_2026-04-13_full.md)

| ID | Batch | Phase | Cat | Kind | Pri | Status | Summary | Affected files |
|----|-------|-------|-----|------|-----|--------|---------|----------------|
| FB-AUD-018 | AUD-SUP | B | CP | platform | MEDIUM | Done | **pip-audit CI policy:** `scripts/ci_pip_audit.sh` + dedicated `.audit-venv/`; release-blocking in CI; `requests>=2.33`, `jwcrypto>=1.5.7`. *(AUD-SUP-001.)* **Done 2026-04-13.** | `scripts/ci_pip_audit.sh`, `.github/workflows/ci.yml`, `pyproject.toml` |
| FB-AUD-019 | AUD-SEC | B | CP | platform | MEDIUM | Done | **Gitleaks in CI:** Docker job `zricethezav/gitleaks:v8.21.2`; local one-liner in AGENTS/RUNBOOKS. *(AUD-SEC-REPO-001.)* **Done 2026-04-13.** | `.github/workflows/ci.yml`, `AGENTS.md`, `docs/RUNBOOKS.MD` |
| FB-AUD-020 | AUD-STATIC | B | CP | tech_debt | LOW | Done | **Bandit:** `scripts/ci_bandit.sh`, `[tool.bandit]` in `pyproject.toml`, CI step; reviewed `nosec` on RSS/torch. *(AUD-STATIC-001.)* **Done 2026-04-13.** | `scripts/ci_bandit.sh`, `pyproject.toml`, `.github/workflows/ci.yml` |

### 2.7 Canonical replacement program (**FB-CAN-***)

**Brief:** Hard replacement from current architecture to canonical specs in `docs/Human Provided Specs/new_specs/canonical/`. This program explicitly replaces conflicting legacy abstractions (DecisionPipeline/ForecastPacket/PolicySystem-centric flow, classic hard-gate risk, thin execution service, and old config/doc surfaces) rather than preserving dual paths.

| ID | Batch | Phase | Cat | Kind | Pri | Status | Summary | Affected files |
|----|-------|-------|-----|------|-----|--------|---------|----------------|
| FB-CAN-001 | CAN-AUD | A | XX | platform | HIGH | Done | **Gap audit:** `docs/reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md` — authoritative repo-vs-canonical audit, delete/create/refactor plan, explicit **FB-CAN-001..014** domain table. **Done 2026-04-18.** | `docs/reports/REPO_VS_CANONICAL_SPECS_GAP_AUDIT.md`, core runtime modules |
| FB-CAN-002 | CAN-DOC | A | OP | change | HIGH | Done | **Canonical docs adoption:** `docs/CANONICAL_SPEC_INDEX.MD`, README + Specs/Human README precedence, banners on `docs/Specs/*.MD` and legacy root human specs. **Done 2026-04-18.** | `README.md`, `docs/Specs/`, `docs/Human Provided Specs/`, `docs/CANONICAL_SPEC_INDEX.MD` |
| FB-CAN-003 | CAN-CFG | A | CFG | feature | HIGH | Done | **Canonical config bundle:** `app/config/canonical_config.py`, `AppSettings.canonical`, optional `apex_canonical` in YAML; legacy projection + merge. **Done 2026-04-18.** | `app/config/settings.py`, `app/config/default.yaml`, `app/config/canonical_config.py`, tests/docs |
| FB-CAN-004 | CAN-STATE | A | FR | feature | HIGH | Done | **State engine:** `decision_engine/state_engine.py`, `CanonicalStateOutput` on `RegimeOutput.apex`, `RiskState` degradation sizing in `RiskEngine`. **Done 2026-04-18.** | `decision_engine/`, `app/contracts/`, `risk_engine/`, tests |
| FB-CAN-005 | CAN-TRIG | A | FR | feature | HIGH | Done | **Trigger engine:** `trigger_engine.py`, `TriggerOutput`, diagnostics on `ForecastPacket`, policy gate + size scale. **Done 2026-04-18.** | `decision_engine/`, `app/contracts/`, tests |
| FB-CAN-006 | CAN-AUCT | A | FR | feature | HIGH | Done | **Auction:** `auction_engine.py`, ranked long/short vs flat, penalties + top-N/notional cap, diagnostics on packet. **Done 2026-04-18.** | `decision_engine/`, `app/contracts/`, tests |
| FB-CAN-007 | CAN-RISK | A | RT | feature | HIGH | Done | **Canonical sizing:** `canonical_sizing.py`, layered notional + `last_risk_sizing`; hard gates unchanged. **Done 2026-04-18.** | `risk_engine/`, `app/contracts/risk.py`, tests |
| FB-CAN-008 | CAN-EXEC | A | EX | feature | HIGH | Done | **Execution guidance:** `execution_logic.py`, `ExecutionService` prepare path, live `execution_context` + feedback stub. **Done 2026-04-18.** | `execution/`, `app/contracts/`, tests |
| FB-CAN-009 | CAN-REPL | A | DS | feature | MEDIUM | Done | **Replay:** `ReplayRunContract`, canonical event envelopes, fault injection, deterministic tick. **Done 2026-04-18.** | `backtesting/`, `app/contracts/`, `decision_engine/run_step.py`, tests |
| FB-CAN-010 | CAN-MON | B | OB | feature | MEDIUM | Done | **Monitoring:** `canonical_metrics.py` `tb_canonical_*`, Grafana panels, `MONITORING_CANONICAL.MD`. **Done 2026-04-18.** | `observability/`, `infra/`, docs |
| FB-CAN-011 | CAN-GOV | B | OP | platform | MEDIUM | Done | **Release gating + experiment registry:** `orchestration/release_gating.py` (`ReleaseCandidate`, `evaluate_promotion_gates`, ledger I/O), `models/registry/experiment_registry.py`, `scripts/validate_release_gates.py`, `docs/GOVERNANCE_RELEASE_AND_EXPERIMENTS.MD`. **Done 2026-04-18.** | `orchestration/`, `models/registry/`, `scripts/`, `docs/`, `tests/` |
| FB-CAN-012 | CAN-DEL | A | CP | tech_debt | MEDIUM | Done | **Superseded doc removal:** deleted `docs/Specs/MASTER_SPEC_RISK_STATE_GAP.MD` (gap audit §5.1); added [`CANONICAL_DELETION_LOG.MD`](CANONICAL_DELETION_LOG.MD) + updated gap audit §5.1 and Specs index. **Done 2026-04-18.** | `docs/`, `docs/reports/` |
| FB-CAN-013 | CAN-TEST | B | CI | platform | HIGH | Done | **Canonical CI + tests:** `scripts/ci_canonical_contracts.sh` (release-gate fixture), `tests/test_canonical_contracts_smoke.py`, `tests/fixtures/canonical_release_candidate_live.json`, workflow step in `ci.yml`; `AGENTS.md` §9 + `TESTING_AND_CI.MD`. **Done 2026-04-18.** | `tests/`, `scripts/`, `.github/workflows/ci.yml`, `docs/` |
| FB-CAN-014 | CAN-CLEAN | B | OP | change | MEDIUM | Done | **Module map + pointers:** [`CANONICAL_MODULE_MAP.MD`](CANONICAL_MODULE_MAP.MD); README + `CANONICAL_SPEC_INDEX` + `Specs/README` + `QUEUE_SCHEMA`; clarified deletion log vs as-built Specs. **Done 2026-04-18.** | `README.md`, `docs/` |
| FB-CAN-015 | CAN-DATA | A | DP | feature | HIGH | Done | **Typed input snapshots:** `app/contracts/decision_snapshots.py`, `snapshot_builders.py`; `DecisionPipeline.step` merges canonical scalars + stores `forecast_diagnostics["canonical_boundary_input"]`; `run_step` passes `data_timestamp`. **Done 2026-04-18.** | `app/contracts/`, `decision_engine/`, tests, docs |
| FB-CAN-016 | CAN-NORM | A | DP | feature | HIGH | Done | **`canonical_normalize.py`:** `ret_*`→`return_*`, freshness/reliability/completeness, structural proxies; `enrich_bars_last_row` + tick path; `RiskState` fields; `apply_normalization_degradation`. **Done 2026-04-18.** | `data_plane/features/`, `decision_engine/`, `app/contracts/`, tests, docs |
| FB-CAN-017 | CAN-STR | A | FR | feature | MEDIUM | Done | **`CanonicalStructureOutput` + `structure_from_forecast_packet`:** pipeline attaches `forecast_diagnostics["canonical_structure"]`; trigger and auction take optional canonical structure (backward-compatible default). **Done 2026-04-18.** | `app/contracts/`, `decision_engine/`, tests, docs |
| FB-CAN-018 | CAN-CARRY | A | FR | feature | MEDIUM | Done | **`carry_sleeve/`:** eligibility + sizing from `domains.carry`; `RouteId.CARRY`; pipeline attaches `carry_sleeve` diagnostics; `RiskState.carry_sleeve_last`; Prometheus carry metrics; replay `decision_output_event` includes carry. **Done 2026-04-18.** | `carry_sleeve/`, `decision_engine/`, `risk_engine/`, `observability/`, `backtesting/`, tests |
| FB-CAN-019 | CAN-DEL-DEC | A | CP | tech_debt | HIGH | Done | Removed unused **`decision_engine/action_generator.py`** + **`tests/test_action_generator_routes.py`**; `decision_engine` package no longer exports `propose_action`. **Done 2026-04-18.** | [`CANONICAL_DELETION_LOG.MD`](CANONICAL_DELETION_LOG.MD) |
| FB-CAN-020 | CAN-DEL-RISK | A | CP | tech_debt | HIGH | Done | Removed unused **`risk_engine/equal_weight.py`** + **`tests/test_equal_weight.py`** (deferred FB-N3 stub, not on `RiskEngine` path). Updated **`docs/architecture/risk_precedence.md`** to match gates + canonical sizing + signing. **Done 2026-04-18.** | [`CANONICAL_DELETION_LOG.MD`](CANONICAL_DELETION_LOG.MD) |
| FB-CAN-021 | CAN-DEL-EXEC | A | CP | tech_debt | HIGH | Done | Removed duplicate **`get_execution_adapter`** alias in **`execution/router.py`** (use **`create_execution_adapter`** only). **`ExecutionService`** + guidance path unchanged. **Done 2026-04-18.** | [`CANONICAL_DELETION_LOG.MD`](CANONICAL_DELETION_LOG.MD) |
| FB-CAN-022 | CAN-DEL-CFG | A | CFG | tech_debt | MEDIUM | Done | Removed top-level YAML **`risk:`**; limits in **`apex_canonical.domains.risk_sizing`**; loader maps domain → flat `NM_RISK_*`; **`risk:`** present → **ValueError**. **Done 2026-04-18.** | [`CANONICAL_DELETION_LOG.MD`](CANONICAL_DELETION_LOG.MD) |
| FB-CAN-023 | CAN-DEL-DOC | B | OP | tech_debt | MEDIUM | Done | **Docs precedence + tombstones:** [`Specs/CANONICAL_PRECEDENCE.MD`](Specs/CANONICAL_PRECEDENCE.MD); tombstone notes on key Specs; Human Provided Specs README + gap audit §5.1; [`CANONICAL_DELETION_LOG.MD`](CANONICAL_DELETION_LOG.MD). **Done 2026-04-18.** | `docs/Specs/`, `docs/Human Provided Specs/`, `docs/reports/`, docs |
| FB-CAN-024 | CAN-DEL-TEST | B | CI | tech_debt | HIGH | Done | **Tests:** removed redundant `test_spec_pipeline_mode.py`; expanded `test_pipeline_forecast_packet.py` + `test_decision_risk.py` for `canonical_structure` and merged `RiskState`; noted in `TESTING_AND_CI.MD`. **Done 2026-04-18.** | `tests/`, `docs/Specs/TESTING_AND_CI.MD` |
| FB-CAN-025 | CAN-CI | B | CI | platform | MEDIUM | Done | **`ci_canonical_gates.sh`:** config YAML + `load_settings` bundle, forbidden legacy paths, replay event hash stability; **`ci_queue_consistency.py`** requires Open `CAN-*` rows under §2.7 anchor; CI workflow step. **Done 2026-04-18.** | `scripts/`, `.github/workflows/ci.yml`, `tests/test_ci_queue_consistency.py`, docs |
| FB-CAN-026 | CAN-REL | B | OP | platform | MEDIUM | Done | **`orchestration/release_evidence.py`:** `ReleaseEvidenceBundle`, fingerprint, canonical diff vs baseline YAML; **`scripts/build_release_evidence_bundle.py`**; **`GET /governance/release-evidence`**, **`POST /governance/release-evidence/diff`**; tests. **Done 2026-04-18.** | `orchestration/`, `control_plane/`, `scripts/`, `tests/`, `docs/` |
| FB-CAN-027 | CAN-EXP | B | DS | feature | MEDIUM | Done | **Experiment registry:** lifecycle transitions + required fields when not `draft`; `upsert`/`delete` sync `release_ledger.json` `linked_experiment_ids`; **`GET/POST/DELETE /governance/experiments`**; **`research/`** package re-export. **Done 2026-04-18.** | `research/`, `models/registry/`, `control_plane/`, `tests/` |
| FB-CAN-028 | CAN-MON-ALRT | B | OB | feature | MEDIUM | Done | **Prometheus:** `infra/prometheus/alerts/canonical_apex.yml` + `rule_files` in `prometheus.yml`; **Compose** mounts alerts; **Grafana** `tb-canonical-health.json`; **`GET /governance/monitoring`**; **`MONITORING_CANONICAL.MD`**. **Done 2026-04-18.** | `infra/`, `control_plane/`, `docs/`, `tests/` |
| FB-CAN-029 | CAN-RUNTIME | A | RT | feature | HIGH | Done | **`decision_engine/canonical_orchestrator.py`** — explicit post-forecast sequence (structure → state → trigger → auction → carry); `pipeline.step` delegates; `run_step` / `live_service` docstrings. **Done 2026-04-18.** | `decision_engine/`, `app/runtime/live_service.py` |
| FB-CAN-030 | CAN-REPLAY-DELTA | A | DS | feature | HIGH | Done | **`backtesting/live_replay_equivalence.py`**; **`scripts/ci_live_replay_equivalence.py`** + **`ci_canonical_gates.sh`**; **`EvidencePackage.live_replay_equivalence_*`** gates **logic**/**combined** live promotion; tests. **Done 2026-04-18.** | `backtesting/`, `orchestration/release_gating.py`, `scripts/`, `tests/`, `docs/` |
| FB-CAN-031 | CAN-TRIG-MISSED | A | FR | feature | HIGH | Done | **§8 missed-move** after strength; **`execution_too_degraded`** reason; **`RiskState.trigger_false_positive_memory`** EMA in **`merge_canonical_into_risk`**; auction **P** uses memory; **`safety_snapshot_event`**. **Done 2026-04-18.** | `decision_engine/`, `app/contracts/risk.py`, `backtesting/replay_events.py`, `tests/` |
| FB-CAN-032 | CAN-SIGNAL-FAM | A | CFG | feature | MEDIUM | Done | **`app/config/signal_confidence.py`**; **`default.yaml`** `signal_confidence` + `feature_families` + `enabled_feature_families`; **`build_decision_boundary_input`** applies per-family scores; **`ci_canonical_config_gates`**. **Done 2026-04-18.** | `app/config/`, `app/contracts/snapshot_builders.py`, `scripts/`, `tests/` |
| FB-CAN-033 | CAN-STATE-OVR | A | RT | feature | HIGH | Done | **`classify_hard_override`** + **`RiskState`** fields; **`canonical_orchestrator`** forces **NO_TRADE** + skips auction; **`pipeline.step`** passes **`feed_last_message_at`** / **`now_ref`**; replay **`safety_snapshot_event`** + **`tb_canonical_hard_override_*`** metrics. **Done 2026-04-18.** | `decision_engine/`, `observability/canonical_metrics.py`, `backtesting/replay_events.py`, `tests/` |
| FB-CAN-034 | CAN-AUCT-CORR | A | FR | feature | MEDIUM | Done | **D** split into **`D_corr`**, **`D_thesis`**, **`D_liq`**; **`AuctionResult.clustering_metadata`** (thesis buckets, liquidation cluster key, long/short correlation proxy); **`penalties`** expose subcomponents. **Done 2026-04-18.** | `decision_engine/auction_engine.py`, `app/contracts/auction.py`, `tests/` |
| FB-CAN-035 | CAN-EXEC-FEED | A | EX | feature | MEDIUM | Done | **`execution_feedback_memory.py`**: EMA trust/slippage/fill/latency/venue degradation; **`pipeline.step`** + replay; **`canonical_exec_quality_penalty`** → trigger + auction; live **`update_execution_feedback_memory`** after submit. **Done 2026-04-18.** | `data_plane/memory/`, `decision_engine/`, `backtesting/`, `app/runtime/`, `tests/` |
| FB-CAN-036 | CAN-DEC-REC | B | DP | feature | MEDIUM | Done | **`app/contracts/decision_record.py`** (`DecisionRecord`, `TradeIntentCanonical`, `NoTradeDecision`, `SuppressionEvent`, `SafetyOverrideEvent`); **`build_decision_record`** + **`RiskState.last_decision_record`**; **`RiskEngine`** `last_risk_block_codes`; **`GET /governance/decision-record`**; replay event payload. **Done 2026-04-18.** | `app/contracts/`, `decision_engine/`, `risk_engine/`, `control_plane/api.py`, `backtesting/`, `tests/` |
| FB-CAN-037 | CAN-FAULT | B | DS | feature | MEDIUM | Done | **Named fault profiles** (`orchestration/fault_injection_profiles.py`), **`ReplayRunContract.fault_injection_profile_id`**, extended **`apply_fault_injection`**, **`ReleaseEvidenceBundle`/`EvidencePackage` fault stress fields**, shadow/live **`fault_stress_evidence`** gate, **`scripts/run_fault_profile_replay.py`**. **Done 2026-04-18.** | `backtesting/`, `orchestration/`, `scripts/`, `tests/`, `docs/` |
| FB-CAN-038 | CAN-SHADOW | B | OP | platform | MEDIUM | Done | **Shadow replay comparison:** `orchestration/shadow_comparison.py`, `domains.shadow_comparison` in **`default.yaml`**, **`EvidencePackage.shadow_comparison_*`** + shadow/live gates, **`ReleaseEvidenceBundle.shadow_comparison_report`**, **`GET/POST /governance/shadow-comparison`**, **`scripts/compare_shadow_replays.py`**, store JSON. **Done 2026-04-18.** | `orchestration/`, `app/config/`, `models/registry/`, `control_plane/`, `scripts/`, `tests/`, `docs/` |
| FB-CAN-039 | CAN-MON-DRIFT | B | OB | feature | MEDIUM | Done | Implement calibration/drift monitoring and alert escalation policies for edge erosion/divergence. | `observability/`, `infra/`, `control_plane/`, tests/docs |
| FB-CAN-040 | CAN-DOC-GLOSS | B | OP | change | LOW | Done | Publish canonical glossary and old->new naming map to eliminate ambiguity during migration. | `docs/`, `README.md`, `.github/`, tests |
| FB-CAN-041 | CAN-STATE-REGIME | A | RT | feature | HIGH | Done | Implement full 5-class regime probability vector and transition confidence math. | `decision_engine/state_engine.py`, contracts/replay/tests |
| FB-CAN-042 | CAN-STATE-HEAT | A | RT | feature | MEDIUM | Done | Implement novelty/heat/reflexivity score stack and downstream propagation. | `decision_engine/state_engine.py`, trigger/risk/observability/tests |
| FB-CAN-043 | CAN-TRIG-STAGE | A | FR | feature | MEDIUM | Done | Implement explicit Setup/Pre-Trigger/Confirm stage records with latency metrics and reason codes. | `decision_engine/trigger_engine.py`, contracts/observability/replay/tests |
| FB-CAN-044 | CAN-AUCT-TOPN | A | FR | feature | MEDIUM | Done | Implement auction throughput bounds, top-N saturation controls, and deterministic tie-breakers. | `decision_engine/auction_engine.py`, contracts/observability/tests |
| FB-CAN-045 | CAN-RISK-SIZE | A | RT | feature | HIGH | Done | Implement canonical sizing formula stack (inertia/asymmetry/degradation/edge-budget terms). | `risk_engine/`, contracts/tests |
| FB-CAN-046 | CAN-RISK-THESIS | A | RT | feature | MEDIUM | Done | Implement thesis-aware concentration and overlap risk constraints with offense/defense posture handling. | `risk_engine/`, `decision_engine/auction_engine.py`, contracts/tests |
| FB-CAN-047 | CAN-EXEC-STYLE | A | EX | feature | HIGH | Done | Implement execution style policy (passive/aggressive/staggered/twap) with urgency + remaining-edge logic. | `execution/`, contracts/observability/tests |
| FB-CAN-048 | CAN-EXEC-PFILL | A | EX | feature | MEDIUM | Done | Implement partial-fill reconciliation and residual-intent handling logic for live and replay. | `execution/`, `backtesting/`, contracts/tests |
| FB-CAN-049 | CAN-DATA-STRUCT | A | DP | feature | MEDIUM | Done | Implement structural signal families (funding/OI/basis/liquidations/divergence) with confidence/freshness normalization. | `data_plane/ingest/`, `data_plane/features/`, contracts/tests |
| FB-CAN-050 | CAN-DATA-OPTIONS | A | DP | feature | MEDIUM | Done | Implement options-context and stablecoin-flow proxy feature families and monitoring availability rates. | `data_plane/features/`, `decision_engine/`, observability/config/tests |
| FB-CAN-051 | CAN-GOV-OBJ | B | OP | platform | MEDIUM | Done | Implement release-object model (config/logic/model-family/feature-family/combined). | `models/`, `orchestration/`, `control_plane/`, contracts/tests |
| FB-CAN-052 | CAN-GOV-ENV | B | OP | platform | MEDIUM | Done | Implement research/simulation/shadow/live stage-enforcement rules for promotion flow. | `orchestration/`, `scripts/`, `models/`, docs/tests |
| FB-CAN-053 | CAN-GOV-ROLL | B | OP | platform | LOW | Done | Implement rollback playbooks and rollback-target validation as promotion requirements. | `docs/`, `scripts/`, `orchestration/`, `control_plane/`, tests |
| FB-CAN-054 | CAN-EXP-METRICS | B | DS | feature | MEDIUM | Done | Enforce experiment predeclared metrics and failure-mode capture before promotion states. | `research/`, `models/`, `scripts/`, tests |
| FB-CAN-055 | CAN-REPLAY-MODES | B | DS | feature | MEDIUM | Done | Implement canonical replay modes and required event-family coverage checks. | `backtesting/`, contracts/scripts/tests |
| FB-CAN-056 | CAN-MON-DOMAINS | B | OB | feature | MEDIUM | Done | Implement monitoring-domain coverage checklist and CI assertions for required metric families. | `observability/`, `scripts/`, docs/tests |
| FB-CAN-057 | CAN-CONFIG-DIFF | B | CFG | platform | LOW | Done | Implement immutable config diff audit trail and semantic-change guardrails. | `app/config/`, `models/`, `control_plane/`, scripts/tests |
| FB-CAN-058 | CAN-DOC-TOMBS | B | OP | change | LOW | Done | Add tombstone index for removed modules/docs with canonical replacement pointers. | `docs/`, `README.md`, reports/tests |
| FB-CAN-059 | CAN-RUNTIME-CUT | A | RT | hardening | MEDIUM | Done | Add runtime cutover guard preventing unintended legacy+canonical dual-path activation. | `app/runtime/live_service.py`, `decision_engine/run_step.py`, config/tests |
| FB-CAN-060 | CAN-FINAL-PRUNE | B | CP | tech_debt | HIGH | Done | Final hard-prune of temporary migration shims and remaining old architecture remnants. | core modules, docs, tests |
| FB-CAN-061 | CAN-CONFIG-META | B | CFG | feature | MEDIUM | Done | Implement canonical config metadata fields and validation rules. | `app/config/`, `models/`, `scripts/`, tests |
| FB-CAN-062 | CAN-CONTRACT-COMMON | A | DP | feature | MEDIUM | Done | Enforce canonical contract common conventions (UTC/freshness/confidence typing and bounds). | `app/contracts/`, `decision_engine/`, `data_plane/`, tests |
| FB-CAN-063 | CAN-SUPPRESS-TAX | B | DP | feature | MEDIUM | Done | Implement stable suppression/safety reason taxonomy and machine-readable reason codes. | contracts + decision/risk/execution/observability/tests |
| FB-CAN-064 | CAN-CARRY-MON | B | OB | feature | LOW | Done | Implement carry-sleeve monitoring and alert families. | `carry_sleeve/`, observability/infra/control_plane/tests |
| FB-CAN-065 | CAN-GOV-MON | B | OB | feature | LOW | Done | Implement governance/operator health monitoring domain and alerts. | observability/orchestration/models/scripts/tests |
| FB-CAN-066 | CAN-EVIDENCE-SCHEMA | B | OP | platform | MEDIUM | Done | Define canonical evidence-package schema for promotion decisions and gate completeness. | `models/`, `orchestration/`, `scripts/`, docs/tests |
| FB-CAN-067 | CAN-CONSTANTS | B | CP | hardening | LOW | Done | Add guardrails against hidden magic constants in canonical code paths. | scripts + core canonical modules/tests |
| FB-CAN-068 | CAN-REPLAY-SEED | B | DS | feature | LOW | Done | Enforce deterministic seed policy and replay/sim reproducibility metadata. | `backtesting/`, scripts/models/tests/docs |
| FB-CAN-069 | CAN-PROBATION | B | OP | platform | LOW | Done | Implement post-release probation-window monitors and automatic abort triggers. | orchestration/observability/models/scripts/tests |
| FB-CAN-070 | CAN-COVERAGE-MAP | B | OP | change | MEDIUM | Done | Publish canonical spec section-to-queue coverage matrix and closure checklist gate. | docs/reports/scripts/QUEUE/tests |
| FB-CAN-071 | CAN-CONFIG-SNAP | A | DP | feature | MEDIUM | Done | Implement canonical Service Configuration Snapshot contract and traceability bindings. | contracts/decision/replay/config/tests |
| FB-CAN-072 | CAN-MON-LAG | B | OB | feature | MEDIUM | Done | **`tb_canonical_lag_seconds`** per-domain lag + Grafana + alerts (**FB-CAN-072**). **Done 2026-04-18.** | observability/runtime/decision/execution/infra/tests |
| FB-CAN-073 | CAN-STATE-WEEKEND | A | RT | feature | LOW | Done | Session mode + throttle on **`CanonicalStateOutput`**; **`RiskState`** occupancy; sizing + trigger tightening; **`tb_canonical_session_mode`** + transition gauge (**FB-CAN-073**). **Done 2026-04-18.** | state/risk/observability/tests |
| FB-CAN-074 | CAN-STATE-EXRISK | A | RT | feature | MEDIUM | Done | Integrate exchange-risk flags and data-integrity warnings into degradation/override logic. | state/data/risk/contracts/tests |
| FB-CAN-075 | CAN-EXEC-LAT | A | EX | feature | LOW | Open | Implement execution latency/reliability quality terms in execution confidence. | execution/config/replay/observability/tests |
| FB-CAN-076 | CAN-EDGE-MON | B | OB | feature | MEDIUM | Open | Implement edge-budget proxy monitoring and alert thresholds. | decision/execution/observability/infra/tests |
| FB-CAN-077 | CAN-BINDING-IMM | B | DS | hardening | LOW | Open | Implement immutable run-binding guard for config/logic/dataset/seed identifiers. | contracts/replay/orchestration/models/tests |
| FB-CAN-078 | CAN-FINAL-AUDIT | B | OP | platform | MEDIUM | Open | Add automated canonical acceptance audit as final migration completion gate. | scripts/reports/tests/QUEUE |

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
| FB-N1 | deferred | D | XX | LOW | Done | Multi-exchange execution — **first slice Done 2026-04-13** (**FB-N1-S1** in [§5.5](#55-complete-archive-one-row-per-id)). **Closed for now 2026-04-17** — full multi-venue scope remains future; reopen via [**§8**](#8-how-to-add-an-entry) or reprioritize. [`DEFERRED_ROADMAP.MD`](DEFERRED_ROADMAP.MD) §FB-N1 |
| FB-N3 | deferred | D | XX | LOW | Done | Portfolio optimization — **first slice Done 2026-04-13** (**FB-N3-S1** in [§5.5](#55-complete-archive-one-row-per-id)). **Closed for now 2026-04-17** — full optimizer scope remains future; reopen via [**§8**](#8-how-to-add-an-entry) or reprioritize. [`DEFERRED_ROADMAP.MD`](DEFERRED_ROADMAP.MD) §FB-N3 |

*Rows retained for traceability. Full multi-venue / optimizer work remains **out of scope** until reprioritized; see [`DEFERRED_ROADMAP.MD`](DEFERRED_ROADMAP.MD).*

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
| HG-01 | Hardening | Done | Kraken-only market data: CI blocks Alpaca *data* imports outside paper adapter (`ci_spec_compliance.sh`) |
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
| FB-N1 | 2026-04-17 | Deferred epic **closed for now** — first slice **FB-N1-S1** (registry + `GET /status` → `execution_adapters`); full multi-venue execution → [`DEFERRED_ROADMAP.MD`](DEFERRED_ROADMAP.MD) §FB-N1 |
| FB-N3 | 2026-04-17 | Deferred epic **closed for now** — first slice **FB-N3-S1** (`risk_engine/equal_weight`); full portfolio optimization → [`DEFERRED_ROADMAP.MD`](DEFERRED_ROADMAP.MD) §FB-N3 |
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
| FB-N1 | deferred | 2026-04-17 | Epic **closed for now** — first slice **FB-N1-S1**; full multi-venue execution → [`DEFERRED_ROADMAP.MD`](DEFERRED_ROADMAP.MD) §FB-N1 |
| FB-N1-S1 | execution | 2026-04-13 | **`execution/adapter_registry`**, **`GET /status` → `execution_adapters`**, [`DEFERRED_ROADMAP.MD`](DEFERRED_ROADMAP.MD) |
| FB-N3 | deferred | 2026-04-17 | Epic **closed for now** — first slice **FB-N3-S1**; full portfolio optimization → [`DEFERRED_ROADMAP.MD`](DEFERRED_ROADMAP.MD) §FB-N3 |
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
3. Edit the script if titles/bodies drift from the **queue system** (`QUEUE_ARCHIVE.MD` / `QUEUE_STACK.csv`); prefer **updating those** for operational truth, then mirroring to the script if you still use bulk create.

**Note:** GitHub issues are optional; this markdown file remains the **local** source of truth for fix tracking.

---

## 8. How to add an entry

1. Pick a **Kind** from [§1.1](#11-kind-work-type).
2. Assign **HIGH** / **MEDIUM** / **LOW** (maps to P0–P3).  
3. Set **Phase** (**A** / **B** / **C** / **D**) and **Category** per [§1.3](#13-priority-phase-category-status).  
4. For multi-row epics (e.g. **FB-AP-P0**), assign a **Batch** code ([§1.3](#13-priority-phase-category-status)) and add **Affected files** (pipe-separated primary paths).
5. Add a row in [§2](#2-open-queue) (roadmap), [§3](#3-resolved-fixes-il) (fixes), or note completed gates in [§4](#4-completed-compliance-gates-hg). Use the next **FB-** or **HG-** id; for large epics prefer **-01, -02** sub-ids.  
6. Link **code paths** or tickets.  
7. **Stack file:** If the item is **Open** (or you close one), update **[`QUEUE_STACK.csv`](QUEUE_STACK.csv)** — keep **`stack_order`** so the **next** **`Open`** row is the smallest number; use **`_QUEUE_EMPTY_`** / **`empty`** when **§2** has no **Open** rows ([§0](#0-next-task-stack-machine-readable)).

**Empty open queue:** If **§2** has **no** rows with **Status = Open** (including no **Open** **HIGH** rows), agents asked to take the “next **HIGH** open item” have **nothing to implement** until you **add** a new row in **§2**, or **reprioritize** closed deferred work (e.g. reopen **FB-N1** / **FB-N3** with an agreed scope — see [`DEFERRED_ROADMAP.MD`](DEFERRED_ROADMAP.MD)).

**Cursor Agent:** For a structured checklist aligned with §1–§2, use the project skill **Add to Queue** at [`.cursor/skills/add-to-queue/SKILL.md`](../.cursor/skills/add-to-queue/SKILL.md) (invoke with `/add-to-queue` or attach `@add-to-queue` when available).

---
