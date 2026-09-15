# Phase 5 — Data Pipeline & Observability

**Completion: 20% (1 / 5) — 5.5 shipped and was never ticked. **5.1 and 5.4 are the two worst unattended-paper blockers in the whole set** (re-audited 2026-09-15; see [MASTER](MASTER.md))**

**Goal:** Wire production visibility, complete the deferred data-pipeline
mechanics, and close the hot path so the ring pipeline actually runs strategies.
**Addresses:** #14, #15, #16, #21, #26.

> Two items here are **producer-ahead-of-consumer** (5.2 compaction has no
> replay reader; 5.3 order book has no feature engine) — they deliver full value
> only once their consumers exist, but completing them removes silent no-ops.
> 5.4 is the largest and most architectural item in the whole plan.

---

## Tasks

### ☐ 5.1 [OPEN — empty stubs, no dep, no route, zero call sites] — ⚠ **BLOCKS UNATTENDED PAPER** Metrics registry + `/metrics` exporter + call sites — M
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

### ☐ 5.2 Parquet compaction + nightly scheduler — M
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

### ☐ 5.3 `OrderBookBuilder` (L2 reconstruction) — M
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

### ☐ 5.4 [OPEN — `hot_path.rs:333` is still `Option::None`; the pipeline emits zero intents] — ⚠ **BLOCKS UNATTENDED PAPER** Hot-path stage-3 strategy integration — L — **largest**
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
