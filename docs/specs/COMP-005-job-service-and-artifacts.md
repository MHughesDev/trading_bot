# COMP-005: Job Service and Artifact Store

**Status:** Proposed (Phase 0 contract; not implemented)
**Version:** 0.1
**ADR(s):** ADR-0030 (durable jobs, content-addressed artifacts), ADR-0025 (counting at
submission)
**Derived from:** BS-007
[05_JOBS_AND_ARTIFACTS](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/05_JOBS_AND_ARTIFACTS.MD)
**Plan set:** L
**Crates:** new `crates/jobs` (service, queues, leases, workers SDK); `crates/storage`
(artifacts: extends `artifacts/{fs,s3}.rs`); `crates/api` (routes);
`crates/backtest` (executor adapters); `apps/model-trainer` (worker); `migrations/0037_*`
**Replaces:**
- `BacktestManager`'s fixed `MAX_CONCURRENT_RUNS = 3` semaphore
  (`crates/backtest/src/manager.rs:153`);
- in-memory sweep and suite state (FEAT-003 / Set J deferrals);
- `asset_init_jobs` (0014) and `training_runs` (0020) as independent lifecycles. Both
  become job kinds; their tables become views or projections.

---

## 1. Purpose

One durable lifecycle for every slow operation, and one content-addressed home for
every large output. Jobs are idempotent by manifest, counted by the evaluation service
when they're trials, observable through events, and budgeted. Artifacts are passed by
handle and are reproducible from their manifests.

## 2. Job kinds

| Kind | Worker class | Evaluation-counted | Notes |
|---|---|---|---|
| `backtest` | `backtest` | Yes | A definition (SLv2) or `position_series` run (FEAT-004/005) |
| `sweep` | `backtest` (children) | Yes (each member) | FEAT-003 sweep; parent of member `backtest` jobs |
| `study` | `backtest` (children) | Yes (each member) | Set J StudyKinds |
| `gate_advance` | `backtest` | Yes | Funnel step (vault included) |
| `dataset_build` | `research` | No | FEAT-006 DatasetSpec |
| `feature_materialise` | `research` | No | DATA-006 |
| `train`, `hpo` | `trainer` | Yes (model trial counter) | FEAT-006 |
| `predict_series` | `trainer` | No | Walk-forward prediction series |
| `simulate_paths` | `research` | No | Process and path artifacts |
| `research_run` | `research` | No | Layer 1 strategy evaluation to a position series; analysis code |
| `backfill` | `data` | No | DATA-005 sources |
| `data_qc`, `instrument_profile` | `data` | No | DATA-005 §7; AGENT-003 L3 |
| `skill_verify` | `eval` | No (own budget) | AGENT-003 |
| `eval_task` | `eval` | No (own budget) | AGENT-004 |
| `asset_init` | `data` | No | Migrated from `asset_init_jobs` |

## 3. Data model (`migrations/0037_jobs_artifacts.sql`)

