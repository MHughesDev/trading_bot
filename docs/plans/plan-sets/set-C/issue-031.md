# Issue #031 — FifoEngine: string clones in P&L path

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | D |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 1 Enum→String clone per lot operation |
| Location | `crates/storage/src/pnl.rs:72` |

## Problem
Trade side enum is converted to String for storage in PnlLot, then that String is used for P&L computation. Enum-to-String conversion allocates on every lot operation (every fill that creates or closes a lot).

## Root Cause
`PnlLot` stores the trade side as a `String` (e.g., "buy" / "sell") rather than the original enum variant. The conversion at `pnl.rs:72` allocates a String on every lot creation. P&L matching (FIFO) then compares these Strings rather than comparing enum variants, which is more expensive.

## Implementation Plan
### Step 1 — Store the Side enum directly in PnlLot
Change `PnlLot.side` from `String` to `Side` (the existing trade side enum). Derive `Copy` on `Side` if it is a simple C-style enum (which it should be).

### Step 2 — Use InstrumentId(u32) for instrument in PnlLot
Change `PnlLot.instrument` from `String` to `InstrumentId(u32)` (from the intern table, coordinating with #2). This eliminates the instrument String clone per lot.

### Step 3 — Remove String conversion at pnl.rs:72
Delete the `to_string()` or `format!()` call that produces the side String. The lot now stores the enum directly.

### Step 4 — Update P&L matching logic
P&L FIFO matching compares sides and instruments. Update comparisons to use enum comparison and u32 comparison instead of String comparison. This is both faster and cleaner.

## Acceptance Criteria
- [ ] PnlLot.side is `Side` enum, not String
- [ ] PnlLot.instrument is `InstrumentId(u32)`, not String
- [ ] Zero String allocation at `pnl.rs:72` per lot operation
- [ ] P&L FIFO calculation produces correct results (regression test with known trades)

## Files to Change
- `crates/storage/src/pnl.rs` — change PnlLot struct; remove String conversion at line 72; update matching logic
