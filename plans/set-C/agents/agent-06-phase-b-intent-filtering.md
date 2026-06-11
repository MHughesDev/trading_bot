# Agent Query — O(1) Signal Filtering with HashSet + Eliminate strategy_id Clone
## Covers Issues: #54, #55, #63
## Phase: B
## Estimated Effort: 2–3 hours (quick wins)
## Prerequisites: None

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

Intent filtering checks whether an action's trigger signal is in the active signal set using `Vec::contains`, which is an O(n) linear scan through strings. With 10 actions and 5 signals per strategy instance, each signal evaluation triggers 50 string comparisons. At 100 strategy instances evaluating at 10 ticks/sec, this creates 50,000 string searches per second from this one spot. Additionally, the `strategy_id` string is unnecessarily cloned into every `OrderIntent` at construction time, even when the caller already has ownership. All three issues (#54, #55, #63) are in `crates/strategy-runtime/src/intents.rs` and each is a small change.

## Codebase Context

- `crates/strategy-runtime/src/intents.rs` — contains the intent building and filtering logic. Key problem locations:
  - Around line 30: `Some(strategy_id.to_owned())` clones the strategy ID string for every order intent constructed.
  - Around line 47: `signals.contains(&a.on_signal)` performs O(n) Vec search on a `Vec<String>`.

The problematic pattern (issues #55 and #63 — same fix):
```rust
// intents.rs ~line 47 — O(n) linear scan per signal check
fn is_signal_active(signals: &Vec<String>, action: &Action) -> bool {
    signals.contains(&action.on_signal)  // ← Vec::contains = O(n) string scan
}
```

The problematic strategy_id clone (issue #54):
```rust
// intents.rs ~line 30 — always clones even when not needed
OrderIntent {
    strategy_id: Some(strategy_id.to_owned()),  // ← .to_owned() = heap alloc
    // ...
}
```

## Task

### Fix #55 + #63 — HashSet for active signal lookup (both issues are the same fix)

**Problem:** `signals.contains(&a.on_signal)` on a `Vec<String>` performs an O(n) string comparison scan every time a signal is checked. Issues #55 and #63 both refer to this same pattern in different calling contexts — fixing it once at the data-structure level eliminates both.

**Solution:** Change the `signals` collection from `Vec<String>` to `HashSet<String>` (or `HashSet<Arc<str>>`). The HashSet is constructed once at the time the strategy instance or action group is initialized, not per tick. The per-tick lookup becomes O(1) average.

**Implementation steps:**

1. Locate the struct or function parameter that holds `signals: Vec<String>` in `crates/strategy-runtime/src/intents.rs`. This may be on an `IntentFilter`, `ActionGroup`, or as a function parameter.

2. Change the field type:
   ```rust
   // Before:
   signals: Vec<String>,
   // After:
   signals: HashSet<String>,
   ```
   Add `use std::collections::HashSet;` at the top of the file.

3. At the construction site (where the strategy definition is parsed and the signal list is known), build the `HashSet` once:
   ```rust
   let signals: HashSet<String> = definition.signals.iter().cloned().collect();
   ```
   This is initialization-time work, not per-tick work.

4. The per-tick check `signals.contains(&a.on_signal)` remains syntactically identical but is now O(1) average instead of O(n).

5. If `signals` is passed as a `&Vec<String>` function parameter across call sites, update those call sites to pass `&HashSet<String>`. Search for all callers of the function containing the `signals.contains` call and update their types accordingly.

6. Optional upgrade: if agent-02 is complete and `Arc<str>` interning is in place, use `HashSet<Arc<str>>` to eliminate even the O(1) hash input allocation. For the `contains` check, use `.get(signal_str.as_ref())` — no allocation needed for the lookup.

### Fix #54 — Remove strategy_id clone in intent construction

**Problem:** `Some(strategy_id.to_owned())` at `crates/strategy-runtime/src/intents.rs:30` clones the strategy ID string for every `OrderIntent` constructed. If the caller already has an owned `String`, this is a wasteful allocation.

**Solution:** Change the function to consume the strategy ID rather than borrowing it, or use `Arc<str>` so the clone is a refcount bump (approximately 1 ns vs. 100+ ns for a heap allocation).

**Implementation steps:**

1. Locate the function in `crates/strategy-runtime/src/intents.rs` that builds `OrderIntent` values (around line 30) and inspect the `strategy_id` parameter type.

2. **Option A (preferred if agent-02 is done):** Change `strategy_id: &str` to `strategy_id: Arc<str>`. The `Arc<str>` is created once per instance at init time. At intent construction:
   ```rust
   strategy_id: Some(Arc::clone(&self.strategy_id)),  // ← refcount bump, ~1 ns
   ```

3. **Option B (if Arc<str> is not yet in place):** Change the parameter from `strategy_id: &str` to `strategy_id: String` and pass the owned value:
   ```rust
   // Caller passes owned String:
   build_intent(strategy_id, ...)
   // Function stores directly:
   OrderIntent { strategy_id: Some(strategy_id), ... }
   ```
   This eliminates the clone when the caller already has ownership.

4. Update all callers of the intent-building function to match the new signature. Verify that no new `.to_owned()` or `.clone()` has been introduced at any call site to compensate.

5. If `strategy_id` appears in multiple `OrderIntent`-building functions, apply the fix consistently to all of them.

**Acceptance test:**
- Write a unit test that constructs 1,000 `OrderIntent` values with a `Vec` of 10 active signals and a `Vec` of 50 candidate actions. Measure the time taken for `is_signal_active` lookups on both the old Vec and new HashSet implementations. The HashSet version should be measurably faster and O(1) per lookup.
- Verify with a `grep -n "signals.contains" crates/strategy-runtime/src/intents.rs` that the call site exists (the function should still work, just using HashSet).
- Verify with `grep -n "to_owned\|\.clone()" crates/strategy-runtime/src/intents.rs` that no `.to_owned()` calls remain for the `strategy_id` field.

## Overall Acceptance Criteria
- [ ] `signals` field/parameter is `HashSet<String>` (or `HashSet<Arc<str>>`), not `Vec<String>`
- [ ] No `Vec::contains` calls on signal names anywhere in `intents.rs`
- [ ] HashSet constructed once at strategy/action-group initialization, not per tick
- [ ] No `.to_owned()` or `.clone()` on `strategy_id` in intent construction (`Some(strategy_id.to_owned())` is gone)
- [ ] All intent filtering tests pass with correct signal match/no-match results
- [ ] `cargo test` passes

## Files to Touch
- `crates/strategy-runtime/src/intents.rs` — change signals Vec to HashSet; fix strategy_id clone; update all related types and signatures
