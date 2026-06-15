# Phase 0 — Foundations

**Completion: 0% (0 / 12 tasks)**

**Goal:** Lay the immutable spine the rest of the Studio stands on — the frozen
**Model Definition format v1.0**, the Rust domain types, the complete Postgres
schema (migrations `0017`–`0024`), the artifact object-store abstraction, and
the ADR that freezes the format. No behaviour yet; this phase is *shape*.

**Depends on:** nothing. **Blocks:** all other phases.

---

## Design — Model Definition v1.0 (frozen)

Mirrors `StrategyDefinition` (ADR-0007). A **definition** is the authoring
artifact a user creates; it is frozen at `schema_version: "1.0"`, validated by
a single validator, and stored verbatim as JSONB. It is *not* a trained model —
it is the recipe a training run consumes.

```jsonc
{
  "schema_version": "1.0",
  "model_kind": "forecaster",            // forecaster|signal_ranker|trade_decision|risk_sizing|embedding|external_llm_adapter
  "framework": "xgboost",                // xgboost|lightgbm|sklearn|torch|external_api
  "runtime": "python",                   // python (default) | rust (deferred, D-5)
  "asset_class": "crypto_spot_cex",      // scoping, same vocabulary as strategies
  "target": {                            // what it predicts (kind-specific)
    "field": "return",                   // return|price|volatility|direction|action|score|size_fraction
    "horizon": "1h",                     // forecast horizon (ISO-8601 duration-ish token)
    "transform": "logret"                // none|logret|zscore|...
  },
  "feature_set_ref": "fs_core_ohlcv_v3", // versioned feature set (Phase 2)
  "hyperparameters": { "max_depth": 6, "n_estimators": 400, "learning_rate": 0.05 },
  "label_spec": { "type": "forward_return", "window": "1h", "clip": [-0.2, 0.2] },
  "inference": { "min_confidence": 0.0, "calibrate": true },
  "adapter": null                        // populated only for external_llm_adapter (provider, model, endpoint)
}
```

`external_llm_adapter` definitions carry an `adapter` block instead of
`target`/`feature_set_ref`:

```jsonc
{
  "schema_version": "1.0",
  "model_kind": "external_llm_adapter",
  "framework": "external_api",
  "runtime": "python",
  "adapter": {
    "provider": "ollama",                // ollama|openai|anthropic|...
    "model": "gemma2:9b",
    "endpoint": "http://localhost:11434",
    "default_params": { "temperature": 0.7, "max_tokens": 2048 },
    "cost_per_1k_tokens": 0.0
  }
}
```

**Validation rules (frozen):** `model_kind` ∈ vocabulary; `framework` valid for
the kind (e.g. `embedding`→`external_api`); `asset_class` in the declared
registry; `feature_set_ref` resolves (Phase 2) for trainable kinds; `target`
required for trainable kinds and absent for adapters; unknown top-level keys
rejected. Validator lives beside the strategy validator pattern.

---

## Tasks

### ☐ H-0.1 Author ADR-0015: Freeze Model Definition Format v1.0 — M

**Why.** Models are user data identical in kind to strategies: persisted,
reloaded on restart, referenced by saved strategies. ADR-0007's entire rationale
applies. Freezing first means every front door (Studio UI, JSON API, future MCP)
targets one stable format.

**Steps:** Create `docs/adr/0015-freeze-model-definition-format-v1.md` following
the exact structure of `0007-freeze-strategy-definition-format-v1.md` (Context /
Decision / Rationale / Consequences / Alternatives). Pin: the `model_kind`
vocabulary, the `framework`×`model_kind` compatibility matrix, `target` schema,
`adapter` schema, `runtime` values, and the `schema_version` evolution rule.
Add the row to `docs/adr/README.md`.

**Acceptance criteria:**
- ADR-0015 exists, Accepted, dated 2026-06-15, listed in the ADR index.
- It explicitly states the frozen format lives in `crates/domain/src/model_def/`.
- It cross-references ADR-0007 as the precedent.

---

### ☐ H-0.2 Domain crate: `model_def` module (frozen format + validator) — L

**Why.** Single authoritative source for the format, mirroring
`crates/domain/src/strategy_def/`.

