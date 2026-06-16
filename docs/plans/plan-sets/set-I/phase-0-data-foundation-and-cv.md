# Phase 0 — Leakage-safe data & walk-forward CV

**Completion: 36% (4 / 11 tasks)** — the pure trust-foundation core (ADR-0017, the
`WalkForwardSpec` domain type, the PURE fold generator) plus the leakage-safe
`DataView` is landed and tested. The bar-level data-quality compute core (I-0.7)
also landed (its REST endpoint is pending under I-0.11). The remaining tasks are
the DB/sidecar-integrated layer (real materialization, snapshots, leakage harness,
trainer dispatch wiring, REST) — these touch live ClickHouse/Postgres and the
Python sidecar and are not runtime-verifiable in CI without those services.

**Goal:** Stand up the **trust substrate** the whole suite rests on: a
point-in-time, forming-bar-safe data view; a **walk-forward CV engine** with three
roles (train/calibration/test), expanding or rolling, with **purge + embargo**; a
data-quality preview; and an **automated leakage test** that runs in every
pipeline. Close the Set H `datasets.rs` materialization stub so a training run
consumes a real, pinned, hashed dataset rather than `row_count = 0`. Per the
capability spec, *nothing above this is trustworthy until it is solid* — so this
phase ships before any new feature or model family.

**Depends on:** Set H (`crates/model-registry`, `apps/model-trainer`).
**Blocks:** Phases 1–5 (every training/eval run uses these windows and this view).

---

## Design notes

**Build on the real PIT primitives — do not invent a new data stack.** The repo
already has point-in-time access and resampling that are correct by construction:

- `crates/backtest/src/store.rs` → `BarStore::load_bars` / `load_bars_bucketed(instrument_id, base_tf, bucket_seconds, from, to)` — dedup via `argMax(field, revision) GROUP BY available_time`.
- `crates/backtest/src/aggregate.rs` → `aggregate_bars(bars, target_tf, base_tf)` + `bucket_close_ns` (forming-bar-safe higher-timeframe assembly).
- `crates/domain/src/timestamp.rs` → the 4-timestamp model; `available_time` is the universal sort key ("lookahead impossible by construction", ADR-0008).

The data view exposes these to the suite behind a guard that **cannot return a bar
whose `available_time` exceeds the window's `as_of`**.

**Walk-forward spec (frozen shape, lives in `crates/domain/src/model_def/cv.rs`,
added to `ModelDefinition` as the optional `cv` block — D-5):**

```rust
pub enum WindowMode { Expanding, Rolling }

pub struct WalkForwardSpec {
    pub mode: WindowMode,
    pub folds: u32,            // number of sequential folds
    pub train_bars: u64,       // rolling: fixed train length; expanding: minimum
    pub cal_bars: u64,         // calibration role length (NEW vs Set H train/val/test)
    pub test_bars: u64,        // out-of-sample test length per fold
    pub purge_bars: u64,       // drop rows whose label window overlaps the boundary
    pub embargo_bars: u64,     // gap after test before next train (default: label horizon)
}
```

**Three roles, purge, embargo (one fold):**

```
 … train ……………………│purge│ cal …│purge│ test ……│ embargo │ (next fold) …
                     └ no row whose label horizon reaches across a boundary survives ┘
```

- **train** fits the estimator. **cal** is reserved for conformal/calibration
  fitting (Phase 4) and is *never* seen by HPO scoring. **test** is out-of-sample.
- **purge** removes rows whose forward label window (horizon H) overlaps the next
  role's start — the López de Prado discipline. **embargo** adds a gap after test.
- Rust computes fold boundaries as **index ranges over the pinned dataset's
  `available_time` column** and hands them to the sidecar; the sidecar never picks
  its own split and never sees rows outside the role it is fitting.

