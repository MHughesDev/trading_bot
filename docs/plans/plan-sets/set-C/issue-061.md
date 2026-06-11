# Issue #061 — RobotsTxt: linear search through path rules

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | C |
| Pattern | Search |
| Quick Win | No |
| Latency Impact | O(n) string starts_with ops per path check |
| Location | `crates/collectors/src/web/scraper.rs:146-150` |

## Problem
`disallowed.iter().filter(|d| path.starts_with(d.as_str()))` performs O(n) string comparisons for every path check. No early exit. With 50 disallow rules and 10 URLs/sec, this is 500 prefix comparisons per second.

## Root Cause
Same root cause as #53 — linear Vec iteration for prefix matching. The two issues describe slightly different code paths in the same scraper file; #61 is at lines 146-150, #53 is at lines 149, 157. Both are addressed by the same trie solution.

## Implementation Plan
### Step 1 — Consolidate with #53
This issue and #53 describe the same problem in adjacent code. Implement the trie solution from #53; verify it covers this code path as well.

### Step 2 — Specifically address the filter+no-early-exit pattern
The `.filter()` pattern at lines 146-150 continues iterating even after a match is found (unlike `.any()` which short-circuits). Confirm whether the code uses `.filter().count() > 0` or `.any()`:
- If `.filter().count()`: replace with `.any()` as an interim fix
- After trie: lookup replaces both

### Step 3 — Add early exit to any remaining linear scans
If any linear scan remains after the trie, replace `.filter(pred).next().is_some()` with `.any(pred)` for proper short-circuit behavior.

## Acceptance Criteria
- [ ] No linear Vec iteration for path disallow/allow checking at `scraper.rs:146-150`
- [ ] Trie lookup (from #53) covers this code path
- [ ] If trie not yet landed: `.any()` used instead of `.filter()` for short-circuit
- [ ] Path checking test: /admin/ correctly blocked by Disallow: /admin/

## Files to Change
- `crates/collectors/src/web/scraper.rs` — consolidate with #53 trie; replace linear scan at lines 146-150
