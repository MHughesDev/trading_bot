# Agent Query — Replace Async Mutex in Registries with DashMap + Arc<str> IDs
## Covers Issues: #22, #29, #30, #34, #40, #60, #66
## Phase: D
## Estimated Effort: 3–5 days
## Prerequisites: None (fully independent of Phase A/B/C; #29 is the easiest and can land immediately)

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

Six separate registries across venue-router, demand-manager, ui-gateway, and storage use `Arc<Mutex<HashMap>>` or tokio async `Mutex` with `.lock().await` on every read and write. Under multi-strategy load (100 strategy instances, 20 instruments, 10 collectors) these become contention points: async Mutex `.lock().await` suspends the current task and wakes it when the lock is available, causing unnecessary task switches. All six can be replaced with `dashmap::DashMap` — a sharded concurrent HashMap with lock-free reads under most access patterns — plus `Arc<str>` key interning to eliminate string clones on every registry operation.

## Codebase Context

- `crates/demand-manager/src/registry.rs` — around lines 53, 75: lane ID and instrument ID Strings cloned on every `add`/`remove` operation. Lines 55, 66, 77, 102: multiple `.lock().unwrap()` per operation. Uses `Arc<Mutex<HashMap<String, DemandEntry>>>`.
- `crates/venue-router/src/lifecycle.rs` — around lines 42, 78–80: three `.to_owned()` calls to build a composite collector key on every start/stop event.
- `crates/venue-router/src/registry.rs` — around lines 18–19, 36, 46, 61, 75: tokio async Mutex on every demand change and lifecycle event; all reads and writes await the lock.
- `crates/storage/src/writer.rs` — around line 42: `Arc<Mutex>` held during batch writes, serializing all writers behind a single lock.
- `crates/ui-gateway/src/subscriptions.rs` — around lines 96, 102, 114, 121: 3–4 lock acquisitions per subscription operation.
- `Cargo.toml` — `dashmap` may not yet be a dependency.

The problematic async Mutex pattern in venue-router registry:
```rust
// registry.rs ~line 36 — suspends task on every demand increment
pub async fn increment(&self, key: CollectorKey) -> u32 {
    let mut map = self.inner.lock().await;  // ← task switch on every call
    *map.entry(key).or_insert(0) += 1;
    map[&key]
}
```

The problematic string clone pattern in demand-manager:
```rust
// registry.rs ~line 53
pub fn add_demand(&self, lane: &str, instrument: &str) {
    let mut map = self.inner.lock().unwrap();
    map.entry(lane.to_owned())                   // ← to_owned() on every call
       .or_default()
       .insert(instrument.to_owned(), DemandEntry::new());  // ← to_owned() again
}
```

## Task

### Fix #29 + #30 — Demand registry: Arc<str> keys + DashMap

**Problem:** `crates/demand-manager/src/registry.rs` clones lane and instrument ID strings as HashMap keys on every registry operation, and uses a blocking `Mutex` for all access.

**Solution:** Replace `Arc<Mutex<HashMap<String, DemandEntry>>>` with `Arc<DashMap<Arc<str>, DemandEntry>>`. Use `Arc<str>` as the key type so clone is a refcount bump, not a heap allocation.

**Implementation steps:**

1. Add `dashmap = "6"` to workspace `Cargo.toml`.

2. Change the registry field type:
   ```rust
   // Before:
   inner: Arc<Mutex<HashMap<String, DemandEntry>>>,
   // After:
   inner: Arc<DashMap<Arc<str>, DemandEntry>>,
   ```

3. Update all function signatures to accept `lane: Arc<str>, instrument: Arc<str>` (or `lane: &Arc<str>` for by-reference access). Callers are expected to hold `Arc<str>` values interned at startup (e.g., from the intern table added in agent-02, or a local intern map in the demand manager).

