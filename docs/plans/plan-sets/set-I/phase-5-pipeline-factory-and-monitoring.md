# Phase 5 — Pipeline factory, fan-out & quality monitoring

**Completion: 100% (12 / 12 tasks)**

**Goal:** Turn one-off training into a **factory**. A declarative pipeline DAG
(data → features → target → train → calibrate → evaluate → register), templated and
spec-driven so it runs across many (asset, timeframe, window) cells in one **fan-out**,
on a **bar-based schedule**, with run history, **spec-hash caching**, incremental
re-runs, and retries. Two kinds: **training** pipelines (produce bundles) and
**inference** pipelines (assemble bundles → calibrated forecast — the thing that gets
published). Then add **suite-scoped quality monitoring** — rolling CRPS/coverage
drift, calibration and data/feature drift alerts, staleness — that **triggers a
retrain pipeline** (replacing the calendar-only scheduler).

**Depends on:** Phases 0–4 (a pipeline orchestrates the data view, training, eval,
and calibration that already exist).
**Blocks:** Phase 6 (Pipeline Builder UI; monitoring feeds the published contract).

---

## Design notes

**A pipeline is a DAG of existing steps — not a new compute layer.** Each node wraps a
capability already built: `data` (Phase 0 materialize), `features` (Phase 3),
`target` (Phase 1), `train` (Phase 1), `calibrate` (Phase 4), `evaluate` (Phase 2),
`register` (Set H registry). `PipelineManager` reuses the Set H job model
(tokio jobs, atomic progress, Postgres snapshots, the `models.jobs` WS lane —
or a new `pipelines.jobs` lane). This is distinct from the strategy automation
`PipelineRuntime` (a sequential trade-filter pipeline) — that is not reused.

```jsonc
{
  "schema_version": "1.1",
  "kind": "training",                 // training | inference
  "dag": [
    { "id": "data",     "op": "materialize", "params": { "feature_set_ref": "fs_core_ohlcv_v3" } },
    { "id": "train",    "op": "train",       "needs": ["data"], "params": { "framework": "lightgbm" } },
    { "id": "evaluate", "op": "evaluate",    "needs": ["train"] },
    { "id": "register", "op": "register",    "needs": ["evaluate"], "params": { "promote_if": "beats_baseline" } }
  ],
  "matrix": {                          // fan-out cross-product (the factory)
    "asset":     ["BTC-USD", "ETH-USD"],
    "timeframe": ["5m", "1h", "1d"],
    "window":    ["fast", "slow"]
  }
}
```

**Fan-out** instantiates the DAG once per matrix cell (asset × timeframe × window),
scheduled across the existing concurrency semaphore. **Fast/slow** are named window
presets of one architecture (e.g. short vs long train spans).

**Monitoring (D-11, suite-scoped).** For each published artifact, re-score recent
forecasts on a rolling basis (Phase 2 loop) to track CRPS and coverage **over time**;
detect calibration drift (PIT moving off uniform) and data/feature drift (input
distribution shift). When drift or **staleness** (no retrain in N bars) crosses a
threshold, **trigger a retrain pipeline** — closing the loop the Set H calendar
scheduler only half-served.

---

## Tasks

### ☑ I-5.1 `PipelineDefinition` (DAG) domain type + validation — M
Add the pipeline definition (shape above) to `crates/domain`: nodes with `op`,
`needs` (DAG edges), params, `kind`, and an optional `matrix`. Validate: acyclic,
all `needs` resolvable, ops known, training-vs-inference node legality.
**Acceptance:** `cargo test -p domain`: a valid DAG round-trips; a cycle and an unknown op are rejected; an inference-only op in a training pipeline is rejected.

