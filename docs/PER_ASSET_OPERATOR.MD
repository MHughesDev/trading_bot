# Per-asset model registry (operator)

This document describes the **per-asset model manifest** introduced for **FB-AP-001** / **FB-AP-002**. Full per-asset UI and init pipelines are tracked in [`QUEUE.MD`](QUEUE.MD) **§2.2 (FB-AP-P0)**.

## Operator user accounts (FB-UX-001)

Multi-tenant groundwork: **SQLite** user table at **`NM_AUTH_USERS_DB_PATH`** (default **`data/users.sqlite`**, gitignored). Passwords stored as **Argon2** hashes only.

- **`POST /auth/register`** — JSON **`{"email":"...","password":"..."}`** (password min length 8). Returns **`id`**, normalized **`email`**, **`created_at`**. **409** if email already exists; **422** on validation errors.
- **`GET /status` → `user_store`** — **`db_path`**, **`user_count`**, **`session_auth_enabled`**, **`active_session_rows`**.

## Sessions and mutating API (FB-UX-002)

Optional **server-side sessions** in the same SQLite file (`operator_sessions` table). Enable with **`NM_AUTH_SESSION_ENABLED=true`** (default **false** — automation continues to use **`X-API-Key`** only when **`NM_CONTROL_PLANE_API_KEY`** is unset).

- **`POST /auth/login`** — body **`{email,password}`**; sets HTTP-only cookie **`NM_AUTH_SESSION_COOKIE_NAME`** (default **`tb_operator_session`**), **`max_age`** = **`NM_AUTH_SESSION_TTL_SECONDS`**. **`Secure`** and **`SameSite`** via **`NM_AUTH_SESSION_COOKIE_SECURE`** / **`NM_AUTH_SESSION_COOKIE_SAMESITE`** (use **`Secure=true`** behind HTTPS).
- **`POST /auth/logout`** — revokes the session row and clears the cookie.
- **`GET /auth/me`** — current user (**401** if no valid session).

**Authorization for mutating routes** (params, lifecycle, manifests, etc.): if **`NM_CONTROL_PLANE_API_KEY`** is set, **`X-API-Key`** must match **or** (when sessions are enabled) a valid session cookie. If the API key is **not** configured, mutating routes require a valid session cookie when **`NM_AUTH_SESSION_ENABLED=true`**; when sessions are **off** and no key is set, behavior stays open (development only).

## Dashboard health strip (FB-UX-008)

On **Home**, a row above holdings/PnL summarizes **`GET /status`**: **preflight** (severity, issues/warnings), **production_preflight** (signing / unsigned / venue credentials flags), **execution profile** (legacy API off → default mode + per-asset note). Implemented in **`control_plane/health_strip.py`**.

## In-app toasts (FB-UX-013)

