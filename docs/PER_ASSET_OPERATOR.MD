# Per-asset model registry (operator)

This document describes the **per-asset model manifest** introduced for **FB-AP-001** / **FB-AP-002**. Full per-asset UI and init pipelines are tracked in [`QUEUE.MD`](QUEUE.MD) **§2.2 (FB-AP-P0)**.

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

## Control plane API

Requires **`X-API-Key`** when **`NM_CONTROL_PLANE_API_KEY`** is set (same as other mutating routes).

| Method | Path | Description |
|--------|------|-------------|
| GET | `/assets/models` | List symbols with manifests. |
| GET | `/assets/models/{symbol}` | Get one manifest. |
| PUT | `/assets/models/{symbol}` | Create/replace manifest; body **`canonical_symbol`** must equal `{symbol}`. |
| DELETE | `/assets/models/{symbol}` | Remove manifest file. |
| POST | `/assets/init/{symbol}` | Start per-asset init job (one global runner); returns `job_id`. |
| GET | `/assets/init/jobs/{job_id}` | Poll job status and per-step detail. |

**GET `/status`** includes **`asset_model_registry`**: `registry_dir`, `initialized_symbols`.

## Asset init — Kraken bootstrap (FB-AP-007)

The first pipeline step pulls OHLCV from **Kraken public REST** via the same path as training data (`orchestration/real_data_bars.fetch_symbol_bars_sync`). Configure the bootstrap window in **`default.yaml`** under **`init_pipeline`** (`bootstrap_lookback_days`, `bootstrap_granularity_seconds`) or override with **`NM_ASSET_INIT_BOOTSTRAP_LOOKBACK_DAYS`** and **`NM_ASSET_INIT_BOOTSTRAP_GRANULARITY_SECONDS`** (optional; if unset, bar size defaults to at least 60 seconds for Kraken OHLC). The job step records **display symbol**, **Kraken REST pair**, and **wsname** for traceability.

## Asset init — clean and validate (FB-AP-008)

The second step (`validate`) runs **`data_plane.bootstrap_bars.validate_and_clean_init_bootstrap_bars`** on the fetched frame: required columns match training/storage OHLCV, timestamps are normalized to **UTC**, duplicate **timestamps** are collapsed (last row wins), **gaps** vs the bootstrap bar size are counted (logged in job detail; the step does not fail on gaps), and structurally invalid rows (e.g. `high < low`, negative volume, open/close outside `[low, high]`) are **dropped** — see the module docstring for the full policy. The job step **`detail`** includes `meta={...}` with row counts and gap stats.

## Related

- [`RUNBOOKS.MD`](RUNBOOKS.MD) — execution and preflight.
- [`QUEUE.MD`](QUEUE.MD) **FB-AP-P0** — remaining per-asset UI, init pipeline, guards in `DecisionPipeline`.
