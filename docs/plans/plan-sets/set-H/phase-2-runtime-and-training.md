# Phase 2 — Hybrid Runtime & Training

**Completion: 0% (0 / 12 tasks)**

**Goal:** Replace Phase 1's stub driver with **real machine learning**. Stand up
the Python sidecars (`apps/model-trainer`, `apps/model-inference`), define the
Rust↔Python **orchestration contract**, build the **dataset/feature
materialization** path in Rust, and port the legacy `forecaster_model/` +
`nightly_retrain.py` logic into the trainer. After this phase, "Train" produces a
real, versioned, artifact-backed model with live loss/metric curves.

**Depends on:** Phases 0–1. **Blocks:** Phase 3 (eval needs real artifacts),
Phase 4 (inference needs the inference sidecar).

---

## Design — orchestration contract

Rust is the orchestrator; Python is a **stateless compute worker**. The boundary
is a small HTTP+NATS contract. Large bytes (datasets, artifacts) move via the
shared `ArtifactStore` URIs (Phase 0), never over the wire.

**Dispatch — Rust → trainer (`POST {trainer}/train`):**
```jsonc
{
  "run_id": "uuid", "model_id": "mdl_…", "model_kind": "forecaster",
  "framework": "xgboost", "runtime": "python",
  "definition": { /* frozen ModelDefinition v1.0 */ },
  "dataset_uri": "s3://…/datasets/ds_…/v3.parquet",
  "dataset_hash": "sha256:…",
  "output_prefix": "s3://…/models/mdl_…/v4/",
  "progress": { "nats_subject": "models.run.<run_id>.progress" }
}
```

**Progress — trainer → Rust (NATS publish, per phase/epoch):**
```jsonc
{ "run_id": "uuid", "phase": "fitting", "progress": 62.5,
  "metric": { "step": 240, "train_loss": 0.0142, "val_loss": 0.0161, "val_auc": 0.71 } }
```
`ModelManager` consumes these (NATS subscription), updates the job's atomic
`progress`/`phase`, appends the metric tick to the run's series, persists, and
rebroadcasts on the `models.jobs` WS lane (Phase 1) → the live training console.

**Completion — trainer → Rust (HTTP response / terminal NATS):**
```jsonc
{ "status": "succeeded", "artifact_uri": "s3://…/models/mdl_…/v4/model.bin",
  "sha256": "…", "size_bytes": 1048576,
  "metrics": { "val_auc": 0.71, "rmse": 0.013, "n_train": 84000, "n_val": 21000 },
  "framework_version": "xgboost==2.0.3" }
```
Rust then writes the `model_versions` row (`Evaluating` status), `model_artifacts`
row (uri+sha256+size), and a `version_created` event.

**Inference — Rust → inference sidecar (`POST {inference}/predict`):**
```jsonc
{ "model_id": "mdl_…", "version": 4, "model_kind": "forecaster",
  "artifact_uri": "s3://…/v4/model.bin",
  "instances": [ { "instrument_id": "BTC-USD", "features": { "ema_7": …, "rsi_14": … } } ] }
→ { "predictions": [ { "direction": "up", "magnitude": "0.0042", "confidence": 0.68, "horizon": "1h" } ] }
```
For `external_llm_adapter`, the inference sidecar proxies the provider and returns
`{ text, tokens, latency_ms, cost_usd, trace_id }`.

**Design rationale.** This mirrors ADR-0001's satellite-process model (collectors,
embedder): Python sidecars are satellites that fail/restart independently without
touching the Rust core's authority. The contract is framework-agnostic, so adding
LightGBM or a torch time-series model is a trainer-internal change, invisible to
Rust.

---

## Tasks

### ☐ H-2.1 `apps/model-trainer` service skeleton (FastAPI) — L
New Python app (its own `pyproject.toml`, Dockerfile, `docker-compose` service).
Endpoints: `POST /train`, `GET /health`, `GET /capabilities` (frameworks +
versions). NATS + ArtifactStore (S3/MinIO/FS) clients. Job runs in a worker task;
progress published to the run's NATS subject.
**Acceptance:** `docker compose up model-trainer` healthchecks green; `/capabilities`
lists `xgboost,lightgbm,sklearn,torch`; a `POST /train` against a fixture dataset
publishes ≥2 progress messages and returns a `succeeded` body with a real artifact
in the store.

### ☐ H-2.2 Trainer: framework adapters — L
Pluggable `train(definition, dataset, emit_progress) -> (artifact, metrics)` per
`framework`: `xgboost`, `lightgbm`, `sklearn` (classical), `torch` (NN +
time-series). Each emits progress (boosting round / epoch) and a standard metrics
dict. Artifact serialization standardized (native format + a `metadata.json`).
**Acceptance:** each adapter trains on the fixture and produces a loadable
artifact; metrics dict has the documented keys; `torch` adapter emits per-epoch
`train_loss/val_loss`.

