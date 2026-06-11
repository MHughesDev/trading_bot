# Agent Query — O(1) Instrument-Indexed Dispatch + Arc<Universe> Through Pipeline Stages
## Covers Issues: #5, #13, #18, #21
## Phase: B
## Estimated Effort: 3–5 days
## Prerequisites: #2 (InstrumentId(u32) must exist for dispatch keying)

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

Every market event dispatches to all strategy instances by iterating the entire instance map and comparing String instrument IDs, then deep-cloning the event payload for each match. With 100 strategy instances across 20 instruments, every event does 100 string comparisons and up to 5 deep clones. Additionally, the Universe struct is cloned at each pipeline stage (filter, rank, limit), and node IDs are stored and compared as Strings. All of these are unnecessary: dispatch can be O(1) with a properly keyed HashMap; event clones can be eliminated by passing references; Universe can be reference-counted; node IDs can be `u32` newtypes.

## Codebase Context

- `crates/strategy-runtime/src/runtime.rs` — around lines 166–181, iterates all instances comparing `instance.instrument_id` (String) to the event's `instrument_id` (String), then calls `event.clone()` for each matching instance.
- `crates/strategy-runtime/src/nodes/mod.rs` — around lines 22–78: Universe struct with String fields; pipeline stages at lines 46, 53, 59, 65, 71 each clone the Universe; node IDs stored and compared as Strings.
- `crates/strategy-runtime/src/nodes/filter.rs` — around lines 12–18; receives owned Universe at each filter call.
- `crates/domain/src/instrument.rs` — `InstrumentId(u32)` added by agent-02.

The problematic dispatch pattern in `runtime.rs`:
```rust
// lines 166-181 — O(n) scan with String comparison and event clone per match
for (_id, instance) in &mut self.instances {
    if instance.instrument_id == event.instrument_id {  // ← String comparison x N
        instance.process_event(event.clone())?;         // ← deep clone per match
    }
}
```

The problematic Universe clone pattern in `nodes/mod.rs`:
```rust
// Multiple pipeline stages — each clones the full Universe
let filtered  = filter_node.filter(universe.clone())?;
let ranked    = rank_node.rank(filtered.clone())?;
let limited   = limit_node.limit(ranked.clone())?;
```

## Task

### Fix #5 — Instrument-indexed dispatch, zero event clones

**Problem:** Dispatch iterates all `N` strategy instances with String ID comparison and clones the event for each match. With 100 instances, every event triggers 100 string comparisons even if only 5 instances match.

**Solution:** Re-key the instance map by `InstrumentId` so dispatch is a single HashMap lookup. Pass events by reference so no clone is needed.

**Implementation steps:**

1. Change `InstanceManager.instances` type from `HashMap<Uuid, StrategyInstance>` to `HashMap<InstrumentId, Vec<StrategyInstance>>`. When a new instance is registered, `entry(instrument_id).or_default().push(instance)`. The per-bucket `Vec` holds all instances for that instrument. The original `Uuid` uniqueness constraint is enforced within each bucket (check for duplicate UUIDs on insert).

2. Change the dispatch function signature:
   ```rust
   // Before:
   pub fn dispatch_event(&mut self, event: MarketEvent) -> Result<()>
   // After:
   pub fn dispatch_event(&mut self, instrument: InstrumentId, event: &MarketEvent) -> Result<()>
   ```

3. In `dispatch_event`, replace the O(n) iteration with a single lookup:
   ```rust
   if let Some(bucket) = self.instances.get_mut(&instrument) {
       for instance in bucket.iter_mut() {
           instance.process_event(event)?;  // ← reference, no clone
       }
   }
   ```

4. Update `StrategyInstance::process_event` signature to accept `event: &MarketEvent` (reference, not owned).

5. Delete `event.clone()` at the old dispatch site. The event is now passed by reference. If any stage downstream needs to store the event, it clones only the fields it needs, not the full event.

6. Update `WorldEvent.instrument_id` field type from `String` to `InstrumentId` (coordinating with agent-02). The event dispatch key and the event field must be the same type.

### Fix #13 — Thread Arc<Universe> through pipeline stages

**Problem:** `crates/strategy-runtime/src/nodes/mod.rs` (around lines 46, 53, 59, 65, 71) clones the Universe at each pipeline stage: `filter(universe.clone())`, `rank(filtered.clone())`, etc.

**Solution:** Wrap the Universe in `Arc`. Clone the `Arc` (a refcount bump, ~1 ns) rather than the struct. Pipeline stages accept `Arc<Universe>` and return `Arc<Universe>` or indices into the shared Universe.

**Implementation steps:**

1. At strategy-load time, wrap the initial Universe in `Arc<Universe>`:
   ```rust
   let universe: Arc<Universe> = Arc::new(Universe::from_definition(&definition)?);
   ```

