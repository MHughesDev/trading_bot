# Issue #041 — Reconciliation: string comparison with alloc

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | E |
| Pattern | Allocation |
| Quick Win | Yes |
| Latency Impact | 1 string replace per position in reconciliation loop |
| Location | `crates/reconciliation/src/positions.rs:35-36` |

## Problem
`.replace()` allocates a new string on every position during reconciliation. If the platform reconciles 1,000 positions at startup, this creates 1,000 unnecessary String allocations just for normalization comparisons.

## Root Cause
The reconciliation logic normalizes position identifiers for comparison using `.replace("-", "_")` or similar string manipulation at the comparison point. This creates a new String per comparison rather than normalizing once at load time.

## Implementation Plan
### Step 1 — Pre-normalize at load time
When position data is loaded from the broker API or database, normalize the identifier format once:
```rust
// At data load:
let normalized_id = raw_id.replace("-", "_");
```
Store the normalized form. All subsequent comparisons use the already-normalized stored value — no per-comparison allocation.

### Step 2 — Use interned IDs (coordinate with #2)
Longer-term: if position instrument IDs are interned as `InstrumentId(u32)`, no string comparison is needed at all — just compare u32 values. This eliminates the normalization entirely.

### Step 3 — Verify no other per-comparison allocations
Check lines 35-36 and surrounding context for any other `format!`, `.to_string()`, or `.replace()` calls in the reconciliation comparison loop.

## Acceptance Criteria
- [ ] No `.replace()` or other String allocation in the reconciliation comparison loop
- [ ] Position IDs normalized once at load time, not per-comparison
- [ ] Reconciliation test with 1,000 positions: zero allocation in comparison loop
- [ ] Reconciliation produces correct results (regression test)

## Files to Change
- `crates/reconciliation/src/positions.rs` — pre-normalize IDs at load time; remove per-comparison .replace() at lines 35-36
