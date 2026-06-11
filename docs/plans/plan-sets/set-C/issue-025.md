# Issue #025 — Subscription cloned on insert

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 1 struct copy per subscribe |
| Location | `crates/ui-gateway/src/subscriptions.rs:96` |

## Problem
A Subscription struct is cloned instead of moved on insert into the registry. The clone is unnecessary — the caller has no further use for the original value after inserting it.

## Root Cause
At `subscriptions.rs:96`, a `.clone()` call copies the Subscription struct into the registry map. This is likely an artifact of the struct also being stored in a secondary data structure (e.g., a Vec and a HashMap), requiring two copies. See also #37 (Subscription fully cloned at insertion into multiple structures).

## Implementation Plan
### Step 1 — Check if the value is inserted into multiple structures
Read `subscriptions.rs` around line 96 to see if the Subscription is inserted into both a HashMap and a Vec (or similar). If so, this is the same issue as #37.

### Step 2 — Wrap Subscription in Arc (preferred)
Change the primary Subscription type to `Arc<Subscription>`. Both data structures hold the same Arc, not clones of the struct. The Arc clone is a cheap atomic increment.

### Step 3 — Alternative: move into one structure, reconstruct the key for the other
If the second structure only needs a key (e.g., a connection_id or subscription_id), don't store the full struct — store the ID and look up in the primary map when needed.

### Step 4 — Consolidate with #37
This fix and #37 are likely the same issue. Resolve together: wrap Subscription in Arc and store Arc clones everywhere.

## Acceptance Criteria
- [ ] No `.clone()` on Subscription at `subscriptions.rs:96`
- [ ] Subscription stored via Arc<Subscription> in all data structures
- [ ] Subscribe operation is zero-allocation (only atomic Arc increment)
- [ ] WS subscription tests pass

## Files to Change
- `crates/ui-gateway/src/subscriptions.rs` — change Subscription storage to Arc; remove clone at line 96
