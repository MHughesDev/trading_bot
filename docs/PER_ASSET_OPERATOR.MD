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

## Runtime binding (FB-AP-003 / FB-AP-004)

`DecisionPipeline` resolves forecaster and policy artifact paths **per tick** via `resolve_serving_paths(symbol, settings)` in `decision_engine/pipeline.py`:

- **Single-symbol** `market_data_symbols` (length 1): global **`NM_MODELS_*`** paths apply as today; an optional per-asset manifest **overrides** those fields when present.
- **Multi-symbol** and global **`NM_MODELS_*`** point at **existing model files**: those globals are **not** applied to any symbol without a manifest file for that symbol (the tick **abstains** from trading proposals; `ForecastPacket.forecast_diagnostics` may include `binding_abstain` / `binding_reason`). Add manifests under **`NM_ASSET_MODEL_REGISTRY_DIR`** (or use a single-symbol deployment for shared global weights).
- With a manifest in multi-symbol mode, **only manifest paths** are used for that symbol’s weights (no silent fallback to another symbol’s global env).

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

## Asset init pipeline (FB-AP-006+)

**POST `/assets/init/{symbol}`** runs a background sequence: **Kraken REST** bootstrap history (**FB-AP-007**), **validate** (**FB-AP-008**), **features** (**FB-AP-009**), **forecaster_train** (**FB-AP-010** — `forecaster_torch.pt` under `<run_dir>/forecaster/`; skipped if `torch` missing), **rl_init** (**FB-AP-011** — `MultiBranchMLPPolicy` saved to `<run_dir>/policy/policy_mlp.npz`), **register** (**FB-AP-012** — writes/updates per-asset **`AssetModelManifest`** with those paths under **`NM_ASSET_MODEL_REGISTRY_DIR`**). Returns **409** if another init job is already running.

## Related

- [`RUNBOOKS.MD`](RUNBOOKS.MD) — execution and preflight.
- [`QUEUE.MD`](QUEUE.MD) **FB-AP-P0** — remaining per-asset UI, init pipeline, guards in `DecisionPipeline`.