Optional **`st.toast`** notifications (default **on**): set **`NM_STREAMLIT_TOASTS_ENABLED=false`** to disable. **Asset** page: init complete/fail, watch start, **Stop** (flatten submitted / no position / API error). **Dashboard**: init monitor toasts when a followed job reaches **succeeded**/**failed**; nightly scheduler toasts when **`last_run_finished_utc`** advances (success summary, RL-skipped hint, or last error).

## Nightly scheduler panel (FB-UX-012)

The **Dashboard** shows **Nightly scheduler** status from **`GET /scheduler/nightly`**: enabled flag, **interval** (seconds), **last tick** (job start), **last run finished**, **next run after** (approximate — first wake after start, then after each job completes), **last error**, and an expander with the **last report** JSON (same object as **`run_nightly_training_job`**). **`GET /status` → `app_scheduler`** includes the timing fields without the full report (use the detail route for JSON).

## Dashboard CSV export (FB-UX-011)

On **Home**, **Download positions (CSV)** builds UTF-8 CSV from the same **`GET /portfolio/positions`** JSON used for the holdings table (one row per position, plus adapter/mode columns; if the venue call fails, one row records **`ok=false`** and the error). **Download P&L summary (CSV)** exports **`GET /pnl/summary`** for the selected timeframe (flattened window, realized/unrealized, ledger path). No server-side CSV route — client-side only.

## Init pipeline monitor (FB-UX-009)

After **Initialize** on an **Asset** page, **Home** shows an **Init pipeline** panel: **symbol**, **job_id**, **`GET /assets/init/jobs/{id}`** status, and last completed step. It polls until the job reaches **`succeeded`** or **`failed`**. Use **Stop following** to clear the panel (or wait for a terminal state and dismiss). Implemented in **`control_plane/init_monitor.py`**.

## Watchlist / favorites (FB-UX-014)

On the **Asset** page for a symbol, **Pin ★** adds it to a **session** watchlist (up to **24** symbols); **Remove ★** drops it. The sidebar lists starred symbol links to **`/Asset?symbol=…`**. Cleared when the Streamlit session ends — not persisted server-side.

## Streamlit routes (FB-AP-027)

- **Dashboard** — `streamlit run control_plane/Home.py` (main entry).
- **Sign in / Register** — **`pages/0_Login.py`** (**FB-UX-003**): email + password; **`POST /auth/login`** stores server session and keeps an opaque token in **`st.session_state`** for API **`Cookie`** headers; register uses **`POST /auth/register`**. Requires **`NM_AUTH_SESSION_ENABLED=true`** on the control plane for sign-in. **Sign out** on the Dashboard sidebar calls **`POST /auth/logout`**.
- **Route guard (FB-UX-004)** — Optional **`NM_STREAMLIT_ROUTE_GUARD_ENABLED=true`** (default **false**): Dashboard, Asset, and sidebar pages call **`require_streamlit_app_access()`** — redirect to **`0_Login`** if no valid session (**`GET /auth/me`**). Set **`NM_CONTROL_PLANE_API_KEY`** in the Streamlit environment to bypass (operator automation / scripts). Deep links to a page without a prior Streamlit session still require signing in once in that browser session.
- **Account (FB-UX-005)** — Sidebar **Account** link (**`:material/settings:`**) → **`pages/7_Account.py`**. Shared chrome: **`control_plane/streamlit_chrome.py`** **`render_app_sidebar()`**.
- **Venue keys (FB-UX-006)** — **`GET`/`PUT /auth/venue-credentials`** (session auth): per-user **Alpaca** / **Coinbase** API key + secret stored encrypted (Fernet) in SQLite; **`NM_AUTH_VENUE_CREDENTIALS_MASTER_SECRET`** required. UI shows **masked** values only (`****` + last 4 chars). **Kraken** remains deployment **`NM_*`** only.
- **Per-user data + execution (FB-UX-007)** — **`NM_MULTI_TENANT_DATA_SCOPING=true`**: control plane resolves session → **`user_id`** and stores operator state under **`data/users/<id>/...`** (mirrors `data/` layout: manifests, lifecycle, execution mode, PnL ledger, trade markers, init artifacts). Existing top-level **`data/`** files are **not** migrated automatically — copy or re-init per user. **`NM_MULTI_TENANT_EXECUTION_CREDENTIALS=true`**: **`/portfolio/positions`**, PnL summary unrealized path, and **Stop flatten** use **merged** Alpaca/Coinbase secrets from **`/auth/venue-credentials`** when set (paper → Alpaca, live → Coinbase). Kraken / QuestDB unchanged (shared).
- **Asset page** — multipage route **`/Asset`** (`control_plane/pages/Asset.py`): optional **`?symbol=<canonical_symbol>`** deep link; sidebar links from Home include configured **`GET /status`** symbols.
- **Active assets (FB-AP-033)** — The shared sidebar (**`render_app_sidebar`**) lists symbols whose effective lifecycle is **`active`** (from **`GET /status` → `asset_lifecycle.states`**), with links to **`/Asset?symbol=…`**. Shown on Dashboard and Asset page.

## Asset page — model / manifest card (FB-UX-010)

Below lifecycle and execution mode, the Asset page shows a read-only **Model / manifest** table sourced from **`GET /assets/models/{symbol}`**: **last trained** timestamps (forecaster / RL), **forecaster** and **policy** artifact paths, and **runtime instance id** when set. With **`NM_MULTI_TENANT_DATA_SCOPING`**, the registry paths resolve per user (same API; see **FB-UX-007**).

## Asset page lifecycle controls (FB-AP-031)

Top-right **Initialize** / **Start** / **Stop** follows **`GET /assets/lifecycle/{symbol}`**:

| State | Button | API |
|-------|--------|-----|
| `uninitialized` | **Initialize** | **`POST /assets/init/{symbol}`** then poll job until **`succeeded`** / **`failed`** (spinner during pipeline). |
| `initialized_not_active` | **Start** | **`POST /assets/lifecycle/{symbol}/start`** |
| `active` | **Stop** | **`POST /assets/lifecycle/{symbol}/stop`** — **market** flatten via venue (**FB-AP-032**): fetches position for that symbol, submits **SELL** (long) or **BUY** (short) for full size; then **`active` → `initialized_not_active`**. If flatten cannot run (**502**), lifecycle stays **active**. |

Requires **`NM_CONTROL_PLANE_API_KEY`** on mutating routes when the API enforces it (same as other control-plane writes).

## Asset page OHLC chart (FB-AP-028)

On **`/Asset`** with a symbol, the page loads **`GET /assets/chart/bars`** for the selected interval preset (second through month). **Week** and **month** (**FB-AP-017**): when **1s** canonical rows exist in QuestDB, the client builds **daily** bars from them, when **`NM_TRAINING_DATA_GRANULARITY_SECONDS`** is greater than **1** and less than **86400**, charts aggregate to that bar width before daily rollup (same geometry as the training campaign), then roll up to week/month. If **1s** data is missing, the UI falls back to **stored daily** bars only. Shared Polars OHLC rules live in **`data_plane/bars/derived_intervals.py`**. Install **`pip install -e ".[dashboard]"`** for Plotly candlesticks.

## Asset page trade markers (FB-AP-029)

Overlays **`GET /assets/chart/trade-markers`** (same time window as the chart) on the candlestick figure: **green** ▲ buy, **red** ▼ sell, hover shows quantity and **`intent_submit`** metadata. Requires append-only **`data/trade_markers.jsonl`** populated by the live loop when orders submit (**FB-AP-025**).

## Per-asset execution mode (FB-AP-030)

Paper vs live routing is **per symbol** for orders submitted through **`ExecutionService`**: sidecar JSON under **`data/asset_execution_mode/<symbol>.json`** (override directory with **`NM_ASSET_EXECUTION_MODE_DIR`**). If no sidecar exists, **`NM_EXECUTION_MODE`** / app settings apply.

- **GET `/assets/execution-mode/{symbol}`** — effective mode, optional **`override`**, and **`default_execution_mode`** from settings.
- **PUT `/assets/execution-mode/{symbol}`** — body **`{"execution_mode":"paper"|"live"}`** (mutating key when configured).
- **DELETE `/assets/execution-mode/{symbol}`** — remove override (use default again).

**Asset page** — dropdown + **Apply** (uses the same API). **`GET /status`** includes **`asset_execution_mode`** (`sidecar_dir`, `overrides` list). **`NM_EXECUTION_ADAPTER`** (e.g. stub) still wins for the whole process when set — used for tests/dry-run.

## Purpose

Each **initialized** tradable symbol can have a **manifest** that lists **forecaster** and **RL / policy** artifact paths scoped to that symbol only. The canonical binding field is **`canonical_symbol`**: it must match the symbol in the API path and runtime request.

## Storage

- **Default directory:** `data/asset_model_registry/manifests/` (override with **`NM_ASSET_MODEL_REGISTRY_DIR`**).
- **File per symbol:** `<canonical_symbol>.json` (e.g. `BTC-USD.json`).
- Writes use **atomic replace** (temp file + `os.replace`).

## JSON schema (v1)

| Field | Type | Notes |
|-------|------|--------|
| `schema_version` | string | Default `"1"`. |
| `canonical_symbol` | string | Required; must match URL path when using the API. |
| `forecaster_weights_path` | string? | NPZ or weights path. |
| `forecaster_conformal_state_path` | string? | Conformal JSON. |
| `forecaster_config_path` | string? | Optional config. |
| `forecaster_torch_path` | string? | PyTorch bundle (`NM_MODELS_FORECASTER_TORCH_PATH` style). |
| `policy_mlp_path` | string? | Policy NPZ. |
| `policy_checkpoint_path` | string? | RL checkpoint. |
| `runtime_instance_id` | string? | Optional instance id. |
| `forecaster_last_trained_at` | string? | ISO-8601. |
| `rl_last_trained_at` | string? | ISO-8601. |

Pydantic model: `app.contracts.asset_model_manifest.AssetModelManifest`.

## Runtime binding (FB-AP-003 / FB-AP-004)

`DecisionPipeline` resolves forecaster and policy weights **per tick** via `resolve_serving_paths(symbol, settings)` in `decision_engine/pipeline.py`:

- **Single-symbol** `market_data_symbols` (length 1): global **`NM_MODELS_*`** paths apply as before; an optional per-asset manifest **overrides** those fields when set.
- **Multi-symbol** and global **`NM_MODELS_*`** point at **existing model files**: those globals are **not** used for a symbol without a manifest (the tick abstains from proposals; packet diagnostics may include `binding_abstain`). Use per-asset manifests or a single-symbol config for shared global weights.
- With a manifest in multi-symbol mode, **only manifest paths** apply for that symbol (no cross-symbol env fallback).

## Per-asset lifecycle (FB-AP-005)

States: **`uninitialized`** (no manifest), **`initialized_not_active`** (manifest present, watch not started), **`active`** (operator started watch — live loop wiring is **FB-AP-038**).

- **Storage:** `data/asset_lifecycle_state/<symbol>.json` — override with **`NM_ASSET_LIFECYCLE_STATE_DIR`**.
- **Effective state:** If a manifest exists but no state file → **`initialized_not_active`**. If the manifest is removed, the lifecycle file for that symbol is dropped and the API reports **`uninitialized`**.
- **Successful init** (**`POST /assets/init/{symbol}`** job **succeeded**) sets **`initialized_not_active`**.
- **Stop** here only updates lifecycle; **flatten** and watch teardown are **FB-AP-032**.

## Control plane API

Requires **`X-API-Key`** when **`NM_CONTROL_PLANE_API_KEY`** is set (same as other mutating routes).

| Method | Path | Description |
|--------|------|-------------|
| GET | `/assets/models` | List symbols with manifests. |
| GET | `/assets/models/{symbol}` | Get one manifest. |
| PUT | `/assets/models/{symbol}` | Create/replace manifest; body **`canonical_symbol`** must equal `{symbol}`. |
| DELETE | `/assets/models/{symbol}` | Remove manifest file (also clears lifecycle state for that symbol). |
| POST | `/assets/init/{symbol}` | Start per-asset init job (one global runner); returns `job_id`. |
| GET | `/assets/init/jobs/{job_id}` | Poll job status and per-step detail. |
| GET | `/assets/lifecycle/{symbol}` | Current **`lifecycle_state`**. |
| GET | `/assets/execution-mode/{symbol}` | Effective paper/live for this symbol + optional persisted override (**FB-AP-030**). |
| PUT | `/assets/execution-mode/{symbol}` | Persist **`execution_mode`** (`paper` or `live`) for routing this symbol’s orders. |
| DELETE | `/assets/execution-mode/{symbol}` | Drop sidecar; use application default again. |
| POST | `/assets/lifecycle/{symbol}/start` | **`initialized_not_active` → `active`** (requires manifest). |
| POST | `/assets/lifecycle/{symbol}/stop` | **Market flatten** (if non-flat) then **`active` → `initialized_not_active`**; response includes **`flatten`** (acks / skip reason). **502** if venue fetch or submit fails — state unchanged. |
| GET | `/assets/chart/bars` | **Chart OHLCV** (FB-AP-024): **`symbol`** (required), **`start`**, **`end`** (UTC range), optional **`interval_seconds`** (defaults to **`NM_MARKET_DATA_BAR_INTERVAL_SECONDS`**), **`limit`**. Returns rows from QuestDB **`canonical_bars`** for that symbol only — no multi-symbol series. |
| GET | `/assets/chart/trade-markers` | **Buy/sell markers** (FB-AP-025): **`symbol`** (required), **`start`**, **`end`**, optional **`limit`**. Reads **`data/trade_markers.jsonl`** (append-only; populated when the live loop **successfully submits** an order — **`source=intent_submit`**). |

**GET `/status`** includes **`asset_model_registry`**: `registry_dir`, `initialized_symbols`; **`asset_lifecycle`**: `state_dir`, `states`; **`asset_execution_mode`**: `default_execution_mode`, `sidecar_dir`, `overrides`; and **`app_scheduler`** (optional nightly training thread while the control plane process is up — **FB-AP-035**, see **`RUNBOOKS.MD`**).

## App-wide execution profile API (legacy, FB-AP-040)

**Default:** **`GET /system/execution-profile`** and **`POST /system/execution-profile`** are **disabled** (HTTP **410**). Use **`PUT /assets/execution-mode/{symbol}`** for paper/live per symbol. Set **`NM_EXECUTION_PROFILE_LEGACY_API=true`** only to restore the old app-wide intent + `data/execution_profile.json` flow.

## Global system power (legacy, FB-AP-039)

**Default:** global **OFF** switch is **removed** — **`NM_SYSTEM_POWER_LEGACY_ENABLED=false`**: `run_decision_tick` and nightly training are **not** blocked by `data/system_power.json`. Use **per-asset** **Stop** on the Asset page. Operators who need the old behavior set **`NM_SYSTEM_POWER_LEGACY_ENABLED=true`** (then **`POST /system/power`** and the JSON file apply again).

## Live loop — active watch (FB-AP-038)

When **`NM_LIVE_WATCH_LIFECYCLE_GATE=true`**, the **`live_service`** loop runs **`run_decision_tick`** (forecaster + risk + order path) **only** for symbols whose effective lifecycle is **`active`**. Ingestion (WS ticks, rollers, optional QuestDB bars) continues for all configured **`NM_MARKET_DATA_SYMBOLS`**. Optional **`NM_LIVE_DECISION_MIN_INTERVAL_SECONDS`** enforces a minimum gap between decision ticks **per symbol** (wall clock, monotonic). Defaults **false** / **0** — enable after operators use **Start** on assets to watch.

## Asset init pipeline (FB-AP-006+)

**POST `/assets/init/{symbol}`** runs a background sequence: **Kraken REST** bootstrap history (**FB-AP-007**), **validate** (**FB-AP-008**), **features** (**FB-AP-009**), **forecaster_train** (**FB-AP-010** — `forecaster_torch.pt` under `<run_dir>/forecaster/`; skipped if `torch` missing), **rl_init** (**FB-AP-011** — `MultiBranchMLPPolicy` saved to `<run_dir>/policy/policy_mlp.npz`), **register** (**FB-AP-012** — writes/updates per-asset **`AssetModelManifest`** with those paths under **`NM_ASSET_MODEL_REGISTRY_DIR`**). Returns **409** if another init job is already running.

## Alpaca tradable universe (FB-AP-020)

**Paper venue metadata only** — not Kraken market data. Snapshots **tradable** Alpaca **crypto** assets (active) into **`NM_ALPACA_UNIVERSE_DB_PATH`** (default **`data/alpaca_universe.sqlite`**, gitignored). Requires **`NM_ALPACA_API_KEY`** / **`NM_ALPACA_API_SECRET`** and **`pip install -e ".[alpaca]"`**.

- **`GET /universe/alpaca`** — paginated rows (`limit`, `offset`, optional `q` filter on symbol/name); metadata only.
- **`POST /universe/alpaca/sync`** — on-demand refresh (mutating operator: API key or session when enabled).
- Optional background sync: **`NM_ALPACA_UNIVERSE_SYNC_ENABLED=true`**, **`NM_ALPACA_UNIVERSE_SYNC_INTERVAL_SECONDS`** (default **86400**).
- **`GET /status` → `alpaca_universe`** — row count, last sync time, scheduler tick / last error.

## Coinbase tradable universe (FB-AP-021)

**Live venue metadata only** — not Kraken market data. Snapshots **SPOT** products from Coinbase Advanced Trade **`GET /products`** (tradable: not **`trading_disabled`** / **`is_disabled`**) into **`NM_COINBASE_UNIVERSE_DB_PATH`** (default **`data/coinbase_universe.sqlite`**, gitignored). Requires **`NM_COINBASE_API_KEY`** / **`NM_COINBASE_API_SECRET`** (CDP JWT).

- **`GET /universe/coinbase`** — paginated rows (`limit`, `offset`, optional `q`).
- **`POST /universe/coinbase/sync`** — on-demand refresh (mutating operator).
- Optional: **`NM_COINBASE_UNIVERSE_SYNC_ENABLED`**, **`NM_COINBASE_UNIVERSE_SYNC_INTERVAL_SECONDS`**.
- **`GET /status` → `coinbase_universe`**.

## Platform-supported cross-venue set (FB-AP-022)

**Search and eligibility hints only** — not Kraken pair resolution or market-data routing.

- **Rule version 1 (default):** **`intersection`** — canonical pair (e.g. `BTC-USD`) is *platform-supported* only if it appears in **both** the Alpaca tradable-crypto snapshot **and** the Coinbase SPOT snapshot (same string: Alpaca `canonical_symbol` ≡ Coinbase `product_id`).
- **Optional:** **`union`** — symbol appears on **either** venue (`NM_PLATFORM_SUPPORTED_UNIVERSE_MODE=union` or YAML `universe.platform_supported_mode: union`). Useful for operator visibility; default remains intersection so paper **and** live routing can both target the pair.

- **`GET /universe/platform-supported`** — paginated canonical symbols (`limit`, `offset`, optional `q` on symbol / Alpaca name / Coinbase base).
- **`GET /status` → `platform_supported_universe`** — `rule_version`, `mode`, `symbol_count` (no symbol list).

## Universe search (FB-AP-023)

**Metadata only** — same FB-AP-022 symbol set as **`GET /universe/platform-supported`**, with Alpaca / Coinbase fields per row for UI search (no OHLC or bar series).

- **`GET /universe/search`** — paginated `rows` with `canonical_symbol`, `paper_tradable` / `live_tradable` (presence in each snapshot under the current intersection or union rule), `alpaca_symbol`, `alpaca_name`, `coinbase_product_id`, `coinbase_base_name`, `coinbase_quote_name`. Query params: `limit`, `offset`, optional `q`.

## Related

- [`ADR_CANONICAL_BAR_STORAGE.MD`](ADR_CANONICAL_BAR_STORAGE.MD) — canonical bar storage decision (FB-AP-013).
- **FB-AP-014** — logical schema: bucket-start UTC `timestamp`, `interval_seconds`, OHLCV; Parquet `bars_clean.parquet` column order in `data_plane.storage.canonical_bars`.
- **FB-AP-015** — dedupe key ``(symbol, timestamp, interval_seconds)``; last-write-wins in Polars merge helpers; QuestDB `insert_bar` is idempotent (delete-then-insert).
- **FB-AP-016 (live loop)** — Kraken WS messages are normalized to ticks; **`RollingBars`** rolls OHLCV by **`NM_MARKET_DATA_BAR_INTERVAL_SECONDS`** (default 1s). **Persisted** rows are **closed buckets only** (last-write-wins per FB-AP-015). Enable with **`NM_QUESTDB_PERSIST_CANONICAL_BARS=true`** (requires QuestDB connection settings). Pair strings from Kraken (e.g. `XBT/USD`) map to canonical symbols (`BTC-USD`) for rollers and storage.
- **FB-AP-018 (startup)** — With **`NM_QUESTDB_STARTUP_GAP_DETECTION=true`** and QuestDB reachable, the live loop compares **`max(ts)`** in **`canonical_bars`** (per manifest symbol, same **`interval_seconds`** as live bars) to the **last closed** UTC bucket; logs **`canonical_bar_gap`** when behind or empty (**FB-AP-019** fills gaps).
- **FB-AP-019 (startup backfill)** — With **`NM_QUESTDB_STARTUP_KRAKEN_BACKFILL=true`**, after gap detection the loop fetches missing Kraken REST OHLC for each gap, validates with the same cleaner as init bootstrap, **`insert_bar`** into QuestDB (idempotent), and writes **`canonical_through_ts`** to **`data/canonical_bar_watermarks/<symbol>.json`** (override **`NM_CANONICAL_BAR_WATERMARK_DIR`**). Requires QuestDB connection (opens even if traces/canonical persist flags are off). **`questdb.backfill_max_lookback_days`** (default **14**) limits how far back REST runs when storage is empty.
- [`RUNBOOKS.MD`](RUNBOOKS.MD) — execution and preflight.
- [`QUEUE.MD`](QUEUE.MD) **FB-AP-P0** — remaining per-asset UI, init pipeline, guards in `DecisionPipeline`.
