# Agent Query — Replace Per-Tick HashMap Feature Rebuild with a Stable Slot Array
## Covers Issues: #4, #12, #17
## Phase: B
## Estimated Effort: 1 week
## Prerequisites: #2 (intern table must exist to assign stable slot IDs at init); implement alongside #3 since bytecode uses slot IDs as operands

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

`process_event` in the strategy runtime copies the entire feature set into a fresh `HashMap<String, f64>` on every market event. Features are stable — the set of feature names for a strategy instance doesn't change between ticks, only their values change. With 50 features and 100 strategy instances running at 10 ticks/sec, this creates 50,000 HashMap constructions per second and 50,000 string key clones. The correct data structure is a pre-allocated `Vec<f64>` indexed by a compile-time-resolved feature slot index. Issues #12 and #17 are the same root cause: deep-cloning `FeatureValue` and cloning string keys on every HashMap insert.

## Codebase Context

- `crates/strategy-runtime/src/runtime.rs` — around lines 65–70, rebuilds a `HashMap<String, f64>` from scratch on every event to pass to `evaluate_condition`.
- `crates/strategy-runtime/src/world.rs` — around line 100, `WorldState.features: HashMap<String, FeatureValue>` stores feature values; feature name strings are cloned as HashMap keys on every update.
- The bytecode `Op::LoadFeature(u16)` instruction (added in agent-03) expects to read from `slots: &[f64]`, not from a HashMap.

The problematic pattern in `runtime.rs`:
```rust
// lines 65-70 — runs on every market event
let features: HashMap<String, f64> = world_state.features
    .iter()
    .map(|(k, v)| (k.clone(), v.as_f64()))  // ← clones every key string
    .collect();                               // ← allocates new HashMap
evaluate_condition(&condition.expr, &features)?;
```

And in `world.rs`:
```rust
// line 100 — clones key string on every feature update
pub fn update_feature(&mut self, name: &str, value: FeatureValue) {
    self.features.insert(name.to_owned(), value);  // ← to_owned() every update
}
```

## Task

### Fix #4 — Slot-array features (root fix)
### Fix #12 — Eliminate deep FeatureValue clone (same root cause)
### Fix #17 — Eliminate string key clone (same root cause)

