# Phase 1 — Registry & API

**Completion: 0% (0 / 13 tasks)**

**Goal:** Make the foundation *live*. A new `model-registry` crate owns model
identity, versions, aliases, and the async job engine (mirroring
`BacktestManager`); a Postgres storage adapter persists it; the `api` crate
exposes the complete REST surface plus a WebSocket lane for live job progress;
`AppState` carries a `ModelManager` handle. After this phase the entire Studio
API is callable end-to-end with training/inference **stubbed** (real ML is
Phase 2) — enough for the frontend (Phase 5) to build against a frozen contract.

**Depends on:** Phase 0. **Blocks:** Phases 2–5 share this contract.

---

## Design notes

- **`crates/model-registry` (new)** is the orchestrator, structured exactly like
  `crates/backtest`: a `ModelManager { jobs: RwLock<HashMap<Uuid, Arc<Job>>>, pg,
  artifacts, sidecar_client, .. }` with `create/list/get/cancel`, `tokio::spawn`
  background drivers, atomic progress counters, and `persist()` to Postgres. In
  Phase 1 the driver is a stub loop (like the backtest manager skeleton); Phase 2
  swaps the body for real sidecar calls without changing the public API.
- **Handlers** follow the house pattern verbatim
  (`crates/api/src/routes/backtests.rs`): `State(AppState)` + `BearerToken` +
  `Json<Req>` → `impl IntoResponse`, ad-hoc `(StatusCode, Json(json!{…}))` errors,
  user-scoped by `token.user_id()`.
