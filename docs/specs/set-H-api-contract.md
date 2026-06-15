# Set-H API Contract — AI Model Studio

**Status:** Frozen v1.0 (Phase 1)
**Date:** 2026-06-15

This document freezes the HTTP API surface for the AI Model Studio feature set (Set-H, Phases 0–1).
Fields marked `[stub]` return placeholder values until the indicated Phase is complete.

## Authentication

All endpoints require `Authorization: Bearer <token>` (same as all other API endpoints).
`user_id` is extracted from the bearer token via `BearerToken::user_id()`.

---

## Model CRUD

### GET /api/models

List all models owned by the authenticated user.

**Query params:** `kind`, `status`, `asset_class`, `limit` (default 50, max 200)

**Response:**
```json
{ "models": [...], "total": 3 }
```

### POST /api/models

Create a new model definition.

**Request:**
```json
{
  "display_name": "BTC Return Forecaster v2",
  "description": "XGBoost 1h return forecast for BTC-USD",
  "definition": {
    "schema_version": "1.0",
    "model_kind": "forecaster",
    "framework": "xgboost",
    "runtime": "python",
    "asset_class": "crypto_spot_cex",
    "target": { "field": "return", "horizon": "1h", "transform": "logret" },
    "feature_set_ref": "fs_core_ohlcv_v3",
    "hyperparameters": { "max_depth": 6, "n_estimators": 400 },
    "inference": { "min_confidence": 0.0, "calibrate": true }
  }
}
```

**Response (201):**
```json
{ "model_id": "mdl_abc123..." }
```

**Error (422):** `{ "error": "invalid_definition", "errors": [{ "path": "target", "message": "required for trainable model kinds" }] }`

### GET /api/models/:id

Get a single model record.

**Response:** `ModelRecord` JSON object.

### PATCH /api/models/:id

Rename a model or update its description.

**Request:** `{ "display_name": "New Name", "description": "Optional" }`

**Response:** `{ "ok": true }`

### DELETE /api/models/:id

Delete a model (only allowed when `status == "draft"`).

### POST /api/models/:id/archive

Archive a model (idempotent, status transitions to "archived").

---

## Training Runs

### POST /api/models/:id/train

Start a training run. Returns immediately; job runs async.

**Request:**
```json
{
  "dataset_version_id": "uuid-or-null",
  "hyperparameter_overrides": { "max_depth": 8 }
}
```

**Response (201):** `{ "run_id": "uuid" }`

**Note:** `external_llm_adapter` models reject this endpoint (D-3).

### GET /api/models/:id/runs

List training runs for a model (in-memory only; Phase 2 adds DB hydration).

**Response:** `{ "runs": [...ModelRunSnapshot] }`

### GET /api/models/:id/runs/:run_id

Get a single run snapshot.

**Response:** `ModelRunSnapshot`

`ModelRunSnapshot` fields: `run_id`, `model_id`, `kind`, `status`, `progress` (0-100), `phase`, `error`, `metrics`, `created_at`, `started_at`, `finished_at`.

### POST /api/models/:id/runs/:run_id/cancel

Request cancellation of a running job.

---

## Versions

### GET /api/models/:id/versions

List registered versions.

**Response:** `{ "versions": [{ "version": 1, "status": "draft", "metrics": {...}, "created_at": "..." }] }`

### POST /api/models/:id/versions

Register a new version from an externally produced artifact.

**Request:**
```json
{
  "artifact_uri": "file:///path/to/model.pkl",
  "artifact_hash": "abc123...",
  "notes": "trained on Q1 2026 data"
}
```

**Response (201):** `{ "version": 1 }`

### POST /api/models/:id/versions/:v/evaluate

Start an evaluation run for a specific version.

**Response (201):** `{ "eval_id": "uuid" }`

### POST /api/models/:id/versions/:v/promote

Promote a version to the `production` alias.

### POST /api/models/:id/versions/:v/test

`[stub until Phase 4]` Synchronous inference test against a version.

**Response:**
```json
{
  "model_id": "...",
  "version": 1,
  "status": "stub",
  "note": "real inference available in Phase 4",
  "input_echo": {...},
  "output": { "direction": "flat", "magnitude": "0", "confidence": 0.5 }
}
```

---

## Evaluations

### GET /api/models/:id/evaluations

List evaluation runs for a model.

**Response:** `{ "evaluations": [...ModelRunSnapshot] }`

---

## Aliases

### GET /api/models/:id/aliases

Get alias -> version mapping.

**Response:** `{ "production": 3, "candidate": 4 }`

### POST /api/models/:id/aliases/:alias/rollback

Roll back an alias to the previous version.

---

## Deployments

### GET /api/models/:id/deployments

List deployments.

**Response:** `{ "deployments": [{ "deployment_id": "dep_...", "version": 1, "environment": "paper", "status": "active", "traffic_pct": 100 }] }`

### POST /api/models/:id/deployments

Create a deployment record.

**Request:** `{ "version": 1, "environment": "paper" }`

**Response (201):** `{ "deployment_id": "dep_..." }`

---

## Test Lab

### GET /api/models/:id/test-cases

List saved test cases.

### POST /api/models/:id/test-cases

Add a test case.

**Request:** `{ "name": "BTC pump scenario", "input": {...}, "expected": {...} }`

**Response (201):** `{ "case_id": "uuid" }`

---

## Lineage & Observability

### GET /api/models/:id/lineage

Get lineage graph. `[full graph stub until Phase 2]`

### GET /api/models/:id/traces

`[stub until Phase 4]` Returns empty traces array.

### GET /api/models/:id/used-by

`[stub until Phase 4]` Returns empty strategies array.

---

## Asset-scoped model lookup

### GET /assets/models/:symbol

List non-archived models for the asset class resolved from `:symbol`.

**Response:** `{ "symbol": "BTC-USD", "models": [...ModelRecord] }`

---

## ModelRecord shape

```json
{
  "model_id": "mdl_abc123",
  "slug": "btc-return-forecaster-v2",
  "display_name": "BTC Return Forecaster v2",
  "description": "...",
  "model_kind": "forecaster",
  "asset_class": "crypto_spot_cex",
  "definition": { "schema_version": "1.0", ... },
  "status": "draft",
  "created_by": "uuid",
  "created_at": "2026-06-15T00:00:00Z",
  "updated_at": "2026-06-15T00:00:00Z"
}
```

---

## WebSocket lane

Training and evaluation progress is published to the `models.jobs` lane (private, per-user).

Payload shape:
```json
{
  "run_id": "uuid",
  "model_id": "mdl_...",
  "run_kind": "train",
  "status": "running",
  "progress": 35.0,
  "phase": "fitting"
}
```

---

## Stub phases

| Field/endpoint | Available in |
|---|---|
| `GET /api/models/:id/traces` | Phase 4 |
| `GET /api/models/:id/used-by` | Phase 4 |
| `POST /api/models/:id/versions/:v/test` (real inference) | Phase 4 |
| Lineage dataset version refs | Phase 2 |
| Real training sidecar | Phase 2 |
| Real evaluation sidecar | Phase 2 |