**Files:** new `crates/domain/src/model_def/{mod.rs,kinds.rs,target.rs,adapter.rs,validate.rs}`;
register in `crates/domain/src/lib.rs`.

**Shape:**
```rust
// kinds.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind { Forecaster, SignalRanker, TradeDecision, RiskSizing, Embedding, ExternalLlmAdapter }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Framework { Xgboost, Lightgbm, Sklearn, Torch, ExternalApi }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime { Python, Rust }

// mod.rs
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelDefinition {
    pub schema_version: String,          // must equal "1.0"
    pub model_kind: ModelKind,
    pub framework: Framework,
    #[serde(default)] pub runtime: Runtime,
    pub asset_class: String,
    #[serde(default)] pub target: Option<TargetSpec>,
    #[serde(default)] pub feature_set_ref: Option<String>,
    #[serde(default)] pub hyperparameters: serde_json::Value,
    #[serde(default)] pub label_spec: Option<serde_json::Value>,
    #[serde(default)] pub inference: InferenceCfg,
    #[serde(default)] pub adapter: Option<AdapterSpec>,
}
```

**Acceptance criteria:**
- `ModelDefinition` round-trips both example documents above (serde tests).
- `validate(&def) -> Result<Validated, Vec<ValidationError>>` exists; rejects
  unknown keys, wrong `schema_version`, kind/framework mismatch, missing `target`
  for trainable kinds, and missing `adapter` for `external_llm_adapter`.
- `cargo test -p domain model_def` passes; no `f64` money types introduced (ADR-0002).

---

### ☐ H-0.3 Domain crate: registry record + lifecycle enums — M

**Why.** Status, alias, and run-state enums are shared across storage, API, and
runtime; they belong in `domain` so every crate agrees.

**Files:** `crates/domain/src/model/{mod.rs,status.rs,alias.rs,forecast.rs}`.

**Shape:**
```rust
#[serde(rename_all = "snake_case")]
pub enum ModelStatus { Draft, Training, Evaluating, Candidate, Active, Archived, Failed }

#[serde(rename_all = "snake_case")]
pub enum AliasName { Production, Candidate, Staging, Fallback }

#[serde(rename_all = "snake_case")]
pub enum RunStatus { Queued, Running, Succeeded, Failed, Cancelled }

/// Canonical inference output a strategy consumes (kind-agnostic envelope).
pub struct Forecast {
    pub model_id: String, pub version: u32, pub instrument_id: String,
    pub direction: Direction,        // Up | Down | Flat
    pub magnitude: Decimal,          // ADR-0002 decimal, never f64
    pub confidence: f64,             // 0..1, calibration metric, not money
    pub horizon: String, pub produced_at: DateTime<Utc>,
}
```

**Acceptance criteria:**
- Enums serialize to the snake_case wire tokens used in §lifecycle.
- `Forecast.magnitude` is `Decimal` (ADR-0002 compliance test).
- Unit tests for status transition legality (`Draft→Training` ok, `Active→Training`
  rejected — a new version trains, not a promoted one).

---

### ☐ H-0.4 Migration 0017 — `ai_models`, `model_aliases`, `model_events` — M

**Why.** Model identity, the movable alias pointers, and the audit trail
(including renames, which D-1/§rename require be tracked, not silent string
edits).

**File:** `migrations/0017_ai_models.sql`

```sql
CREATE TABLE ai_models (
    model_id     TEXT PRIMARY KEY,                 -- e.g. "mdl_01J..."
    slug         TEXT NOT NULL UNIQUE,             -- immutable, url-safe
    display_name TEXT NOT NULL,                    -- renamable
    description  TEXT,
    model_kind   TEXT NOT NULL,
    asset_class  TEXT NOT NULL,
    definition_json JSONB NOT NULL,                -- frozen ModelDefinition v1.0
    status       TEXT NOT NULL DEFAULT 'draft',    -- coarse model status (= latest meaningful version)
    created_by   UUID NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_ai_models_owner ON ai_models(created_by, created_at DESC);
CREATE INDEX idx_ai_models_kind  ON ai_models(model_kind, asset_class);

CREATE TABLE model_aliases (
    model_id   TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    alias      TEXT NOT NULL,                       -- production|candidate|staging|fallback
    version    INTEGER NOT NULL,                    -- -> model_versions.version
    updated_by UUID NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (model_id, alias)
);

CREATE TABLE model_events (                          -- immutable audit log
    event_id   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    model_id   TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,                        -- created|renamed|version_created|promoted|rolled_back|archived
    payload    JSONB NOT NULL,                       -- {from,to,reason,...}
    actor      UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_model_events_model ON model_events(model_id, created_at DESC);
```