4. Replace all `self.inner.lock().unwrap()` and `self.inner.lock().await` with direct DashMap method calls:
   ```rust
   // Before:
   let mut map = self.inner.lock().unwrap();
   map.entry(lane.to_owned()).or_default().insert(...);
   // After:
   self.inner.entry(Arc::clone(&lane)).or_default().insert(Arc::clone(&instrument), DemandEntry::new());
   ```
   DashMap's `entry` API is lock-free for non-contended shards.

5. Remove all `.lock()` / `.lock().await` / `.lock().unwrap()` from `registry.rs`. If any remain (e.g., for a statistics snapshot), document why.

### Fix #34 + #42 — Venue router: typed CollectorKey struct

**Problem:** `crates/venue-router/src/lifecycle.rs` (around lines 42, 78–80) builds collector registry keys using three `.to_owned()` calls: `lane.to_owned() + ":" + instrument.to_owned() + ":" + venue.to_owned()`. This allocates a new String on every start/stop event.

**Solution:** Create a typed `CollectorKey` struct that holds `Arc<str>` values. Since `Arc<str>` clone is a refcount bump, no String allocation occurs when constructing or cloning keys.

**Implementation steps:**

1. In `crates/venue-router/src/lifecycle.rs` or a new `crates/venue-router/src/key.rs`, define:
   ```rust
   #[derive(Debug, Clone, PartialEq, Eq, Hash)]
   pub struct CollectorKey {
       pub lane:       Arc<str>,
       pub instrument: Arc<str>,
       pub venue:      Arc<str>,
   }

   impl CollectorKey {
       pub fn new(lane: Arc<str>, instrument: Arc<str>, venue: Arc<str>) -> Self {
           Self { lane, instrument, venue }
       }
   }
   ```

