# Agent Query — Delete Redis from the Write Path; PnL Lot Move Semantics
## Covers Issues: #8, #31, #35, #36
## Phase: D
## Estimated Effort: 1 week
## Prerequisites: #6/#7 (deterministic xxh3 IDs from agent-07 are needed for the in-process ring deduplicator)

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

The storage writer calls Redis `mark_seen` for every event in a sequential loop during each flush cycle, causing up to 10,000 network round-trips per 100 ms flush. Each round-trip adds ~1 ms of Redis latency, meaning a 10,000-event flush takes 10 seconds in the worst case — far exceeding the 100 ms flush budget. This is the primary cause of write-path backpressure. Additionally, `raw_json` bytes are cloned per row during the batch write loop, doubling memory for in-flight batches. On the PnL side, `PnlLot` is unnecessarily cloned twice when archiving closed lots.

## Codebase Context

- `crates/storage/src/writer.rs` — around lines 115–141: `raw_json.clone()` per row (line 115); sequential `mark_seen` call per event in a loop (lines 134–141). The `StorageWriter` struct holds an `Arc<Mutex<RedisClient>>` field.
- `crates/storage/src/pnl.rs` — around lines 72, 99, 103: `PnlLot.side` stored as `String` (line 72); `PnlLot` cloned on VecDeque push (line 99) and again on archive insert (line 103).
- ClickHouse DDL files — event tables currently use `MergeTree` without deduplication semantics.
- `Cargo.toml` — `ahash` may or may not be a dependency.