### ☑ I-5.2 `PipelineManager` DAG execution engine — L
Add `PipelineManager` (mirroring `ModelManager`'s job model) that topologically
executes the DAG, each node delegating to the existing capability, with per-node
progress on a WS lane and a persisted run record. Migrations (Postgres 0026+:
`pipelines`, `pipeline_versions`, `pipeline_runs`, `pipeline_node_runs`).
**Acceptance:** a 4-node training pipeline runs end to end, producing a registered version; node progress streams; the run is persisted and queryable.

### ☑ I-5.3 Training vs inference pipeline kinds — M
Implement both kinds: **training** produces+registers bundles; **inference**
assembles published bundles (model or ensemble) into a calibrated forecast and is the
unit that gets published (Phase 6). Enforce kind-appropriate node sets.
**Acceptance:** a training pipeline yields a registered artifact; an inference pipeline, given published bundles, yields a calibrated distribution through the parity path.

### ☑ I-5.4 Templated, spec-driven pipelines — M
Allow a pipeline to be a **template** parameterized by (asset, timeframe, window),
cloneable and reusable; store templates in the registry (feeds Phase 6 presets).
**Acceptance:** one template instantiates for two different assets without edits; the instantiations differ only by bound parameters and have distinct spec hashes.

### ☑ I-5.5 Fan-out across asset × timeframe × window — L
Execute the `matrix` cross-product as one logical run spawning a child run per cell,
scheduled under the concurrency semaphore, with an aggregate status (N succeeded /
M failed) and per-cell drill-down.
**Acceptance:** a 2×3×2 matrix launches 12 child runs from one request; the parent reports aggregate + per-cell status; a single cell's failure does not abort the others.

### ☑ I-5.6 Fast/slow window instances — S
Define named window presets (`fast`, `slow`) as window-length bindings of one
architecture; usable as a matrix axis.
**Acceptance:** `fast` and `slow` instances of one pipeline train on different spans and register as distinct versions tagged by window.

### ☑ I-5.7 Bar-cadence scheduling — M
Extend the Set H `RetrainScheduler` into a bar-based scheduler: trigger a pipeline
every K bars of a reference instrument/timeframe (not only wall-clock nightly), with
the next-run state persisted.
**Acceptance:** a pipeline scheduled "every 96×15m bars" fires on bar cadence in a simulated clock; schedule state survives a restart.

### ☑ I-5.8 Run history, caching, incremental re-runs, retries — M
Use the Phase 3 spec hash to **cache** node outputs (skip a node whose inputs are
unchanged), support partial/incremental re-runs (resume from a failed node), and
retry transient failures with backoff. Persist full run history.
**Acceptance:** re-running an unchanged pipeline is a cache hit (no recompute); a run that failed at `evaluate` resumes from there; a transient sidecar error is retried, not failed outright.

### ☑ I-5.9 Rolling forecast-quality drift — M
For each published artifact, periodically re-score recent forecasts (Phase 2 loop)
and persist a **rolling CRPS/coverage** series to ClickHouse (DDL 05+:
`forecast_quality`); expose it for the UI and alerts.
**Acceptance:** the rolling series populates for a published model; a synthetic quality decline shows up as rising CRPS / falling coverage over time.

### ☑ I-5.10 Calibration & data/feature drift alerts — M
Detect calibration drift (PIT drifting off uniform) and data/feature drift (input
distribution shift vs the training snapshot, e.g. PSI/KS) and raise alerts (NATS +
persisted) with thresholds.
**Acceptance:** a shifted input distribution and a decalibrated forecast each raise a distinct alert; within-tolerance behavior raises none.

### ☑ I-5.11 Staleness → retrain trigger — M
Wire drift/staleness alerts to **trigger a retrain pipeline** (I-5.2) for the affected
artifact; record the causal link (alert → run). Staleness = no successful retrain in N
bars.
**Acceptance:** crossing a drift threshold (or staleness) enqueues a retrain pipeline run tagged with the triggering alert; the link is queryable.

### ☑ I-5.12 Pipeline & monitoring REST + WS surface — S
Add `/api/pipelines/**` (CRUD, run, fan-out status, history) and
`/api/models/{id}/quality` (rolling series + alerts); stream pipeline/node progress on
a WS lane. Additive to the Set H contract.
**Acceptance:** pipelines are creatable/runnable/queryable over REST; fan-out status and quality series are returned; progress streams over WS.

---

## Phase 5 exit criteria

- A declarative DAG pipeline runs end to end; training and inference kinds both work.
- Templated pipelines fan out across asset × timeframe × window in one run, on a
  bar-based schedule, with caching, incremental re-runs, and retries.
- Rolling CRPS/coverage drift, calibration drift, and data/feature drift are tracked
  and alert; staleness/drift **triggers a retrain pipeline** with a recorded causal link.
- `cargo test -p domain -p model-registry` + sidecar pytest green; `just lint`,
  `just check-money` green.