**Problem:** `WorldState.features` is a `HashMap<String, FeatureValue>` that is rebuilt or updated with string clones on every event. The bytecode compiler (agent-03) needs `&[f64]` not a HashMap. All three issues (#4, #12, #17) are eliminated by switching to a pre-allocated slot array.

**Solution:** Assign a stable `u16` slot index to each feature name at instance-initialization time. Replace `WorldState.features` with a `Vec<f64>` (the "slot array"). Feature updates write directly into the Vec at the pre-resolved index — no String clone, no HashMap operation.

**Implementation steps:**

1. Create a `FeatureRegistry` at the instance-manager level (or attach it to the strategy runtime coordinator):

```rust
/// Assigns stable u16 slot IDs to feature names.
/// All assignments happen at instance-init time; the registry is read-only during the hot loop.
pub struct FeatureRegistry {
    name_to_slot: HashMap<String, u16>,
    slot_to_name: Vec<String>,   // for debug/logging only
}

impl FeatureRegistry {
    pub fn new() -> Self { Self { name_to_slot: HashMap::new(), slot_to_name: Vec::new() } }

    /// Get or assign a slot ID for a feature name.
    /// Must only be called during instance initialization, not on the hot path.
    pub fn get_or_assign(&mut self, name: &str) -> u16 {
        if let Some(&id) = self.name_to_slot.get(name) {
            return id;
        }
        let id = self.slot_to_name.len() as u16;
        self.name_to_slot.insert(name.to_owned(), id);
        self.slot_to_name.push(name.to_owned());
        id
    }

    pub fn get(&self, name: &str) -> Option<u16> {
        self.name_to_slot.get(name).copied()
    }

    pub fn len(&self) -> usize { self.slot_to_name.len() }
}
```

2. During `StrategyInstance::new`, for every feature name referenced by the strategy's conditions and actions, call `registry.get_or_assign(feature_name)` to reserve a slot. Store the resulting `feature_slots: HashMap<String, u16>` on the instance for the `slot_resolver` closure used in bytecode compilation (agent-03).

3. Change `WorldState` in `crates/strategy-runtime/src/world.rs`:

```rust
pub struct WorldState {
    /// Feature slot array — indexed by u16 slot ID from FeatureRegistry.
    /// f64::NAN = "not yet received for this instrument".
    pub feature_slots: Vec<f64>,
    /// Parallel timestamp array — nanos since epoch, i64::MIN = absent.
    pub feature_time_ns: Vec<i64>,
    /// Current bar snapshot (open, high, low, close, volume).
    pub current_bar: BarSnapshot,
    // Remove: pub features: HashMap<String, FeatureValue>
}

impl WorldState {
    pub fn new(num_slots: u16) -> Self {
        Self {
            feature_slots: vec![f64::NAN; num_slots as usize],
            feature_time_ns: vec![i64::MIN; num_slots as usize],
            current_bar: BarSnapshot::default(),
        }
    }
}
```

4. Change `apply_feature_event` (or equivalent update function) to write directly into the slot array:

```rust
pub fn apply_feature_event(
    &mut self,
    name: &str,
    value: f64,
    timestamp_ns: i64,
    registry: &FeatureRegistry,
) {
    if let Some(slot) = registry.get(name) {
        self.feature_slots[slot as usize] = value;
        self.feature_time_ns[slot as usize] = timestamp_ns;
    }
    // Unknown feature names are silently ignored — not registered for this instance
}
```

No `to_owned()`, no HashMap insert, no allocation.

5. Delete the entire `HashMap<String, f64>` rebuild block at `runtime.rs:65-70`. The bytecode `run()` call (from agent-03) now passes `&world_state.feature_slots` directly:

```rust
// Before (deleted):
let features: HashMap<String, f64> = world_state.features.iter()
    .map(|(k, v)| (k.clone(), v.as_f64())).collect();
let result = evaluate_condition(&condition.expr, &features)?;

// After:
let result = bytecode::run(
    &instance.compiled_conditions[&node_id],
    &world_state.feature_slots,
    &world_state.current_bar,
) != 0.0;
```

6. Remove the `FeatureValue` type if it was only a wrapper around `f64`. If it held other variants (e.g., string features), audit whether those are needed on the hot path. If not, remove them. If yes, they become separate `Vec<T>` parallel arrays on `WorldState`.

7. Remove all `.clone()` calls on `FeatureValue` in `world.rs`. There should be none remaining.

**Acceptance test:**
- Add an integration test that creates a `StrategyInstance` with 50 named features, processes 10,000 events, and verifies zero HashMap allocations occur during processing using a `#[global_allocator]` counter or `dhat-rs`.
- Verify `world_state.features` field no longer exists (the HashMap type is gone from `WorldState`).
- Feature slot IDs must be identical across two calls to `get_or_assign` for the same name (idempotency).
- All feature evaluation tests must pass.

## Overall Acceptance Criteria
- [ ] Zero `HashMap` allocations per tick in `process_event` (verified with allocation counter in tests)
- [ ] `WorldState.features` field is a `Vec<f64>`, not a HashMap
- [ ] Feature slot IDs assigned exactly once at instance init (`get_or_assign` never called during hot loop)
- [ ] `apply_feature_event` contains no `.to_owned()` or `.clone()` calls
- [ ] `FeatureValue` type removed or reduced to `f64` alias with no per-event clones
- [ ] All feature evaluation tests pass
- [ ] `cargo build --release` succeeds

## Files to Touch
- `crates/strategy-runtime/src/runtime.rs` — delete HashMap rebuild block; use slot array in bytecode::run call
- `crates/strategy-runtime/src/world.rs` — replace HashMap<String, FeatureValue> with Vec<f64> slot array and parallel timestamp Vec
- `crates/strategy-runtime/src/registry.rs` (new, or add to existing module) — FeatureRegistry struct
- `crates/strategy-runtime/src/bytecode.rs` — confirm run() signature accepts &[f64] slot array (from agent-03)
- `crates/strategy-runtime/src/lib.rs` — export FeatureRegistry; update module list
