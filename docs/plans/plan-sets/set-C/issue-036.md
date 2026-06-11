# Issue #036 — PnlLot: lot cloned again on VecDeque push

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | Redundant struct copy |
| Location | `crates/storage/src/pnl.rs:103` |

## Problem
PnlLot is cloned a second time redundantly when pushed to the VecDeque history. This is the second of two identical clones at lines 99 and 103, both operating on the same lot.

## Root Cause
Same root cause as #35: two data structures both need ownership, and without Arc the lot must be cloned for each. The VecDeque push at line 103 is the second clone.

## Implementation Plan
### Step 1 — Consolidated fix with #35
This issue is fixed by the same Arc wrapping described in #35. Fix both #35 and #36 together in a single PR.

### Step 2 — Verify the fix eliminates both clones
After the Arc fix, confirm that `pnl.rs:99` and `pnl.rs:103` both perform `Arc::clone(&lot)` (atomic increment) rather than `lot.clone()` (struct copy).

## Acceptance Criteria
- [ ] No PnlLot struct copy at `pnl.rs:103`
- [ ] VecDeque holds `Arc<PnlLot>` (atomic increment clone from #35)
- [ ] P&L history test: all closed lots appear in history in FIFO order

## Files to Change
- `crates/storage/src/pnl.rs` — consolidated with #35: Arc wrapping at line 103
