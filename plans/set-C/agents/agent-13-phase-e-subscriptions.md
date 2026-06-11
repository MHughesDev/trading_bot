# Agent Query — Arc<Subscription> Through the UI Gateway Subscription Lifecycle
## Covers Issues: #25, #26, #27, #28, #37, #48
## Phase: E
## Estimated Effort: 2–3 hours
## Prerequisites: None

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

The UI gateway subscription system manages WebSocket client subscriptions to market data streams. The subscription lifecycle — insert, remove, list — clones the full `Subscription` struct on every operation. With many concurrent WebSocket clients, each connecting and subscribing at startup, this creates a burst of allocation churn. The fix is simple: wrap `Subscription` in `Arc` once at creation time and share the `Arc` everywhere. All subsequent "copies" are refcount bumps (~1 ns each) instead of full struct copies. Six issues (#25, #26, #27, #28, #37, #48) all point to different clone sites in the same file — fixing `Arc<Subscription>` at the storage layer fixes all of them at once.

## Codebase Context

- `crates/ui-gateway/src/subscriptions.rs` — the entire subscription lifecycle is implemented here. Key problem locations:
  - Around lines 84–96: insertion path clones `Subscription` 2–3 times (issues #25, #37).
  - Around lines 102–108: removal path clones `Subscription` when it should just return the owned value (issue #48).
  - Around lines 112–148: remove-by-filter iterates to collect full `Subscription` clones, then iterates again to delete (issues #26, #27).
  - Around lines 152–157: list call clones every `Subscription` in the map (issue #28).

The problematic insertion pattern:
```rust
// subscriptions.rs ~lines 84-96
pub fn insert(&self, sub: Subscription) {
    let mut map = self.inner.lock().unwrap();
    map.insert(sub.id, sub.clone());          // ← clone #1 (issues #25, #37)
    self.by_instrument
        .entry(sub.instrument_id.clone())
        .or_default()
        .push(sub.clone());                   // ← clone #2
    self.by_panel
        .entry(sub.panel_id.clone())
        .or_default()
        .push(sub);                           // ← consumed here
}
```

The problematic list pattern:
```rust
// subscriptions.rs ~lines 152-157 (issue #28)
pub fn list(&self) -> Vec<Subscription> {
    self.inner.lock().unwrap()
        .values()
        .cloned()                             // ← clones every Subscription
        .collect()
}
```

## Task

### Fix #25 + #37 — Arc on insert (both issues same fix)

**Problem:** The insertion path clones the full `Subscription` struct 2–3 times to populate multiple indexes (primary map, instrument index, panel index). Issues #25 and #37 both refer to these clones.

**Solution:** Wrap `Subscription` in `Arc<Subscription>` at the call site. All subsequent index inserts clone the `Arc` (a refcount bump, ~1 ns) instead of the struct.

**Implementation steps:**

1. Change the primary storage type:
   ```rust
   // Before:
   inner: Arc<Mutex<HashMap<Uuid, Subscription>>>,
   by_instrument: Arc<Mutex<HashMap<String, Vec<Subscription>>>>,
   by_panel: Arc<Mutex<HashMap<String, Vec<Subscription>>>>,
   // After:
   inner: Arc<Mutex<HashMap<Uuid, Arc<Subscription>>>>,
   by_instrument: Arc<Mutex<HashMap<String, Vec<Arc<Subscription>>>>>,
   by_panel: Arc<Mutex<HashMap<String, Vec<Arc<Subscription>>>>>,
   ```
   Note: agent-10 changes these to `DashMap`. If agent-10 is done, skip the `Arc<Mutex<...>>` wrapper — just use `DashMap<Uuid, Arc<Subscription>>`.

2. Change `insert` to accept an `Arc<Subscription>`:
   ```rust
   pub fn insert(&self, sub: Arc<Subscription>) {
       let id = sub.id;
       let instrument_id = sub.instrument_id.clone();  // String clone of a field — still needed
       let panel_id = sub.panel_id.clone();            // Same
       let mut map = self.inner.lock().unwrap();
       map.insert(id, Arc::clone(&sub));               // ← refcount bump, not struct clone
       self.by_instrument
           .lock().unwrap()
           .entry(instrument_id)
           .or_default()
           .push(Arc::clone(&sub));                    // ← refcount bump
       self.by_panel
           .lock().unwrap()
           .entry(panel_id)
           .or_default()
           .push(sub);                                 // ← consumed (last use)
   }
   ```
   The caller creates the Arc once: `store.insert(Arc::new(subscription))`.

3. If `Subscription` has `String` fields like `instrument_id` and `panel_id` that are cloned for map keys, consider changing those fields to `Arc<str>` or using `InstrumentId(u32)` from agent-02. But for this issue, the primary win is wrapping `Subscription` itself in Arc — the field clones are a smaller secondary concern.

### Fix #26 + #27 — Remove path: collect Uuid, then delete (both issues same fix)

**Problem:** `crates/ui-gateway/src/subscriptions.rs` (around lines 112–148): the remove-by-filter path iterates the map to collect full `Subscription` clones (to identify which to remove), then iterates again to delete. Issues #26 and #27 both refer to this two-pass clone-heavy pattern.

**Solution:** First pass collects only `Uuid` values (8 bytes each, no heap). Second pass calls `map.remove(id)` for each collected ID. Zero `Subscription` struct copies.

**Implementation steps:**

1. Find the remove-by-instrument or remove-by-panel filter functions (around lines 112–148).

2. Replace the pattern:
   ```rust
   // Before: collects full Subscription clones to find IDs
   let to_remove: Vec<Subscription> = map.values()
       .filter(|s| s.instrument_id == instrument_id)
       .cloned()
       .collect();
   for sub in to_remove {
       map.remove(&sub.id);
   }
   // After: collect only IDs (cheap), then remove
   let ids_to_remove: Vec<Uuid> = map.iter()
       .filter(|(_, s)| s.instrument_id == instrument_id)
       .map(|(id, _)| *id)
       .collect();
   for id in ids_to_remove {
       map.remove(&id);
   }
   ```
   A `Uuid` is 16 bytes — the `Vec<Uuid>` is tiny. No `Subscription` clones needed.

3. Apply the same transformation to all filter-based remove functions in the file (by instrument, by panel, by user, etc.).

### Fix #28 — List returns Arc references

**Problem:** `crates/ui-gateway/src/subscriptions.rs` (around lines 152–157): `list()` clones every `Subscription` in the map.

**Solution:** Return `Vec<Arc<Subscription>>` — refcount bumps only.

**Implementation steps:**

1. Change the `list` function return type and implementation:
   ```rust
   // Before:
   pub fn list(&self) -> Vec<Subscription> {
       self.inner.lock().unwrap().values().cloned().collect()
   }
   // After:
   pub fn list(&self) -> Vec<Arc<Subscription>> {
       self.inner.lock().unwrap().values().map(Arc::clone).collect()
   }
   ```

2. Update all callers of `list()` to handle `Vec<Arc<Subscription>>`. Most callers iterate and read fields — `arc_sub.instrument_id` works identically to `sub.instrument_id` since `Arc<T>` auto-derefs to `T`.

### Fix #48 — Remove path: use map.remove() for owned value

**Problem:** `crates/ui-gateway/src/subscriptions.rs` (around lines 102–108): the single-item remove path clones the `Subscription` when `HashMap::remove()` already returns the owned value.

**Solution:** Call `map.remove(&id)` and use the returned `Option<Arc<Subscription>>` directly. No clone.

**Implementation steps:**

1. Find the single-item remove function (removing by `Uuid`):
   ```rust
   // Before:
   pub fn remove(&self, id: Uuid) -> Option<Subscription> {
       let mut map = self.inner.lock().unwrap();
       let sub = map.get(&id)?.clone();   // ← unnecessary clone
       map.remove(&id);
       Some(sub)
   }
   // After:
   pub fn remove(&self, id: Uuid) -> Option<Arc<Subscription>> {
       self.inner.lock().unwrap().remove(&id)  // ← returns owned Arc directly
   }
   ```
   `HashMap::remove()` returns `Option<V>` — use it directly.

2. Also clean up the secondary indexes: when removing from `inner`, also remove the entry from `by_instrument` and `by_panel` indexes. The ID-based removal in step #2 above should already collect the instrument/panel IDs from the map before removal to update the secondary indexes.

**Acceptance test:**
- Write a unit test that inserts 100 subscriptions and calls `list()`. Verify using `Arc::strong_count` that each `Arc<Subscription>` has strong_count = 3 (one in `inner`, one in `by_instrument`, one in `by_panel`) — no extra copies.
- Write a test that calls `remove(id)` and verifies `Arc::strong_count` drops from 3 to 0 (fully deallocated when all indexes are cleaned up).
- `grep -n "\.clone()" crates/ui-gateway/src/subscriptions.rs` should return only `Arc::clone` calls, no `Subscription::clone()` calls.

## Overall Acceptance Criteria
- [ ] `Subscription` is stored as `Arc<Subscription>` throughout the gateway (inner, by_instrument, by_panel)
- [ ] `insert()` accepts `Arc<Subscription>` — no struct clone inside insert
- [ ] `list()` returns `Vec<Arc<Subscription>>` — only refcount bumps
- [ ] Remove-by-filter functions collect `Vec<Uuid>` then call `map.remove()` — zero struct clones
- [ ] `remove(id)` uses `map.remove(&id)` directly — no intermediate clone
- [ ] No `Subscription.clone()` calls anywhere in `subscriptions.rs` (only `Arc::clone`)
- [ ] All subscription lifecycle tests pass
- [ ] `cargo test` passes

## Files to Touch
- `crates/ui-gateway/src/subscriptions.rs` — Arc<Subscription> in all containers; fix all 6 clone sites; update list() return type; fix remove() to use map.remove() directly