**Acceptance criteria:**
- `sqlx migrate run` applies cleanly on a fresh DB and is idempotent on re-run.
- `slug` uniqueness enforced; `display_name` non-unique (renames may collide by design).
- FK cascades verified (deleting a model removes aliases + events).

---

### ☐ H-0.5 Migration 0018 — `model_versions`, `model_artifacts` — M

**File:** `migrations/0018_model_versions.sql`

```sql
CREATE TABLE model_versions (
    model_id        TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    version         INTEGER NOT NULL,
    status          TEXT NOT NULL DEFAULT 'draft',   -- ModelStatus
    training_run_id UUID,                            -- -> training_runs (null for registered/adapter)
    dataset_version_id UUID,                         -- -> dataset_versions
    artifact_id     UUID,                            -- -> model_artifacts
    metrics_json    JSONB,                           -- training/holdout metrics snapshot
    scorecard_json  JSONB,                           -- normalized 0-100 sub-scores (Phase 3)
    config_json     JSONB NOT NULL,                  -- resolved definition + code_ref
    notes           TEXT,
    created_by      UUID NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    promoted_at     TIMESTAMPTZ,
    PRIMARY KEY (model_id, version)
);
CREATE INDEX idx_model_versions_status ON model_versions(model_id, status);

CREATE TABLE model_artifacts (
    artifact_id   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    model_id      TEXT NOT NULL REFERENCES ai_models(model_id) ON DELETE CASCADE,
    version       INTEGER NOT NULL,
    storage_uri   TEXT NOT NULL,                     -- file://… | s3://… (Phase 0 abstraction)
    artifact_type TEXT NOT NULL,                     -- model|tokenizer|adapter|metadata|logs
    size_bytes    BIGINT NOT NULL,
    sha256        TEXT NOT NULL,                     -- integrity, verified at promotion gate
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

**Acceptance criteria:**
- Composite PK `(model_id, version)`; versions are append-only (no UPDATE of a
  promoted version's `config_json` — enforced in storage layer Phase 1).
- `sha256` NOT NULL; promotion gate (Phase 3) reads it.

---

### ☐ H-0.6 Migration 0019 — `datasets`, `dataset_versions`, `feature_sets` — M

**File:** `migrations/0019_model_datasets.sql`. Tables for the dataset/feature
registry (built out in Phase 2): a `feature_sets` record (named, versioned list
of features from the `features` crate), a `datasets` identity, and immutable
hashed `dataset_versions` (window + feature_set + label_spec + row count + hash).

**Acceptance criteria:**
- `dataset_versions.content_hash` UNIQUE per `(dataset_id, version)`; re-materializing
  identical inputs yields the same hash (determinism, ADR-0008/0009 discipline).
- `feature_sets.feature_list_json` references feature names the `features` crate
  can produce.

---

### ☐ H-0.7 Migration 0020 — `training_runs` — M

**File:** `migrations/0020_model_training_runs.sql`. One row per training attempt,
mirroring `backtest_runs` (`0011`): `run_id`, `model_id`, `dataset_version_id`,
`status` (RunStatus), `progress FLOAT`, `phase TEXT`, `hyperparameters_json`,
`metrics_json`, `logs_uri`, `sidecar_job_ref`, timestamps, `created_by`.

**Acceptance criteria:**
- `progress` 0–100, `phase` free-text (e.g. `materializing|fitting|validating`).
- Indexed by `(model_id, created_at DESC)` and `(status)` for the active-poll query.

---

### ☐ H-0.8 Migration 0021 — `evaluation_runs` — M

**File:** `migrations/0021_model_evaluation_runs.sql`. One row per eval:
`eval_id`, `model_id`, `version`, `eval_dataset_version_id`, `status`,
`metrics_json`, `scorecard_json`, `regression_report_json`,
`sample_outputs_json` (forecast-vs-actual samples for the UI), `baseline_version`
(for comparison), timestamps.

**Acceptance criteria:**
- Can store a comparison against a `baseline_version` (the current `production`).
- `regression_report_json` shape documented inline (per-metric delta + verdict).

---

### ☐ H-0.9 Migration 0022 — `model_deployments` — S

**File:** `migrations/0022_model_deployments.sql`. Tracks where a version is
actually serving: `deployment_id`, `model_id`, `version`, `environment`
(`paper|live`), `alias`, `status`, `traffic_pct`, `deployed_at`, `deployed_by`.
Supports A/B (`traffic_pct`) and the deployment control-room UI (Phase 5).

**Acceptance criteria:**
- A version can be deployed to `paper` and `live` independently.
- `traffic_pct` constrained 0–100; sum per `(model_id, environment)` ≤ 100
  (enforced in storage layer, documented here).

---

### ☐ H-0.10 Migration 0023 — `model_test_cases` — S

**File:** `migrations/0023_model_test_cases.sql`. Saved Test Lab inputs so a
playground session is reproducible and shareable: `case_id`, `model_id`,
`name`, `input_json` (prompt/params or feature vector/instrument+window),
`expected_json` (optional golden), `created_by`, `created_at`. These also seed
lightweight regression checks.

**Acceptance criteria:**
- A saved case can be re-run from the Test Lab and from an eval suite.

---

### ☐ H-0.11 ClickHouse DDL — `model_predictions`, `model_traces` — M

**Why.** High-volume, append-only prediction logs and inference traces belong in
ClickHouse, not Postgres (ADR-0004 storage split), alongside `instrument_features`.

**Files:** `clickhouse/` DDL + a `crates/storage/src/clickhouse/model_traces.rs`
writer stub (batch insert pattern like `features.rs`).

**Shape:** `model_predictions(model_id, version, instrument_id, event_time_us,
produced_time_us, direction, magnitude_str, confidence, horizon)` and
`model_traces(trace_id, model_id, version, kind, latency_ms, cost_usd_str,
input_hash, output_hash, status, ts_us)`.

**Acceptance criteria:**
- DDL applies via the existing ClickHouse bootstrap path.
- Decimal-bearing fields stored as strings (decimal-safe, matches `FeatureRow`).

---

### ☐ H-0.12 Artifact object-store abstraction — M

**Why.** Artifacts (weights, adapters, tokenizers, logs) need a storage backend
that is local-FS for dev and S3/MinIO in prod, without leaking the choice into
callers. Mirrors how `storage` hides Postgres/ClickHouse/Parquet behind adapters.

**Files:** `crates/storage/src/artifacts/{mod.rs,fs.rs,s3.rs}` exposing:
```rust
#[async_trait] pub trait ArtifactStore {
    async fn put(&self, key: &str, bytes: Bytes) -> Result<ArtifactRef>; // returns uri + sha256 + size
    async fn get(&self, uri: &str) -> Result<Bytes>;
    async fn presign_get(&self, uri: &str, ttl: Duration) -> Result<Url>; // UI download
}
```
Backend selected by config (`ARTIFACT_STORE=fs|s3`, `.env.example` updated).

**Acceptance criteria:**
- `FsArtifactStore` round-trips bytes and reports a stable `sha256`.
- `S3ArtifactStore` compiles behind the same trait (wired to MinIO via env;
  integration test gated like other external-service tests).
- The Python sidecars (Phase 2) read/write the **same** URIs (handoff is a URI +
  hash, never raw bytes over the orchestration channel for large artifacts).

---

## Phase 0 exit criteria

- ADR-0015 accepted; `model_def` + `model` domain modules compile with tests.
- Migrations `0017`–`0023` apply cleanly; ClickHouse DDL applied.
- `ArtifactStore` trait + FS backend usable; S3 backend compiles.
- **No business logic yet** — registry/API/runtime are Phases 1+. A reviewer can
  read this phase and know every table, type, and URI the system will ever touch.
