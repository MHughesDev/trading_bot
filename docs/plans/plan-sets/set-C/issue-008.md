# Issue #008 — 10,000 sequential Redis round-trips per flush

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | D |
| Pattern | I/O |
| Quick Win | No |
| Latency Impact | Up to 10k awaited RTTs (≈ seconds) inside 100 ms flush budget |
| Location | `crates/storage/src/writer.rs:134-141` |

## Problem
`flush_batch` awaits `mark_seen` per event in a loop. A full batch of 10,000 events means 10,000 sequential Redis round-trips, each ~100 µs, totaling ~1 second inside what should be a 100 ms flush budget. This backs up the bounded channel, which eventually leaks backpressure toward the hot path.

## Root Cause
The deduplication strategy uses Redis as a seen-event store, with one `SETNX` call per event checked sequentially in the flush loop. This was added for correctness but the sequential await pattern eliminates all benefit of batching.

## Implementation Plan
### Step 1 — Delete Redis mark_seen from the write path
Remove the `mark_seen` call and the `Arc<Mutex<RedisClient>>` from `StorageWriter`. The flush loop no longer calls Redis.

### Step 2 — Make ClickHouse tables ReplacingMergeTree
Change ClickHouse DDL for event tables to use `ReplacingMergeTree(version)` keyed on `event_id`. Duplicate events written multiple times will be deduplicated on ClickHouse merge. Convergent correctness, no write-path overhead.

### Step 3 — Add in-process fast-path rejector
Add a fixed-capacity `ahash::HashSet<u128>` ring (holding the last ~1M event IDs) in the writer task. Before writing to ClickHouse, check the ring. If present, skip. If not, insert into ring and write. This catches hot duplicates without any network call.

### Step 4 — Fix writer.rs:115 grouping
Group rows by index (table) rather than cloning payload bytes. The row insertion should reference payload slices, not copies.

### Step 5 — Move Redis to UI latest-state only
Redis remains as a last-value cache for the UI gateway (latest price, latest position). Document this narrowed scope.

### Step 6 — Add Parquet replay dedup
The Parquet replay reader should dedup on read using deterministic IDs from #6 (xxh3_128). This handles the offline replay case without needing Redis.

## Acceptance Criteria
- [ ] Zero Redis calls in writer_task flush path
- [ ] Flush of 10,000 events completes in < 10 ms (benchmark with criterion)
- [ ] ClickHouse tables use ReplacingMergeTree
- [ ] In-process ahash ring rejector holds last 1M event IDs
- [ ] No backpressure propagation to hot path under sustained 10k-event batches

## Files to Change
- `crates/storage/src/writer.rs` — remove Redis mark_seen loop; add ahash ring rejector; fix row grouping at line 115
- ClickHouse DDL files — change to ReplacingMergeTree
