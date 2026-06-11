# Issue #035 — PnlLot: lot cloned on archive insert

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 2 clones of same struct |
| Location | `crates/storage/src/pnl.rs:99,103` |

## Problem
PnlLot is cloned twice when archiving a closed lot — once for the archive insert (line 99) and once for the VecDeque push (line 103). The caller has ownership of the lot and could move it into one structure, sharing it with the other via `Arc`.

## Root Cause
The archive insert and the VecDeque both need ownership of the lot. Since neither can share a reference (both own the data), the lot is cloned twice. Using `Arc` would allow both structures to hold the same allocation.

## Implementation Plan
### Step 1 — Wrap PnlLot in Arc at the point of archiving
When a lot is closed and archived:
```rust
let lot = Arc::new(closed_lot);
self.archive.insert(*lot.id(), Arc::clone(&lot));
self.history.push_back(Arc::clone(&lot));
```
Both structures share the same allocation; no struct copies.

### Step 2 — Update PnlLot storage types
Change the archive and history types to use `Arc<PnlLot>`:
```rust
archive: HashMap<LotId, Arc<PnlLot>>,
history: VecDeque<Arc<PnlLot>>,
```

### Step 3 — Coordinate with #31
If #31 (FifoEngine String clones) changes the PnlLot struct, coordinate the Arc wrapping with that change.

### Step 4 — Consolidate with #36
Issue #36 is the same VecDeque push clone. Fix both in the same PR.

## Acceptance Criteria
- [ ] Zero PnlLot struct copies at `pnl.rs:99,103`
- [ ] Archive and history use `Arc<PnlLot>`
- [ ] P&L archive test: closed lots accessible from both archive and history
- [ ] No change to P&L correctness

## Files to Change
- `crates/storage/src/pnl.rs` — wrap PnlLot in Arc at archive point; update storage types at lines 99, 103