### ☐ H-2.3 Port legacy `forecaster_model/` + `nightly_retrain.py` — L
Migrate the legacy torch forecaster and the train/eval/log pipeline (see
`docs/specs/MODELS_AND_ORCHESTRATION.MD`) into the trainer as the reference
`forecaster` implementation. Drop the Prefect/MLflow coupling — the registry +
NATS now play those roles.
**Acceptance:** the ported forecaster trains via `POST /train` and reaches
comparable holdout metrics to the legacy pipeline on the same data window
(documented in the task's results note).

### ☐ H-2.4 `apps/model-inference` service — L
FastAPI scoring service: `POST /predict`, `POST /predict/llm` (adapter proxy),
`GET /health`. Loads artifacts by URI with an LRU cache keyed by
`(model_id, version, sha256)`; verifies hash on load. Returns the canonical
prediction envelope.
**Acceptance:** loads a Phase-2.2 artifact and returns valid predictions;
hash mismatch → 409 (never serves a tampered artifact); cold vs warm load latency
reported.

### ☐ H-2.5 Inference sidecar: external LLM adapter proxy — M
Implement `external_llm_adapter` inference: proxy to `provider` (`ollama`,
`openai`, …) per the definition's `adapter` block, normalize the response to
`{ text, tokens, latency_ms, cost_usd, trace_id }`, compute cost from
`cost_per_1k_tokens`. Reuse the `semantic` crate's provider env conventions.
**Acceptance:** a registered Ollama/Gemma adapter returns a normalized completion
with token + latency + cost fields; provider error surfaces as a structured error,
not a 500 crash.

### ☐ H-2.6 Rust sidecar client (`model-registry/src/sidecar.rs`) — M
Typed `reqwest` client: `dispatch_train(req) -> RunHandle`, `predict(req) ->
Vec<Forecast>`, `predict_llm(req)`. Endpoints from config
(`MODEL_TRAINER_URL`, `MODEL_INFERENCE_URL`). Timeouts, retries with backoff,
and circuit-break on repeated failure (satellite may be down).
**Acceptance:** client round-trips against the live sidecars; trainer-down yields a
clean `RunStatus::Failed` with a diagnostic, not a hang.

### ☐ H-2.7 NATS progress bridge — M
`ModelManager` subscribes to `models.run.*.progress`, maps frames onto the owning
`Job`'s atomic counters + metric series, persists throttled (like the backtest
persist cadence), and rebroadcasts to the `models.jobs` WS lane.
**Acceptance:** a real training run drives a smooth 0→100 progress with a live
`val_loss` series visible over both poll and WS; dropped/duplicate NATS frames do
not corrupt the series (idempotent by `step`).

### ☐ H-2.8 Feature sets on the `features` crate — M
A **feature set** is a named, versioned list of features the `features` crate
already computes (EMA/RSI/rolling windows) plus an extensibility hook. `POST
/api/datasets`-adjacent endpoints register feature sets; the resolver maps a
`feature_set_ref` → concrete feature columns.
**Acceptance:** `fs_core_ohlcv_v3` resolves to a deterministic, ordered feature
list; an unknown feature name is rejected at registration, not at train time.

### ☐ H-2.9 Dataset materialization (Rust) — L
Given a `feature_set_ref` + instrument(s) + window + `label_spec`, Rust builds an
immutable **dataset version**: pull canonical bars (ClickHouse), compute features
via the `features` crate (same builders live + replay, ADR-0008), generate labels
(e.g. forward return), write **Parquet** to the artifact store (repo already uses
Arrow/Parquet), and record `dataset_versions` with a `content_hash`.
**Acceptance:** identical inputs → identical `content_hash` (determinism test);
the Parquet loads in the trainer; row counts + available-time ordering verified;
no look-ahead leakage (labels strictly future of features).

### ☐ H-2.10 Wire real training into `ModelManager::drive` — M
Replace the Phase-1 stub: `drive(Train)` = materialize-or-resolve dataset →
`sidecar.dispatch_train` → consume progress → on success write version/artifact
rows + move model to `Evaluating`; on failure mark `Failed` with the trainer
diagnostic. Concurrency bounded by the existing semaphore.
**Acceptance:** end-to-end `POST …/train` yields a real `model_versions` row with
an artifact + metrics; the model lands in `Evaluating`; a forced trainer error
lands `Failed` with a readable reason.

### ☐ H-2.11 Test Lab inference wiring — M
`POST …/versions/{v}/test` calls the inference sidecar for trainable kinds and the
LLM proxy for adapters; persists the call as a trace (Phase 0 ClickHouse) and
returns the prediction + latency/cost. Heavy kinds may return a `job_id`.
**Acceptance:** testing a trained forecaster returns a `Forecast` with latency;
testing an adapter returns text+tokens+cost; every test writes one trace row.

### ☐ H-2.12 Packaging, config, `.env.example`, compose — S
Add `MODEL_TRAINER_URL`, `MODEL_INFERENCE_URL`, `ARTIFACT_STORE`, MinIO creds to
`.env.example`; add `model-trainer`, `model-inference`, and (dev) `minio` services
to `docker-compose.yml`; document GPU-optional torch.
**Acceptance:** `docker compose up` brings up platform + both sidecars + MinIO;
a fresh clone can train a model end-to-end following the README delta.

---

## Phase 2 exit criteria

- Training a model produces a real artifact + metrics via a Python sidecar, with
  live progress over poll and WS.
- Datasets are deterministic, hashed, leak-free Parquet materializations on the
  `features` crate.
- Inference (trainable kinds + LLM adapters) works through the inference sidecar
  with traces recorded.
- The legacy forecaster is ported and reproduces its prior metrics.