```sql
CREATE TABLE jobs (
  job_id         TEXT PRIMARY KEY,               -- 'job_' || ULID
  kind           TEXT NOT NULL,
  project_id     UUID,                            -- NULL for system jobs
  user_id        UUID NOT NULL,
  session_id     UUID,                            -- agent session if any
  submitted_by   TEXT NOT NULL CHECK (submitted_by IN ('agent','user','system')),
  experiment_id  TEXT,                            -- required for evaluation-counted kinds
  parent_job_id  TEXT REFERENCES jobs(job_id),
  queue          TEXT NOT NULL CHECK (queue IN ('agent','human','system')),
  priority       SMALLINT NOT NULL DEFAULT 5,
  manifest       JSONB NOT NULL,                  -- canonical JSON (RFC 8785 JCS)
  manifest_hash  TEXT NOT NULL,                   -- sha256 hex (see §4)
  state          TEXT NOT NULL,                   -- §5
  progress       JSONB NOT NULL DEFAULT '{}',     -- {pct, stage, message}
  estimate       JSONB,                           -- {compute_s, gpu_s, cost_usd}
  actual         JSONB,
  result_summary TEXT,                            -- ≤ 1536 bytes, for_model
  result         JSONB,                           -- metrics, artifact handles
  error          JSONB,                           -- {code, field, rule, fix, detail_ref}
  attempts       SMALLINT NOT NULL DEFAULT 0,
  lease_owner    TEXT, lease_expires_at TIMESTAMPTZ, heartbeat_at TIMESTAMPTZ,
  created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  started_at     TIMESTAMPTZ, finished_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX jobs_idem ON jobs (coalesce(project_id,'00000000-0000-0000-0000-000000000000'), manifest_hash)
  WHERE parent_job_id IS NULL;
CREATE INDEX jobs_queue ON jobs (queue, state, priority DESC, created_at);
CREATE INDEX jobs_project ON jobs (project_id, created_at DESC);

CREATE TABLE job_events (
  id BIGSERIAL PRIMARY KEY, job_id TEXT NOT NULL REFERENCES jobs ON DELETE CASCADE,
  kind TEXT NOT NULL,            -- state | progress | log_ref | checkpoint
  payload JSONB NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE artifacts (
  handle        TEXT PRIMARY KEY,                -- 'art_' || first 32 hex of sha256
  sha256        TEXT NOT NULL UNIQUE,
  type          TEXT NOT NULL,                   -- §8
  project_id    UUID,                            -- NULL = global
  uri           TEXT NOT NULL,                   -- file:// | s3:// (storage::artifacts)
  size_bytes    BIGINT NOT NULL, xxh3 TEXT NOT NULL,
  manifest      JSONB NOT NULL,                  -- §8.2
  producer_job  TEXT REFERENCES jobs(job_id),
  pinned        BOOLEAN NOT NULL DEFAULT false,
  expires_at    TIMESTAMPTZ,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE job_artifacts (job_id TEXT REFERENCES jobs ON DELETE CASCADE,
  handle TEXT REFERENCES artifacts, role TEXT, PRIMARY KEY (job_id, handle));
CREATE TABLE artifact_refs (          -- pins: who cites an artifact
  handle TEXT REFERENCES artifacts, ref_kind TEXT, ref_id TEXT, PRIMARY KEY (handle, ref_kind, ref_id));

CREATE TABLE exploration_ledger (
  id BIGSERIAL PRIMARY KEY, project_id UUID NOT NULL, user_id UUID NOT NULL, session_id UUID,
  source TEXT NOT NULL,          -- data_api | job | desk
  instruments TEXT[] NOT NULL, timeframe TEXT, window_start TIMESTAMPTZ, window_end TIMESTAMPTZ,
  variables TEXT[] NOT NULL DEFAULT '{}',   -- features/columns touched (for post_hoc detection)
  description TEXT NOT NULL, handle TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX exploration_ledger_proj ON exploration_ledger (project_id, created_at);
```

## 4. Manifests and idempotency

- **Canonical form:** the manifest is serialised with JSON Canonicalization Scheme
  (RFC 8785).
- **Hash:** `manifest_hash = sha256(canonical(manifest) ‖ sorted(code_hashes) ‖
  data_snapshot_id ‖ kind)`.
  - `code_hashes` are the sha256 of code-snapshot artifacts: Layer 1 strategy files, BYO
    model files, skill bundles.
  - `data_snapshot_id` pins data versions (DATA-005 §5). The default is
    `latest@<submission time>`, resolved to a concrete snapshot id at submission.
- **Idempotency:** `POST /api/jobs` with an existing `(project_id, manifest_hash)` for a
  top-level job returns `200 {job_id, deduplicated: true, state}`, whatever that job's
  state, including failed.
- **Explicit re-runs** add a `rerun_nonce` field to the manifest. A re-run is a new job
  and **counted as a new trial**. It must be deliberate.
- Children (sweep and study members) are keyed by `(parent_job_id, member_index)` and
  are never deduplicated across parents.

## 5. Lifecycle

```
queued → leased → running → succeeded | failed | cancelled
   │                 └── paused (awaiting approval, e.g. qc waiver / budget) → running | cancelled
   └── cancelled
```

- **Leasing:** a worker claims with `SELECT … FOR UPDATE SKIP LOCKED` in queue priority
  order and sets `lease_expires_at = now + lease_len` (default 60 s). It heartbeats every
  `lease_len/3`.