The problematic Redis loop (issue #8):
```rust
// writer.rs ~lines 134-141 — runs once per event in every flush
for event in &batch {
    self.redis.mark_seen(&event.event_id).await?;  // ← network round-trip per event
}
```

The raw_json clone (issue #8):
```rust
// writer.rs ~line 115
let row = Row { id: compute_id(), data: event.raw_json.clone() }; // ← clone per row
```

The PnL issues:
```rust
// pnl.rs ~line 72
pub struct PnlLot {
    pub side: String,   // ← should be Side enum
    // ...
}
// pnl.rs ~line 99-103
self.active_lots.push_back(lot.clone());  // ← clone #1
self.archive.push(lot.clone());           // ← clone #2
```

## Task

### Fix #8 — Delete Redis mark_seen from flush path; in-process dedup ring

**Problem:** The sequential per-event Redis `mark_seen` loop is the bottleneck of the storage write path. A 10,000-event flush with ~1 ms Redis RTT = 10 seconds. The flush budget is 100 ms.

**Solution:** Remove Redis from the flush path entirely. Replace with an in-process `AHashSet<u128>` deduplication ring (holds the last 1,000,000 event IDs as u128 values from xxh3_128). Use ClickHouse `ReplacingMergeTree` for convergent deduplication of late-arriving duplicates.

**Implementation steps:**

1. Remove the `Arc<Mutex<RedisClient>>` (or `Arc<RedisClient>`) field from `StorageWriter`. Remove all imports of the Redis client in `crates/storage/src/writer.rs`.

2. Add `ahash = "0.8"` to workspace `Cargo.toml` if not already present.

3. Add an in-process dedup ring to `StorageWriter`:
   ```rust
   use ahash::AHashSet;

   pub struct StorageWriter {
       // ... other fields, minus redis_client ...
       seen_ids: AHashSet<u128>,      // holds last ~1M event IDs
       seen_ring: VecDeque<u128>,     // eviction order (FIFO)
       seen_capacity: usize,           // default: 1_000_000
   }
   ```

4. In the flush loop, replace the `mark_seen` call with the in-process check:
   ```rust
   fn dedup_check(&mut self, event_id: u128) -> bool {
       if self.seen_ids.contains(&event_id) {
           return false;  // duplicate, skip
       }
       // Evict oldest if at capacity
       if self.seen_ring.len() >= self.seen_capacity {
           if let Some(oldest) = self.seen_ring.pop_front() {
               self.seen_ids.remove(&oldest);
           }
       }
       self.seen_ids.insert(event_id);
       self.seen_ring.push_back(event_id);
       true
   }
   ```
   This runs in ~50 ns (hash lookup) vs ~1 ms (Redis RTT).

5. Fix the `raw_json.clone()` at writer.rs:115. Change the batch-write loop to group rows by table index and write using byte-slice references:
   ```rust
   // Before: let row = Row { data: event.raw_json.clone() };
   // After: write directly from the batch buffer using indices
   let mut by_table: HashMap<TableId, Vec<usize>> = HashMap::new();
   for (i, event) in batch.iter().enumerate() {
       by_table.entry(event.table_id).or_default().push(i);
   }
   for (table_id, indices) in &by_table {
       let rows: Vec<&[u8]> = indices.iter().map(|&i| &batch[i].raw_json[..]).collect();
       clickhouse_client.insert_raw(table_id, &rows).await?;
   }
   ```
   This passes references into the batch buffer — no clone.

6. Update ClickHouse DDL for all market event tables: change `ENGINE = MergeTree()` to `ENGINE = ReplacingMergeTree(version)` with `ORDER BY (event_id, timestamp_ns)`. This provides convergent deduplication on merge — any duplicate events that slip through (e.g., from crash recovery) are deduplicated by ClickHouse's merge process.
   - Find existing DDL files in the repository (likely in `crates/storage/migrations/` or `scripts/clickhouse/`).
   - Apply the engine change to all event/trade/bar/quote tables.

7. Restrict Redis scope: Redis now only serves the UI last-value cache (latest price per instrument, latest position per instrument for the dashboard). Add a top-of-file comment in `crates/storage/src/writer.rs`:
   ```rust
   // Redis is intentionally absent from the write path.
   // Deduplication uses the in-process AHashSet ring.
   // Redis is only used by crates/ui-gateway for last-value cache.
   ```

8. Update the Parquet replay reader (if it exists in `crates/storage/src/replay.rs` or similar): add deduplication on `event_id` using a `HashSet<u128>` when reading Parquet files for replay. Document with a comment.

**Criterion benchmark:** Add a `benches/flush.rs` benchmark using `criterion` that flushes 10,000 events and measures wall time. Target: < 10 ms total flush time.

### Fix #31 — PnlLot.side as Side enum

**Problem:** `crates/storage/src/pnl.rs:72` — `PnlLot.side` is stored as `String` ("buy"/"sell"). This is wasteful — the `Side` enum already exists in `crates/domain`.

**Solution:** Change `PnlLot.side` to the `Side` enum type. Remove the `String` conversion at every construction site.

**Implementation steps:**

1. In `crates/storage/src/pnl.rs`, change the field:
   ```rust
   // Before:
   pub side: String,
   // After:
   pub side: domain::Side,
   ```

2. At all `PnlLot` construction sites, remove `.to_string()` on the Side value:
   ```rust
   // Before:
   side: fill.side.to_string(),
   // After:
   side: fill.side,
   ```

3. At all display/serialization sites (e.g., ClickHouse row construction), call `lot.side.as_str()` or use the `Display` impl — no per-use allocation.

### Fix #35 + #36 — PnlLot Arc-wrapping (both issues same fix)

**Problem:** `crates/storage/src/pnl.rs:99,103` — `PnlLot` is cloned once when pushed to `active_lots: VecDeque` and again when pushed to `archive`. Each clone copies the entire struct including its `Decimal` fields.

**Solution:** Wrap `PnlLot` in `Arc<PnlLot>`. Cloning an `Arc` is a refcount bump (~1 ns), not a struct copy.

**Implementation steps:**

1. Change the container types:
   ```rust
   // Before:
   active_lots: VecDeque<PnlLot>,
   archive: Vec<PnlLot>,
   // After:
   active_lots: VecDeque<Arc<PnlLot>>,
   archive: Vec<Arc<PnlLot>>,
   ```

2. At construction time, wrap once:
   ```rust
   let lot = Arc::new(PnlLot { side: fill.side, /* ... */ });
   self.active_lots.push_back(Arc::clone(&lot));
   self.archive.push(lot);
   ```
   Zero struct clones. Two `Arc::clone` calls (~2 ns total).

3. At all read sites (iterating `active_lots`, accessing `archive`), use `&**arc_lot` or `arc_lot.as_ref()` to get `&PnlLot`.

**Acceptance test:**
- Write a benchmark (`benches/flush.rs`) that inserts 10,000 unique events and 100 duplicate events. Verify: (a) the flush completes in < 10 ms, (b) duplicates are detected by the in-process ring and not written to ClickHouse (mock ClickHouse for this test).
- Write a unit test for `PnlLot` that constructs a lot, pushes to active_lots and archive, and verifies zero `.clone()` calls on the `PnlLot` struct (use a `Drop` counter or `Arc::strong_count` to verify refcount behavior).
- `grep -n "mark_seen" crates/storage/src/writer.rs` must return zero results.

## Overall Acceptance Criteria
- [ ] Zero Redis calls in `writer_task` flush path (`grep "mark_seen\|redis" crates/storage/src/writer.rs` returns zero)
- [ ] Flush of 10,000 events completes in < 10 ms (criterion benchmark passes)
- [ ] In-process AHashSet dedup ring in StorageWriter (capacity: 1,000,000 IDs)
- [ ] ClickHouse DDL updated to `ReplacingMergeTree` for all event tables
- [ ] `raw_json` bytes not cloned per row in flush loop (references passed instead)
- [ ] `PnlLot.side` is `Side` enum, not `String`
- [ ] `PnlLot` is `Arc<PnlLot>` in both active_lots and archive containers
- [ ] No `.clone()` calls on `PnlLot` struct (only `Arc::clone` for refcount)
- [ ] `cargo test` passes

## Files to Touch
- `crates/storage/src/writer.rs` — remove Redis field and mark_seen loop; add AHashSet dedup ring; fix raw_json clone
- `crates/storage/src/pnl.rs` — PnlLot.side as Side enum; Arc<PnlLot> in containers
- ClickHouse DDL files (likely `crates/storage/migrations/` or `scripts/clickhouse/`) — ReplacingMergeTree
- `crates/storage/benches/flush.rs` (new) — criterion benchmark for flush performance
- `Cargo.toml` — add `ahash = "0.8"` if not present; add `criterion` to dev-dependencies