2. Update `lifecycle.rs` to construct `CollectorKey` from already-interned `Arc<str>` values (held in the lifecycle manager's configuration at startup). Remove all `.to_owned()` calls for key construction.

3. Change the registry HashMap key type from `String` to `CollectorKey`.

### Fix #60 + #66 — CollectorRegistry: tokio async Mutex → DashMap

**Problem:** `crates/venue-router/src/registry.rs` (around lines 18–19, 36, 46, 61, 75) uses `Arc<tokio::sync::Mutex<HashMap<CollectorKey, u32>>>`. Every demand change and lifecycle event awaits this async Mutex, causing unnecessary task suspensions even under low contention.

**Solution:** Replace with `Arc<DashMap<CollectorKey, u32>>`. All operations become synchronous and non-blocking.

**Implementation steps:**

1. Change the registry field:
   ```rust
   // Before:
   inner: Arc<tokio::sync::Mutex<HashMap<CollectorKey, u32>>>,
   // After:
   inner: Arc<DashMap<CollectorKey, u32>>,
   ```

2. Replace `increment` and `decrement` async functions with synchronous equivalents:
   ```rust
   // Before (async, task-suspending):
   pub async fn increment(&self, key: CollectorKey) -> u32 {
       let mut map = self.inner.lock().await;
       let v = map.entry(key).or_insert(0);
       *v += 1;
       *v
   }
   // After (sync, non-blocking):
   pub fn increment(&self, key: CollectorKey) -> u32 {
       let mut entry = self.inner.entry(key).or_insert(0);
       *entry += 1;
       *entry
   }
   ```

3. Remove `async` and `.await` from all callers of these methods in `lifecycle.rs`. Since the methods are now sync, the callers' `async fn` context is preserved but no longer awaits the registry.

### Fix #22 — Storage writer: channel-based ownership instead of Mutex

**Problem:** `crates/storage/src/writer.rs:42` — `Arc<Mutex>` held during batch writes, serializing all access.

**Solution:** Move the shared mutable state into a single `writer_task` tokio task. The task owns all mutable data exclusively — no Mutex needed. External callers send commands via an `mpsc` channel.

**Implementation steps:**

1. Define a command enum:
   ```rust
   pub enum WriterCommand {
       Flush(Vec<RawEvent>),
       Shutdown,
   }
   ```

2. Create the writer task:
   ```rust
   pub fn spawn_writer(config: WriterConfig) -> WriterHandle {
       let (tx, mut rx) = tokio::sync::mpsc::channel::<WriterCommand>(1024);
       tokio::spawn(async move {
           let mut state = WriterState::new(config);  // owns all mutable data
           while let Some(cmd) = rx.recv().await {
               match cmd {
                   WriterCommand::Flush(batch) => state.flush(batch).await,
                   WriterCommand::Shutdown => break,
               }
           }
       });
       WriterHandle { tx }
   }
   ```

3. Callers (collectors, the event bus consumer) send `WriterCommand::Flush(batch)` via the channel — no lock acquisition.

4. Remove the `Arc<Mutex>` from `StorageWriter`.

### Fix #40 — UI gateway subscription map: DashMap

**Problem:** `crates/ui-gateway/src/subscriptions.rs` (around lines 96, 102, 114, 121): 3–4 lock acquisitions per subscription operation on a `HashMap<Uuid, Subscription>` behind a `Mutex` (or `RwLock`).

**Solution:** Replace with `DashMap<Uuid, Subscription>`. All reads and writes become lock-free under DashMap's sharded model.

**Implementation steps:**

1. Change the subscription store type:
   ```rust
   // Before:
   subscriptions: Arc<Mutex<HashMap<Uuid, Subscription>>>,
   // After:
   subscriptions: Arc<DashMap<Uuid, Subscription>>,
   ```

2. Replace all `.lock()` / `.read()` / `.write()` acquisition chains with direct DashMap method calls (`insert`, `remove`, `get`, `entry`).

3. For operations that previously required holding the lock across multiple steps (e.g., check-then-insert), use DashMap's `entry` API which provides the same atomicity:
   ```rust
   self.subscriptions.entry(id).or_insert_with(|| Subscription::new(id, ...));
   ```

**Acceptance test:**
- Write a concurrent test that spawns 20 tokio tasks each calling `increment`/`decrement` on the `CollectorRegistry` 1,000 times concurrently. Verify the final count is correct and no deadlock occurs.
- `grep -n "lock().await\|lock().unwrap" crates/demand-manager/src/registry.rs` must return zero results.
- `grep -n "lock().await\|lock().unwrap" crates/venue-router/src/registry.rs` must return zero results.
- `grep -n "to_owned" crates/venue-router/src/lifecycle.rs | grep -v "//"` must return zero results for key construction.

## Overall Acceptance Criteria
- [ ] Zero `.lock().await` calls in demand-manager, venue-router, and collector-registry hot paths
- [ ] `DashMap` used for all concurrent map access in the four affected crates
- [ ] Demand registry key type is `Arc<str>`, not `String.clone()` on each operation
- [ ] `CollectorKey` is a typed struct with `Arc<str>` fields, not a String concatenation
- [ ] Storage writer uses channel-based ownership instead of `Arc<Mutex>`
- [ ] UI gateway subscription map is a `DashMap<Uuid, Subscription>`
- [ ] Concurrent registry test passes (20 tasks, 1,000 ops each, correct final count)
- [ ] `cargo test` passes for all affected crates

## Files to Touch
- `crates/demand-manager/src/registry.rs` — DashMap; Arc<str> keys; remove all .lock() chains
- `crates/venue-router/src/lifecycle.rs` — remove .to_owned() for key construction; use CollectorKey
- `crates/venue-router/src/key.rs` (new, or add to lifecycle.rs) — CollectorKey struct with Arc<str> fields
- `crates/venue-router/src/registry.rs` — DashMap; sync increment/decrement; remove async Mutex
- `crates/storage/src/writer.rs` — channel-based ownership pattern; remove Arc<Mutex>
- `crates/ui-gateway/src/subscriptions.rs` — DashMap<Uuid, Subscription>; remove lock acquisition chains
- `Cargo.toml` — add `dashmap = "6"` to workspace dependencies
