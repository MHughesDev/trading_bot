# Issue #029 — Demand registry: string clones on every add/remove

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | D |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 2 string clones per demand change |
| Location | `crates/demand-manager/src/registry.rs:53,75` |

## Problem
Lane ID and instrument ID Strings are cloned on every demand add/remove, even though the caller already owns them. Every collector start or stop operation triggers this path, and the system may handle dozens of demand changes per second during configuration updates.

## Root Cause
The demand registry stores lane IDs and instrument IDs as owned Strings in its internal HashMap. When `add_demand` or `remove_demand` is called, the string parameters are cloned to insert as owned keys.

## Implementation Plan
### Step 1 — Intern lane IDs and instrument IDs at startup
Coordinate with #2 (interned IDs). Lane IDs and instrument IDs should be interned at startup into `u32` numeric IDs. The demand registry uses `u32` keys, not Strings. Clone cost becomes zero (u32 is Copy).

### Step 2 — Standalone fix: use Arc<str> for IDs
If #2 is not yet landed, change the registry to use `Arc<str>` for lane/instrument IDs. At the call sites (lines 53 and 75), the caller passes an `Arc<str>` clone (atomic increment) rather than allocating a new String.

### Step 3 — Update call sites
Pass pre-constructed `Arc<str>` or `InstrumentId(u32)` at all demand add/remove call sites. Do not allocate a new String in the registry.

## Acceptance Criteria
- [ ] Zero heap String allocations for lane/instrument IDs on demand add/remove
- [ ] Registry uses Arc<str> (interim) or u32 (after #2)
- [ ] Demand registry unit tests pass
- [ ] Demand add/remove under collector start/stop: no String allocation

## Files to Change
- `crates/demand-manager/src/registry.rs` — replace String clones at lines 53, 75 with Arc<str> or u32 IDs