2. Change all pipeline stage function signatures:
   ```rust
   // Before:
   fn filter(&self, universe: Universe) -> Result<Universe>
   fn rank(&self, universe: Universe) -> Result<Universe>
   fn limit(&self, universe: Universe) -> Result<Universe>
   // After:
   fn filter(&self, universe: Arc<Universe>) -> Result<Arc<Universe>>
   fn rank(&self, universe: Arc<Universe>) -> Result<Arc<Universe>>
   fn limit(&self, universe: Arc<Universe>) -> Result<Arc<Universe>>
   ```

3. For stages that produce a subset of the universe (filter), return a new `Arc<Universe>` containing only the matching entries. If the Universe implements filtering by index, stages can return `Vec<usize>` (indices into the shared universe) to avoid copying entries at all.

4. Remove all `universe.clone()` at pipeline stage call sites. Replace with `Arc::clone(&universe)` where an owned Arc is needed.

### Fix #18 — Intern Node IDs

**Problem:** `crates/strategy-runtime/src/nodes/mod.rs` (around lines 43–78) stores and compares node IDs as `String` values, causing string allocations and comparisons at routing time.

**Solution:** Add `NodeId(u32)` as a newtype. Assign IDs at strategy-graph parse time. Use `NodeId` for all runtime lookups.

**Implementation steps:**

1. Add to `crates/domain/src/lib.rs` or a new `crates/strategy-runtime/src/ids.rs`:
   ```rust
   #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
   pub struct NodeId(pub u32);
   ```

2. In the strategy-graph parser, maintain a counter and assign `NodeId` values as each node is parsed. Store a `HashMap<String, NodeId>` for name-to-ID lookup during parse. After parse, discard the map — only `NodeId` values are stored in the runtime graph.

3. Update all node structs to use `NodeId` instead of `String` for their own ID and for references to other nodes (e.g., `next_node: NodeId`).

4. Update `compiled_conditions: HashMap<NodeId, Program>` on `StrategyInstance` (from agent-03) to key by `NodeId` — now a cheap u32 HashMap lookup.

### Fix #21 — SmallVec + interned feature IDs in UniverseEntry

**Problem:** `crates/strategy-runtime/src/nodes/mod.rs` (around lines 22–26), `UniverseEntry` has String fields and may hold feature values in a heap-allocated Vec, rebuilt per filter/rank call.

**Solution:** Replace String instrument ID with `InstrumentId(u32)`. Use `SmallVec<[f64; 8]>` for per-entry feature values (avoids heap allocation for entries with ≤8 features, which covers most real-world cases).

**Implementation steps:**

1. Add `smallvec = "1"` to workspace `Cargo.toml` if not already present.

2. Change `UniverseEntry`:
   ```rust
   // Before (approximate):
   pub struct UniverseEntry {
       pub instrument_id: String,
       pub features: Vec<f64>,
   }
   // After:
   pub struct UniverseEntry {
       pub instrument_id: InstrumentId,             // u32, no allocation
       pub features: SmallVec<[f64; 8]>,            // inline for ≤8 features
   }
   ```

3. Update all construction sites of `UniverseEntry` to use `InstrumentId` and `SmallVec`.

**Acceptance test:**
- Write an integration test: create 100 StrategyInstance instances across 20 instruments. Send 1,000 market events. Verify that `dispatch_event` performs exactly 1 HashMap lookup per event (add a counter to verify). Verify zero `event.clone()` calls in the dispatch path.
- Verify `universe.clone()` is absent from the pipeline stage calls (grep `nodes/mod.rs` for `.clone()` on Universe type).

## Overall Acceptance Criteria
- [ ] `dispatch_event` makes exactly one HashMap lookup per event (O(1) by instrument)
- [ ] Zero `event.clone()` calls in `runtime.rs` dispatch path
- [ ] Dispatch cost is independent of total instance count (only bucket size matters)
- [ ] Universe pipeline stages accept and return `Arc<Universe>`, not owned `Universe`
- [ ] `NodeId` is `u32` newtype; no String node IDs in runtime data structures
- [ ] `UniverseEntry.instrument_id` is `InstrumentId(u32)`, not String
- [ ] `UniverseEntry.features` uses `SmallVec<[f64; 8]>`
- [ ] All strategy dispatch and universe pipeline tests pass
- [ ] `cargo build --release` succeeds

## Files to Touch
- `crates/strategy-runtime/src/runtime.rs` — re-key instances HashMap by InstrumentId; dispatch by reference; delete event.clone()
- `crates/strategy-runtime/src/nodes/mod.rs` — Arc<Universe> in all stage signatures; NodeId newtype; UniverseEntry with SmallVec
- `crates/strategy-runtime/src/nodes/filter.rs` — update to Arc<Universe> signature
- `crates/strategy-runtime/src/ids.rs` (new or add to existing) — NodeId newtype
- `crates/domain/src/lib.rs` — export NodeId if placed here
- `Cargo.toml` — add `smallvec = "1"` to workspace dependencies
