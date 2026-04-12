# Per-asset model registry (operator)

This document describes the **per-asset model manifest** introduced for **FB-AP-001** / **FB-AP-002**. Full per-asset UI and init pipelines are tracked in [`QUEUE.MD`](QUEUE.MD) **§2.2 (FB-AP-P0)**.

## Streamlit routes (FB-AP-027)

- **Dashboard** — `streamlit run control_plane/Home.py` (main entry).
- **Asset page** — multipage route **`/Asset`** (`control_plane/pages/Asset.py`): optional **`?symbol=<canonical_symbol>`** deep link; sidebar links from Home include configured **`GET /status`** symbols.

## Asset page OHLC chart (FB-AP-028)

On **`/Asset`** with a symbol, the page loads **`GET /assets/chart/bars`** for the selected interval preset (second through month). **Week** and **month** aggregate from **daily** (`interval_seconds=86400`) bars in QuestDB. Install **`pip install -e ".[dashboard]"`** for Plotly candlesticks.

## Asset page trade markers (FB-AP-029)

Overlays **`GET /assets/chart/trade-markers`** (same time window as the chart) on the candlestick figure: **green** ▲ buy, **red** ▼ sell, hover shows quantity and **`intent_submit`** metadata. Requires append-only **`data/trade_markers.jsonl`** populated by the live loop when orders submit (**FB-AP-025**).

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
| POST | `/assets/lifecycle/{symbol}/start` | **`initialized_not_active` → `active`** (requires manifest). |
| POST | `/assets/lifecycle/{symbol}/stop` | **`active` → `initialized_not_active`**. |
| GET | `/assets/chart/bars` | **Chart OHLCV** (FB-AP-024): **`symbol`** (required), **`start`**, **`end`** (UTC range), optional **`interval_seconds`** (defaults to **`NM_MARKET_DATA_BAR_INTERVAL_SECONDS`**), **`limit`**. Returns rows from QuestDB **`canonical_bars`** for that symbol only — no multi-symbol series. |
| GET | `/assets/chart/trade-markers` | **Buy/sell markers** (FB-AP-025): **`symbol`** (required), **`start`**, **`end`**, optional **`limit`**. Reads **`data/trade_markers.jsonl`** (append-only; populated when the live loop **successfully submits** an order — **`source=intent_submit`**). |

**GET `/status`** includes **`asset_model_registry`**: `registry_dir`, `initialized_symbols`; and **`asset_lifecycle`**: `state_dir`, `states`.

## Asset init pipeline (FB-AP-006+)

**POST `/assets/init/{symbol}`** runs a background sequence: **Kraken REST** bootstrap history (**FB-AP-007**), **validate** (**FB-AP-008**), **features** (**FB-AP-009**), **forecaster_train** (**FB-AP-010** — `forecaster_torch.pt` under `<run_dir>/forecaster/`; skipped if `torch` missing), **rl_init** (**FB-AP-011** — `MultiBranchMLPPolicy` saved to `<run_dir>/policy/policy_mlp.npz`), **register** (**FB-AP-012** — writes/updates per-asset **`AssetModelManifest`** with those paths under **`NM_ASSET_MODEL_REGISTRY_DIR`**). Returns **409** if another init job is already running.

## Related

- [`ADR_CANONICAL_BAR_STORAGE.MD`](ADR_CANONICAL_BAR_STORAGE.MD) — canonical bar storage decision (FB-AP-013).
- **FB-AP-014** — logical schema: bucket-start UTC `timestamp`, `interval_seconds`, OHLCV; Parquet `bars_clean.parquet` column order in `data_plane.storage.canonical_bars`.
- **FB-AP-015** — dedupe key ``(symbol, timestamp, interval_seconds)``; last-write-wins in Polars merge helpers; QuestDB `insert_bar` is idempotent (delete-then-insert).
- **FB-AP-016 (live loop)** — Kraken WS messages are normalized to ticks; **`RollingBars`** rolls OHLCV by **`NM_MARKET_DATA_BAR_INTERVAL_SECONDS`** (default 1s). **Persisted** rows are **closed buckets only** (last-write-wins per FB-AP-015). Enable with **`NM_QUESTDB_PERSIST_CANONICAL_BARS=true`** (requires QuestDB connection settings). Pair strings from Kraken (e.g. `XBT/USD`) map to canonical symbols (`BTC-USD`) for rollers and storage.
- **FB-AP-018 (startup)** — With **`NM_QUESTDB_STARTUP_GAP_DETECTION=true`** and QuestDB reachable, the live loop compares **`max(ts)`** in **`canonical_bars`** (per manifest symbol, same **`interval_seconds`** as live bars) to the **last closed** UTC bucket; logs **`canonical_bar_gap`** when behind or empty (**FB-AP-019** fills gaps).
- **FB-AP-019 (startup backfill)** — With **`NM_QUESTDB_STARTUP_KRAKEN_BACKFILL=true`**, after gap detection the loop fetches missing Kraken REST OHLC for each gap, validates with the same cleaner as init bootstrap, **`insert_bar`** into QuestDB (idempotent), and writes **`canonical_through_ts`** to **`data/canonical_bar_watermarks/<symbol>.json`** (override **`NM_CANONICAL_BAR_WATERMARK_DIR`**). Requires QuestDB connection (opens even if traces/canonical persist flags are off). **`questdb.backfill_max_lookback_days`** (default **14**) limits how far back REST runs when storage is empty.
- [`RUNBOOKS.MD`](RUNBOOKS.MD) — execution and preflight.
- [`QUEUE.MD`](QUEUE.MD) **FB-AP-P0** — remaining per-asset UI, init pipeline, guards in `DecisionPipeline`.