- **Lease expiry:** jobs of idempotent-safe kinds (all kinds, because workers must be
  pure given the manifest) are re-queued with `attempts += 1`. After 2 infrastructure
  re-queues the job fails with `lost_worker`.
- **Logic failures** (validation, strategy error, data error) fail immediately with a
  structured `error` and are never retried automatically.
- **Cancellation:** `cancelled` is terminal. Running jobs get a cancel signal and must
  stop within `cancel_grace` (default 30 s); children are cancelled with their parent.
- **Terminal states are immutable.**

## 6. Events and waiting

- Every state and progress change publishes on NATS subject `jobs.<project_id>.<job_id>`
  (JetStream stream `JOBS`, 7-day retention) and inserts a `job_events` row.
- **SSE:** `GET /api/jobs/events?project_id=&job_ids=&after_id=` streams events
  (resumable by `after_id`). The agent orchestrator relays them into `agent_events`
  (AGENT-001 §18).
- **Progress** updates are rate-limited to one per 5 s per job. There is no per-epoch or
  per-member spam; aggregate progress is on the parent.
- **CLI semantics** (AGENT-002):
  - `wait` subscribes and blocks until terminal or timeout, prints one summary line per
    job, and exits 0 (all succeeded), 1 (any failed) or 2 (timeout). It never loops.
  - `watch` runs detached and prints one line per completion.

## 7. Queues, fair share, budgets

- **Queues** `agent | human | system` have weights (default 3 : 5 : 2) per worker class.
  Within a queue, jobs are ordered by priority, then fair share by project (least recent
  service first), then FIFO.
- **Per-project concurrency caps** come from the project tier, per worker class. Campaign
  workers share their project's cap.
- **Estimates:** each kind provides `estimate(manifest)` (compute_s, gpu_s, cost_usd).
  Submission is refused with `409 budget_exhausted` if the estimate exceeds the project's
  remaining compute budget. `actual` is recorded at completion.
- **Worker pools** are configured in `config/jobs.toml`:
  `backtest.max_parallel`, `research.max_parallel`, `trainer.max_parallel`,
  `trainer.gpu`, and so on. The fixed backtest semaphore is removed.

## 8. Artifact store

### 8.1 Types

`parquet_extract`, `dataset`, `feature_matrix`, `prediction_series`, `model_bundle`,
`run_outputs`, `study_result`, `path_set`, `process`, `chart_spec`, `report`,
`dossier`, `skill_bundle`, `code_snapshot`, `log`, `exploration_summary`.

### 8.2 Manifest (required fields)

```jsonc
{ "type": "parquet_extract", "producer": {"job_id": "job_…"} | {"api": "/api/data/bars", "query": {…}},
  "inputs": ["art_…"], "params": {…}, "schema": {"columns": [{"name","type"}]}, "rows": 0,
  "cutoff_applied": "2026-03-31T00:00:00Z|null", "as_of": "…", "data_snapshot_id": "…",
  "qc_grade": "A|B|C|D|null", "code_hashes": ["sha256…"], "created_at": "…" }
```

### 8.3 Storage and access

- Bytes go through `storage::artifacts::ArtifactStore` (fs in dev, s3/MinIO in prod), at
  key `sha256/<aa>/<sha256>`. The existing xxh3 value is kept as an integrity check;
  handles use sha256.
- **Scope:** an artifact belongs to its project. Reads from another project are `404`
  unless the artifact is global (skill bundles, global feature definitions, core
  references).
- **Pinning:** any `artifact_refs` row (from an experiment, report, finding, skill,
  dataset or model) pins the artifact. Unpinned `parquet_extract` and `log` artifacts
  expire after `artifact_ttl_days` (default 30).
- **API:**
  - `GET /api/artifacts/{handle}` returns the manifest and summary;
  - `GET /api/artifacts/{handle}/content` streams the bytes (range requests supported);
  - `POST /api/artifacts` is for uploads (code snapshots, reports) by `research:artifacts`
    tokens, with a size cap and type allowlist.

## 9. Exploration ledger

