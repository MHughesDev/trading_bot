# Issue #053 — Web scraper: .starts_with() on every filter pass

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | C |
| Pattern | Search |
| Quick Win | No |
| Latency Impact | O(n path comparisons) per fetch |
| Location | `crates/collectors/src/web/scraper.rs:149,157` |

## Problem
`is_allowed()` iterates disallowed/allowed Vecs and calls `.starts_with()` on every entry for every path check. With 50 disallow rules, every fetch pays 50 string prefix comparisons. For a scraper hitting 10 URLs/sec with 50 rules each, this is 500 prefix comparisons per second — negligible in isolation but wasteful when a trie would give O(path_length) lookup.

## Root Cause
The path-matching algorithm is linear scan through a Vec of rule strings. There is no prefix tree (trie) or other O(k) structure to accelerate prefix matching.

## Implementation Plan
### Step 1 — Add a trie or radix tree for robots.txt rules
After parsing robots.txt (in #52), build a prefix trie from the disallow/allow rule paths:
```rust
struct RulesTrie {
    disallowed: radix_trie::Trie<String, ()>,
    allowed: radix_trie::Trie<String, ()>,
}
```
Use the `radix_trie` crate or implement a simple path-segment trie.

### Step 2 — Replace linear scan with trie lookup
Replace:
```rust
disallowed.iter().any(|d| path.starts_with(d.as_str()))
```
with:
```rust
rules_trie.disallowed.get_ancestor(path).is_some()
```
O(path_length) lookup regardless of rule count.

### Step 3 — Build trie at robots.txt load time
The trie is built once per robots.txt fetch (which is infrequent — cached per domain). The per-path lookup cost drops from O(rules) to O(path_length).

### Step 4 — Consolidate with #61 and #65
Issues #53, #61, and #65 all describe the same robots.txt path-matching problem. Resolve all three with a single trie implementation.

## Acceptance Criteria
- [ ] robots.txt path matching uses a trie with O(path_length) lookup
- [ ] No linear Vec iteration in `is_allowed()`
- [ ] robots.txt parse: trie built correctly from disallow/allow rules
- [ ] Path-matching tests pass for common robots.txt patterns (wildcard, path prefix, /admin/)

## Files to Change
- `crates/collectors/src/web/scraper.rs` — replace linear Vec scan at lines 149, 157 with trie lookup
