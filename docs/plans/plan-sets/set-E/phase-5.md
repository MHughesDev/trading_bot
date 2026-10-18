# Phase 5 — Data Pipeline & Observability

**Completion: 60% (3 / 5 — 5.3 DONE 2026-10-03; 5.1 and 5.2 PARTIAL, each waiting on `main.rs` wiring named in its block)**

**Goal:** Wire production visibility, complete the deferred data-pipeline
mechanics, and close the hot path so the ring pipeline actually runs strategies.
**Addresses:** #14, #15, #16, #21, #26.

> Two items here are **producer-ahead-of-consumer** (5.2 compaction has no
> replay reader; 5.3 order book has no feature engine) — they deliver full value
> only once their consumers exist, but completing them removes silent no-ops.
> 5.4 is the largest and most architectural item in the whole plan.

---

## Tasks

### ◑ 5.1 [PARTIAL — registry, exporter and execution call sites done; exporter start + pipeline call sites await other owners] — ⚠ **BLOCKS UNATTENDED PAPER** Metrics registry + `/metrics` exporter + call sites — M

> **2026-10-03 (track PROD).** `observability::metrics`: a process-wide
> registry (counters, gauges, histograms; Prometheus text format 0.0.4) with a
> per-family cardinality cap (256 series → an `{overflow="true"}` series, counted)
> and a loopback-only `GET /metrics` listener, `observability::init_metrics(addr)`
> (one per binary, like `init`). `Metrics` and `CorrectnessMetrics` keep their
> signatures and now record. **ADR-E-5.1:** no `metrics`/`prometheus` crate — the
> registry is ~200 lines and a new dependency tree would rewrite the shared
> lockfile for every crate; rejected alternative: `metrics-exporter-prometheus`.
> Live call sites: `execution_orders_total{outcome}`, `execution_fills_total{path}`,
> `execution_audit_write_failures_total`, `paper_state_saves_total{outcome}`,
> `paper_orders_refused_poisoned_total{asset_class}`. Tests:
> `observability::metrics::tests` (6, incl. a real HTTP scrape and the
> non-loopback refusal) + `correctness::tests` (1); crate total 49 pass.
> **Wave 2 (2026-10-03):** the exporter is started (`main.rs` calls
> `observability::init_metrics` on `127.0.0.1:9100`). Pipeline call sites:
> `event_bus::Publisher::{publish, publish_fire_and_forget}` →
> `increment_published{lane}` (+ `pipeline_publish_failures_total`),
> `QuarantinePublisher::publish_failure` → `increment_quarantined{source}`, and
> `collectors::gap::GapDetector::check` → `increment_gap_detected{lane}` through
> `event_bus::observe::gap_detected` (the one place every collector's gaps pass
> through; `collectors` has no `observability` dependency of its own and its
> manifest is another track's). Tests: `event_bus::observe::tests`,
> `observability::metrics::tests`.
> **Still missing, with the exact lines:** `record_reconciliation_divergence` has
> no call site — `crates/reconciliation` is another track's crate. In
> `crates/reconciliation/Cargo.toml` add `observability = { workspace = true }`;
> in `crates/reconciliation/src/divergence.rs`, inside the `Diverged` arm of
> `check_position_divergence` (after the `error!(...)`), add
> `observability::metrics::inc("reconciliation_divergences_total", "Position divergences that halted trading.", &[]);`
> and the same one line in the venue-scoped form. Status stays PARTIAL until then.
**Addresses #14, #15 (CL metrics, correctness).** `metrics.rs`/`correctness.rs`
are no-op structs with **no call sites anywhere**; no `prometheus`/`metrics` dep;
no `/metrics` endpoint.
- Add `metrics` + `metrics-exporter-prometheus` to `observability/Cargo.toml`;
  add `observability::init_metrics(addr)` installing a `PrometheusBuilder`
  recorder + HTTP listener (per-binary, matching the existing `init` pattern
  since collectors are separate processes).
- Reimplement the stubs as drop-in `counter!`/`gauge!`/`histogram!` calls
  (same signatures). Then add real call sites: publish/quarantine paths, gap
  detector (`collectors/src/gap.rs`), and the three correctness signals in
  reconciliation/consumer-lag code.
- Cardinality policy: label by lane/venue/source only — **never** instrument_id.
  Keep recording off the rtrb hot-path stages.
- **Files:** `crates/observability/src/{lib,metrics,correctness}.rs` + `Cargo.toml`,
  call sites across collectors/reconciliation.
- **Verify:** `/metrics` serves Prometheus text with non-zero counters after a
  publish/quarantine/gap event.

### ◑ 5.2 [PARTIAL — compaction and scheduler built; the scheduler spawn awaits `main.rs`, and nothing writes the archive yet] Parquet compaction + nightly scheduler — M

> **2026-10-03 (track PROD).** `storage::parquet::compaction`: merge every
> parquet file of a **closed** (`date` < today UTC) partition into one, crash-safe
> by temp file + manifest + rename + delete-last, with `recover_partition`
> finishing or rolling back an interrupted pass; mismatched schemas are refused
> untouched. `compact_closed_partitions` walks `base/events/**/date=*/`;
> `spawn_nightly(base, hour_utc)` is the scheduler. Tests (4): merge with
> identical rows; crash after publish → no duplicates after recovery; crash
> before publish → no loss; today's partition untouched.
> **Wave 2 (2026-10-03) — the writer had no production caller, confirmed by a
> repository-wide search; it now has one to wire.** `StorageWriter` (the only
> writer of the raw Parquet archive) was reachable only from its own file.
> Built: `StorageWriter::archive_only(parquet)` (the unused Postgres/ClickHouse
> handles `new` took are no longer required), `RawEvent::from_envelope` (JSON of
> the envelope, partitioned by lane/venue/instrument and the **exchange**-time UTC
> date, id `venue/instrument/source/sequence` for the dedup ring; an envelope
> that cannot be named or dated is dropped and counted, never put in a guessed
> partition), and `apps/platform/src/tee.rs::run_tee_archived` which submits each
> tee'd tick non-blockingly (`archive_dropped_total{reason}`). `run_tee` keeps its
> signature. Tests: `writer::from_envelope_tests`, `compaction::tests` (4).
> **Not live until `main.rs` changes** (orchestrator's file; exact lines in the
> track report): replace `tokio::spawn(tee::run_tee(publisher, tee_rx))` with
> `run_tee_archived(publisher, tee_rx, Some(archive))`, build `archive` from
> `StorageWriter::archive_only(Arc::new(ParquetWriter::new(<dir>)))`, and call
> `storage::parquet::compaction::spawn_nightly(<dir>, 3)`. The archive directory
> is a config decision (there is none today); the tee only runs when NATS is
> connected, so the archive is likewise empty without it. Status stays PARTIAL.
**Addresses #18 (CL compaction).** `compact_partition` logs a warn and returns
`Ok(())`; no caller exists.
- Enumerate `*.parquet` in the partition dir (skip in-progress temp); if <2
  files, return early. Stream each file's `RecordBatch`es through `ArrowWriter`
  into `compacted-{uuid}.tmp` (in `spawn_blocking`, mirroring `mod.rs:77-90`),
  `fs::rename` to final, then delete sources **only after** the rename succeeds.
  Never reorder/dedupe (dedup already happened at write via `DedupRing`); only
  compact **closed/past** date partitions (never today's, which the writer is
  still appending to).
- Add a nightly invoker (tokio interval in `apps/platform`) walking
  `base_path/events/**/date=*/`.
- **Files:** `crates/storage/src/parquet/compaction.rs`, `apps/platform` scheduler.
- **Verify:** a dir of small files merges to one with identical row content; a
  simulated mid-compaction crash loses no rows (temp+rename+delete-last).

### ☑ 5.3 [DONE 2026-10-03 — no consumer yet, as the phase header predicted] `OrderBookBuilder` (L2 reconstruction) — M

> **2026-10-03 (track PROD).** `builders::orderbook::OrderBookBuilder`: snapshot
> rebuilds; a delta applies only at `last_sequence + 1`; a gap clears the book to
> `NeedsSnapshot` and refuses deltas until a snapshot (never applied blindly);
> redeliveries are `Stale`; `is_tentative` updates are refused without moving the
> sequence (policy); a crossed book forces a resync. `best_bid/ask`, `mid`,
> `spread`, `depth(n)`. Pure. Tests: `orderbook::tests` (4); crate 8 pass. No
> production consumer exists (no feature engine reads L2) — producer-ahead-of-
> consumer, as this phase's header records.
**Addresses #16 (CL orderbook).** Empty struct with only `new()`/`Default`.
- Model after `BarState`: `BTreeMap<Price, Size>` bids (desc) / asks (asc) +
  `last_sequence`. `feed_update(&OrderBookPayload)`: `Snapshot` clears+rebuilds;
  `Delta` checks `sequence` contiguity (gap ⇒ force resync, never apply blindly),
  then sets/removes levels (size 0 = remove). Expose `best_bid`/`best_ask`/`mid`/
  `spread`/`depth(n)`. Pure, no I/O (same code live + replay). Define an
  `is_tentative` (low-confirmation on-chain) policy.
- **Files:** `crates/builders/src/orderbook.rs`.
- **Verify:** snapshot→deltas reconstructs a known book; a sequence gap forces
  resync rather than silent corruption.

### ☑ 5.4 Hot-path stage-3 strategy integration — L — **largest** — **DONE 2026-09-15**
**Addresses #26 (NF).** `stage_strategy_eval` declares
`let mut strategy: Option<StrategyInstance> = None;` (`hot_path.rs:153`) — it
pops `WorldEvent`s and produces **no intents**. The machinery is complete
(`StrategyInstance::process_event`, `InstanceManager::dispatch`) but the API owns
the single `Arc<Mutex<InstanceManager>>`, and `spawn_pipeline` takes no strategy
handle; demand-manager uses `NoopPipelineFactory`.
- **Locked decision 12: arc-swap snapshot.** Stage 3 reads an `Arc` snapshot of
  the compiled per-instrument instances; the API swaps it atomically on
  add/remove. Lock-free reads — **no lock or allocation per `WorldEvent`**.
- Thread the arc-swapped instance handle into `spawn_pipeline`; in stage 3,
  iterate the current snapshot calling `process_event` and push intents to
  `ring_intent` (the body of `dispatch` minus the per-event `Mutex`). Replace the
  stage-4 dummy `GateContext` (`hot_path.rs:184`) with the real one from 2.1/2.3.
- **Files:** `apps/platform/src/hot_path.rs`, `crates/demand-manager`,
  `crates/api/src/state.rs`, `crates/strategy-runtime/src/runtime.rs`.
- **Verify:** an end-to-end paper run where a loaded strategy emits an intent
  through the ring pipeline and reaches the gate; no per-event lock (bench/assert).
- **Verified by:** `hot_path::tests::a_hand_authored_strategy_reaches_a_persisted_fill`
  (intent → real gate → paper fill → journal) and
  `hot_path::tests::resync_keeps_warm_instances_and_adds_new_ones`.
- **Landed as:** a generation-counter registry (`strategy_runtime::live`) rather
  than `arc-swap` — same lock-free property (one relaxed atomic load per
  `WorldEvent`, the registry read only when the generation moved), no new
  dependency, and instances are owned by stage 3 because they carry mutable
  per-instrument state and cannot be shared. Declared deviation from locked
  decision 12 as written. The demand-manager keeps `NoopPipelineFactory`: the
  pipeline lifecycle is already owned by `PipelineManager` + `StreamRequest`,
  and a second factory would be a second lifecycle owner. See **ADR-0038**.
- **Stage 4’s dummy `GateContext` is also gone** (it was in scope here and is
  the reason this task could not land alone): it now asks
  `risk::context::context_for`, the one provider both order doors use.
- **Note:** meaningful signals also depend on the proper OHLCV aggregator
  (set-C #3/#24); the stage-2 one-bar-per-tick shim limits fidelity until that
  lands.

### ☑ 5.5 [DONE — 681-line `crates/mcp-server/tests/integration.rs`, 17 tests, run by CI] MCP server tests — S–M
**Addresses #21 (CL mcp tests).** 5 source files, 0 tests. After 0.3 makes
discovery query real registries, add tests for tool discovery, lane queries, and
instrument enumeration against seeded platform state.
- **Files:** `crates/mcp-server/tests/`.
- **Verify:** discovery tools return the same data as the `/api` routes;
  coverage exercised in CI.

---

## Definition of Done
`/metrics` exposes real counters with live call sites; nightly compaction merges
closed partitions crash-safely; the order book reconstructs from snapshot+deltas
with gap detection; the hot path runs loaded strategies and emits intents through
a lock-free hand-off into a real risk-gate context; and the MCP server has test
coverage.
