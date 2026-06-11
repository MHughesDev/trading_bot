# Issue #067 — Reddit: symbol lookup in HashMap per post

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Search |
| Quick Win | Yes |
| Latency Impact | Per symbol per post (minor) |
| Location | `crates/collectors/src/social/reddit.rs:87-90` |

## Problem
`.contains_key(&upper)` on `self.known_instruments` (a HashMap) is called per symbol per post. While HashMap lookup is O(1), this occurs inside the mention extraction loop and hashes a String per check. With 100 symbols per post, this is 100 hash operations.

## Root Cause
`known_instruments` is a `HashMap<String, InstrumentData>`. Each lookup requires hashing the `upper` String (the uppercased symbol). The String must be uppercase-transformed and then hashed on every lookup.

## Implementation Plan
### Step 1 — Convert known_instruments to HashSet for membership check
If the check is only for membership (is this string a known symbol?), use `AHashSet<String>` instead of a full `HashMap`. `AHashSet` uses a faster non-cryptographic hash.

### Step 2 — Pre-uppercase the symbols in the set
Store symbols pre-uppercased in the set. Then at lookup time, the `.to_uppercase()` call on the post token is still needed, but the lookup is as fast as possible.

### Step 3 — Coordinate with #51 (binary search approach)
Issue #51 replaces the per-post HashMap with a sorted Vec + binary search for the scores tracking. If the same sorted Vec is used for membership checking, no separate HashSet is needed — use `binary_search` for both membership test and index lookup.

### Step 4 — Document as minor optimization
This is a very low priority issue. If #51 is implemented (binary search on sorted Vec), this issue is automatically resolved as a side effect.

## Acceptance Criteria
- [ ] Known symbol lookup does not use String hashing in the inner mention-extraction loop
- [ ] Either AHashSet or binary search on sorted Vec used for membership test
- [ ] Reddit mention extraction test passes with correct symbol detection
- [ ] No change to extraction accuracy

## Files to Change
- `crates/collectors/src/social/reddit.rs` — replace HashMap contains_key at lines 87-90 with AHashSet or binary search (coordinate with #51)