- **The old stub** `GET /assets/models/{symbol}` is re-pointed to the registry
  (returns models whose `asset_class` matches the symbol's instrument), closing
  the loop the frontend already calls.

---

## REST surface (end state)

All under `/api/models`. `{id}` = `model_id`, `{v}` = version number.

| Method & path | Purpose |
|---|---|
| `POST /api/models` | Create model from a `ModelDefinition` (validated, `Draft`) |
| `GET /api/models` | List/search (filter `kind`, `status`, `asset_class`, `q`) |
| `GET /api/models/{id}` | Model detail (definition, latest metrics, aliases, used-by count) |
| `PATCH /api/models/{id}` | Rename / edit description / edit draft definition (writes `model_events`) |
| `DELETE /api/models/{id}` | Archive (soft) or delete (draft-only, hard) |
| `GET /api/models/{id}/versions` | Version list (immutable) |
| `GET /api/models/{id}/versions/{v}` | Version detail (metrics, scorecard, artifact, lineage refs) |
| `POST /api/models/{id}/versions` | Register a version directly (import / external adapter) |
| `POST /api/models/{id}/train` | Start a **training run** → returns `run_id` |
| `GET /api/models/{id}/runs` | List training runs |
| `GET /api/models/{id}/runs/{run_id}` | Run snapshot (status, `progress`, `phase`) — **poll** |
| `POST /api/models/{id}/runs/{run_id}/cancel` | Cancel a run |
| `POST /api/models/{id}/versions/{v}/test` | Test Lab inference (sync for fast kinds; `job_id` for heavy) |
| `POST /api/models/{id}/versions/{v}/evaluate` | Start an **evaluation run** → `eval_id` |
| `GET /api/models/{id}/evaluations` | Eval list |
| `GET /api/models/{id}/evaluations/{eval_id}` | Eval detail (scorecard, regression report, samples) |
| `POST /api/models/{id}/versions/{v}/promote` | **Promotion gate** → move `production` alias |
| `POST /api/models/{id}/aliases/{alias}/rollback` | Move an alias to the previous version |
| `GET /api/models/{id}/aliases` | Current alias → version map |
| `GET /api/models/{id}/deployments` · `POST …/deployments` | Deployment control |
| `GET /api/models/{id}/used-by` | Strategies referencing this model (derived) |
| `GET /api/models/{id}/lineage` | Graph payload (datasets→runs→versions→deployments→strategies) |
| `GET /api/models/{id}/traces` | Recent inference traces (from ClickHouse) |
| `GET /api/models/{id}/test-cases` · `POST …/test-cases` | Saved Test Lab cases |
| `GET /api/datasets` · `POST /api/datasets` · `…/versions` | Dataset registry (Phase 2 fills behaviour) |

WebSocket: lane **`models.jobs`**, `instrument` = `model_id`, frames carry
`{ run_kind: "train"|"eval", run_id, status, progress, phase, metric_tick? }`.

---

## Tasks

### ☐ H-1.1 Scaffold `crates/model-registry` — M
New workspace crate (add to root `Cargo.toml` members). Deps: `domain`,
`storage`, `tokio`, `serde`, `uuid`, `chrono`, `anyhow`, `tracing`, `reqwest`
(sidecar client, Phase 2). Public modules: `manager`, `job`, `types`, `sidecar`.
**Acceptance:** `cargo build -p model-registry` succeeds; empty `ModelManager::new(pg, artifacts)` constructs.

### ☐ H-1.2 `ModelManager` + `Job` skeleton (mirror BacktestManager) — L
Port the structure of `crates/backtest/src/manager.rs`: `jobs` map, `run_permits`
semaphore, `create()` spawns a driver, `list/get/cancel`, `snapshot()` →
`RunSnapshot { run_id, model_id, kind, status, progress, phase, error, metrics,
started_at, finished_at }`, atomic `progress`/`phase` read live in `get()`.
Driver body is a **stub** that advances phases on a timer and writes a fake
metric series (so Phase 5 sees real-looking progress).
**Acceptance:** create→poll shows `Queued→Running→Succeeded` with monotonic
progress; `cancel` flips to `Cancelled`; snapshots persist across a restart
(hydrated from Postgres).

### ☐ H-1.3 Storage adapter `storage/postgres/models.rs` — L
sqlx functions over the Phase-0 tables: `create_model`, `get_model`,
`list_models(filter)`, `rename_model`, `archive_model`, `insert_version`,
`get_version`, `list_versions`, `set_alias`, `get_aliases`, `record_event`,
`upsert_training_run`, `upsert_evaluation_run`, `insert_deployment`. Append-only
guard: reject `UPDATE` of a non-`Draft` version's `config_json`.
**Acceptance:** unit/integration tests (behind the DB-test gate) cover create→
rename (event written)→version insert→alias set→archive; `list_models` honours
all filters; rename of a non-existent model returns `NotFound`, not a panic.

### ☐ H-1.4 Extend `AppState` with `models: Arc<ModelManager>` — S
Add the field (`crates/api/src/state.rs`), construct in
`apps/platform/src/main.rs` alongside `backtest_manager`, pass into
`AppState::new`. **Acceptance:** platform boots with the manager; existing routes
unaffected; `cargo build -p platform` green.

### ☐ H-1.5 Route module `api/routes/models.rs` + registration — M
Create the handler module and register every row of the surface table in
`crates/api/src/routes/mod.rs` (`get/post/patch/delete` combinators like the
strategies/backtests blocks). DTOs (`CreateModelRequest`, `ModelResponse`,
`TrainRequest`, `RunSnapshotResponse`, `PromoteRequest`, …) as serde structs in
the module. **Acceptance:** every path returns a well-formed JSON shape (stubs ok);
unknown `{id}` → 404; malformed body → 422 with `{error, ...}`.

### ☐ H-1.6 Create / list / detail handlers — M
`POST /api/models` validates the definition via `domain::model_def::validate`,
persists `Draft`, writes a `created` event, returns `{model_id, slug}`. `GET`
list + detail read the adapter; detail includes alias map + `used_by_count`.
**Acceptance:** invalid definition → 422 with field errors (same shape as
`create_strategy`); slug auto-derived from `display_name`, collision-suffixed.

### ☐ H-1.7 Rename + edit handler (audited) — S
`PATCH /api/models/{id}` updates `display_name`/`description`/draft definition and
writes a `renamed` event `{from, to, reason?, actor, at}`. Immutable `slug` never
changes. **Acceptance:** rename returns 200; `GET …/events` (or detail) shows the
rename with old+new name; renaming does not alter `slug` or break alias/version refs.

### ☐ H-1.8 Train / runs handlers — M
`POST …/train` builds a `TrainRequest` (dataset_version or inline window +
overrides), calls `ModelManager::create(kind=Train)`, returns `run_id`. `GET
…/runs` + `…/runs/{run_id}` return snapshots; `cancel` cancels.
**Acceptance:** training a `Draft` model moves it to `Training`; run is pollable;
cancel works; training an `external_llm_adapter` is rejected (422 — adapters
don't train, D-3).

### ☐ H-1.9 Versions + register handler — S
`GET …/versions[/{v}]`; `POST …/versions` registers an externally-produced
artifact (URI + hash + metrics) or an adapter version. **Acceptance:** registered
version appears immutable; importing with a missing artifact hash → 422.

### ☐ H-1.10 Test / evaluate / promote / rollback handlers (contract-level) — M
Wire the endpoints to manager methods (`evaluate`→eval job; `promote`→gate +
`set_alias('production', v)`; `rollback`→alias to prior). In Phase 1 the gate
checks are stubbed-true except artifact-hash presence; Phase 3 fills real checks.
**Acceptance:** promote moves the alias and writes a `promoted` event; rollback
restores the previous alias target and writes `rolled_back`; both are user-scoped.

### ☐ H-1.11 WebSocket lane `models.jobs` — M
Register the lane in the ui-gateway transport/shaping
(`crates/ui-gateway/src/{transport.rs,shaping.rs}`) and bridge `ModelManager`
progress to it (broadcast on phase change / metric tick). Polling remains the
fallback; the lane powers the live training console.
**Acceptance:** a subscribed client receives `Subscribed` then `Frame`s with
increasing `progress` during a run, and a terminal frame on completion;
unsubscribe + heartbeat behave like other lanes.

### ☐ H-1.12 Re-point `GET /assets/models/{symbol}` to the registry — S
Replace the empty stub in `crates/api/src/routes/asset_lifecycle.rs` with a
registry query returning models whose `asset_class` matches the symbol's
instrument, including the `production` alias version. This is what the existing
frontend `assetApi.models(symbol)` already calls.
**Acceptance:** the endpoint returns real models for a known symbol; empty list
(not 500) for symbols with no models.

### ☐ H-1.13 OpenAPI/contract doc + typed examples — S
Document the surface in `docs/specs/` (or a `set-H/API.md` companion) with
request/response examples for every endpoint, marked as the **frozen Phase-1
contract** the frontend builds against. **Acceptance:** every endpoint has one
example req + resp; the doc notes which fields are stubbed until Phases 2–4.

---

## Phase 1 exit criteria

- `crates/model-registry` builds; `ModelManager` runs stub jobs with live
  progress over both poll and the `models.jobs` WS lane.
- Full REST surface is callable and user-scoped; create/rename/version/alias all
  persist and audit correctly.
- Frontend can develop against a **frozen** contract; only the *content* behind
  train/test/eval changes in later phases, never the shapes.