**Materialization (closing the stub).** `DatasetManager::materialize`
(`crates/model-registry/src/datasets.rs`) becomes real: pull base bars via the PIT
store, resample to the requested timeframe, compute the feature columns (mirroring
the trainer's `features.py` column set so train/serve agree), write **Parquet** to
the `ArtifactStore`, and set `content_hash = sha256(materialization params + bytes)`
and the true `row_count`. The pinned `parquet_uri` + `content_hash` is the
immutable snapshot every reproduce-from-hash run (Phase 3) reloads.

---

## Tasks

### ☑ I-0.1 Author ADR-0017 (walk-forward CV & leakage discipline) — S
Write `docs/adr/0017-walk-forward-cv-and-leakage-discipline.md` (Context / Decision
/ Rationale / Consequences / Alternatives). Record: three-role split, purge +
embargo defaults (embargo ≥ label horizon), expanding vs rolling, and the rule
that **every** pipeline runs a leakage test. Cite ADR-0008/0009. Mark Accepted.
**Acceptance:** ADR-0017 exists, linked from `docs/adr/README.md` and Set I MASTER §9.

### ☑ I-0.2 `WalkForwardSpec` domain type + validation — M
Add `crates/domain/src/model_def/cv.rs` with `WalkForwardSpec`, `WindowMode`, and
validation (folds ≥ 1; all bar counts > 0; `embargo_bars ≥` label horizon in bars;
total span ≤ available history is checked at materialization, not here). Wire an
optional `cv: Option<WalkForwardSpec>` field into `ModelDefinition` (additive,
`#[serde(default)]`). Default when absent: a single expanding fold = today's
behavior, so v1.0 specs are unchanged.
**Acceptance:** `cargo test -p domain` covers round-trip + each validation reject; a
v1.0 definition without `cv` still validates.

### ☑ I-0.3 Pure walk-forward window generator — M
Add a pure function (no I/O) `walk_forward_folds(index: &[AvailableTime], spec: &WalkForwardSpec, horizon_bars: u64) -> Vec<Fold>` where `Fold { train: Range, cal: Range, test: Range }` are index ranges, with purge applied at every boundary and embargo between folds. Place in `crates/features` (the PURE crate) so it is parity-safe and unit-testable. Property test: no index appears in two roles; no train/cal index's `[i, i+horizon]` overlaps a later role.
**Acceptance:** unit + property tests green; an expanding 5-fold and a rolling 5-fold over a synthetic index produce non-overlapping, purged, embargoed folds.

### ☑ I-0.4 Suite point-in-time data view (forming-bar-safe resample) — M
Add a thin `DataView` in `model-registry` that wraps `BarStore::load_bars_bucketed`
+ `aggregate_bars` behind an `as_of: AvailableTime` ceiling, exposing
`bars(instrument, timeframe, from, to, as_of)` that **filters out any bar with
`available_time > as_of`** and never returns a forming (incomplete) higher-timeframe
bucket. No new query path — reuse the backtest store.
**Acceptance:** a request with `as_of` mid-series returns only settled bars ≤ `as_of`; a unit test asserts the last partial bucket is excluded.

### ☐ I-0.5 Real `DatasetManager::materialize` (close the stub) — L
Replace the `datasets.rs` stub (`row_count = 0`, params-only hash) with a real
materialization: PIT pull → resample → compute feature columns (column set matches
`apps/model-trainer/app/features.py`) → forward-label per `label_spec` → drop NaN →
write Parquet to `ArtifactStore` → persist true `row_count`, `content_hash =
sha256(params ‖ bytes)`, `parquet_uri`, and the realized `[from, to]` span in
`dataset_versions`. Idempotent: identical params + data ⇒ identical hash ⇒ reuse.
**Acceptance:** materializing a real ClickHouse window writes a non-empty Parquet, `row_count > 0`, and a stable hash; re-running returns the same `dataset_version_id` without rewriting.

### ☐ I-0.6 Pinned dataset snapshots — S
Guarantee snapshot immutability: once a `dataset_version` is written, its
`parquet_uri` + `content_hash` never change; later revisions (late data, ADR-0009)
produce a **new** version, never a mutation. Record the source data's max
`revision` seen, so a snapshot is reproducible.
**Acceptance:** a unit/integration test shows a second materialize after a late-data revision yields a new `dataset_version_id`, leaving the first byte-identical.

### ☐ I-0.7 Data-quality preview — M
Compute gaps (missing bars vs the timeframe grid), duplicates (same `available_time`),
and outliers (|return| beyond an N-σ robust band) over a selected (instrument,
timeframe, range), returned **before** training. Add `GET /api/models/data/quality`.
Source: COMP-001. No mutation — read-only diagnostics.
**Acceptance:** endpoint returns `{gaps, dupes, outliers, bar_count, coverage_pct}` for a real window; a planted gap and a planted dupe in a fixture are both detected.
> _Status: the pure compute core (`model_registry::data_view::data_quality` →
> `DataQualityReport{gaps,dupes,outliers,bar_count,coverage_pct}`, robust-MAD
> outliers with a std fallback) is **landed and unit-tested** (planted gap + dupe +
> spike). The `GET /api/models/data/quality` endpoint wiring remains (folds into
> I-0.11)._

### ☐ I-0.8 Leakage guard on the data view — M
Make the guard structural: `DataView` and `materialize` take `as_of` and it is
**not optional**; any code path that would read `available_time > as_of` returns an
error, not data. Document the one rule (ADR-0008) at the call sites. Forbid the
sidecar from issuing its own bar queries — it receives only pre-windowed Parquet.
**Acceptance:** a test that asks for a bar past `as_of` gets an `Err`, never a value; grep confirms the trainer sidecar has no ClickHouse client.
> _Status: the structural guard is **half-landed** — `DataView::bars` takes a
> non-optional `AsOf` and `guard_as_of` returns `Err` on any future bar
> (unit-tested). The second half — removing `apps/model-trainer/app/clickhouse.py`
> so the sidecar can only consume pre-windowed Parquet — depends on the
> materialization rewrite (I-0.5/I-0.10) and is **not yet done**; the sidecar still
> holds its own ClickHouse client today._

### ☐ I-0.9 Automated leakage test harness — M
Add a reusable harness (Rust integration test + a sidecar self-check) that, for any
training spec, (a) plants a synthetic future bar and asserts it is unreachable
through the data view, and (b) trains a deliberately-leaky variant (target shifted
+1 instead of −H) and asserts the eval suite flags impossibly-good scores. This is
the leakage test that D-6 mandates in **every** pipeline.
**Acceptance:** the harness passes on a correct spec and **fails** on the planted-leak variant; it is invoked from the train path so no run skips it.

### ☐ I-0.10 Wire walk-forward into trainer dispatch — M
Replace the single ordinal `engine.split_indices` path: Rust computes folds
(I-0.3) over the pinned dataset and passes `folds: Vec<Fold>` in the train dispatch
(`sidecar::TrainDispatchRequest`); the sidecar trains/scores **per fold** using the
provided index ranges and returns per-fold + aggregated metrics. Keep the
"single expanding fold" default so the isolated-train path (spec §4) still works.
**Acceptance:** a walk-forward train run returns per-fold metrics for N folds; a `cv`-less spec still trains as one fold; the sidecar never computes its own split.

### ☐ I-0.11 REST + WS surface for windows, DQ, materialization — S
Add `POST /api/models/data/windows` (preview computed folds for a spec),
surface materialization status on the existing `models.jobs` WS lane, and return
`dataset_version` summaries (row_count, span, hash) from the runs API. Additive to
the Set H contract.
**Acceptance:** the windows endpoint echoes fold boundaries for a spec; a materialize job streams progress on `models.jobs`; OpenAPI/contract doc updated.

---

## Phase 0 exit criteria

- A training run consumes a **real, pinned, hashed** dataset (no `row_count = 0`).
- Walk-forward folds (train/cal/test, expanding|rolling, purge+embargo) are computed
  in Rust and honored by the sidecar; the calibration role exists and is isolated.
- Every data view is `as_of`-bounded; a request for a future bar errors.
- The leakage test harness runs on the train path and fails a planted leak.
- Data-quality preview is available before training.
- `cargo test -p domain -p features -p model-registry` green; `just lint`,
  `just fmt-check`, `just check-money` green (no `Price`/`Size` f64 introduced).