Every Data API read (DATA-005), `feature_materialise`, `research_run`, analysis endpoint
and Desk read appends an `exploration_ledger` row with the variables touched. The ledger
is:
- summarised into an `exploration_summary` artifact per verdict;
- used by the hypothesis registry to set `post_hoc` (BACKTEST_SUITE_CORE_SPEC v2).

## 10. Counting integration (INV-1)

- On insert of a job with an evaluation-counted kind, the job service calls
  `EvaluationService::register_trial(experiment_id, job_id, kind)` **in the same
  transaction**. That covers the Experiment trial counter, or the model trial counter for
  `train` and `hpo`.
- **Missing or foreign experiment:** a missing `experiment_id` for an agent token is
  `422 experiment_required`. An experiment id that isn't in the project is `404`.
- Deduplicated submissions (§4) don't register a new trial. `rerun_nonce` submissions do.

## 11. REST surface

| Method and path | Scope | Notes |
|---|---|---|
| `POST /api/jobs` `{kind, manifest, experiment_id?, priority?}` | `research:jobs` / web | `201` new, `200` deduplicated, `409` budget, `422` invalid |
| `GET /api/jobs/{id}` | same | `?detail=summary|full` |
| `GET /api/jobs?project_id&state&kind&limit` | same | Top-k plus total |
| `POST /api/jobs/{id}/cancel` | same | — |
| `GET /api/jobs/{id}/logs?tail=` | same | Log artifact slice |
| `GET /api/jobs/events` (SSE) | same | §6 |
| `GET/POST /api/artifacts…` | `research:artifacts` / web | §8.3 |

**Kind-specific endpoints** stay as thin façades that build manifests and call the job
service:
- `/api/backtest/experiments/{id}/studies`;
- `/api/research/sweeps`;
- model training.

## 12. Worker SDK

- **Rust** (`crates/jobs::worker`): a `Worker` trait with `kind()`,
  `estimate(&Manifest)` and `run(ctx, manifest) -> Result<JobOutput, JobError>`. `ctx`
  provides progress, heartbeat, cancel token, artifact put and log sink.
- **Python** (`tbot_worker` package, used by `apps/model-trainer` and the research
  runner): the same contract over HTTP (`POST /api/jobs/claim`, `…/heartbeat`,
  `…/complete`) with a worker token (`system:worker` scope).
- Workers must be pure given the manifest (plus the pinned data snapshot and code
  hashes). This is what makes re-queue safe.

## 13. Migration

1. `BacktestManager` becomes the `backtest` worker. Remove the `MAX_CONCURRENT_RUNS`
   semaphore; `backtest.max_parallel` replaces it.
2. `crates/research` sweep state and the Set J `SuiteManager` in-memory stores move to
   `jobs` plus the Set K Postgres/ClickHouse stores. The funnel stops executing under the
   suite lock.
3. `asset_init_jobs` and `training_runs` become kinds `asset_init` and `train`. The old
   tables become views over `jobs` for existing UI queries until COMP-006 lands.

## 14. Test plan and acceptance

| # | Test | BS-007 IDs |
|---|---|---|
| J1 | Same backtest manifest submitted twice → one job; the experiment trial counter +1 | JB-02, JB-03 |
| J2 | Kill a backtest worker mid-job → re-queued, completes; a platform restart leaves no orphans or double counts | JB-04 |
| J3 | 20-member sweep plus an unrelated human backtest run concurrently under fair share | JB-05 |
| J4 | `tbot jobs wait` on a 10-minute job prints exactly one line; `watch` notifies on completion | JB-06 |
| J5 | A failing strategy returns a ≤ 400 B structured error with the detail in an artifact; no auto-retry | JB-07 |
| J6 | Budget exhaustion refuses submission with `409` | JB-08 |
| J7 | A report's cited artifacts resolve to manifests that reproduce their numbers | JB-09 |
| J8 | A Layer 1 job's code snapshot hash is in its manifest hash; editing the file changes the hash | JB-10 |
| J9 | Data reads and analysis jobs appear in the exploration ledger with their variables | JB-11 |
| J10 | Sweep state survives a restart | JB-12 |

## 15. Traceability

Implements BS-007 JB-01…JB-12, and supports RT-17, CX-04 and EV-10.
