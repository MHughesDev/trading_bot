# Issue #052 — robots.txt parsing: Vec<String> per line without capacity hint

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | C |
| Pattern | Allocation |
| Quick Win | Yes |
| Latency Impact | ~20–50 string allocations per fetch + Vec reallocs |
| Location | `crates/collectors/src/web/scraper.rs:105-132` |

## Problem
The robots.txt parser pushes one String per line to `disallowed` and `allowed` Vecs without `with_capacity()`. Vec reallocs double the capacity on each overflow — for a robots.txt with 50 lines, this causes multiple reallocs during parsing. Additionally, each line is allocated as a separate String.

## Root Cause
`Vec::new()` (capacity 0) is used before pushing parsed rules. When the Vec grows past its capacity, Rust reallocates and copies the existing elements. Without a capacity hint, this happens O(log n) times for n elements.

## Implementation Plan
### Step 1 — Estimate line count before parsing
When parsing a robots.txt file, count the newlines in the raw bytes to get an approximate line count before starting the parse:
```rust
let approx_lines = content.bytes().filter(|&b| b == b'\n').count();
let mut disallowed = Vec::with_capacity(approx_lines / 2);
let mut allowed = Vec::with_capacity(approx_lines / 2);
```

### Step 2 — Use with_capacity on both Vecs
Initialize both `disallowed` and `allowed` with the capacity hint before the parse loop.

### Step 3 — Consider using &str slices into the content buffer
Instead of allocating a String per rule, store `&str` slices into the already-allocated robots.txt content string. This requires the content to outlive the parsed rules — use `Arc<str>` for the content and `&str` slices for rules, or store the content on the struct.

### Step 4 — Consolidate with #53 (trie optimization)
If #53 builds a trie from the parsed rules, the per-rule Strings are consumed at trie-build time and not stored long-term. The allocation optimization in #52 reduces the cost of the short-lived parse; the trie removes the per-lookup cost. Both should be implemented together.

## Acceptance Criteria
- [ ] `Vec::with_capacity()` used for disallowed and allowed rule Vecs before parse loop
- [ ] No Vec reallocations for a robots.txt with up to 100 rules
- [ ] robots.txt parse test passes with correct allow/disallow rules extracted
- [ ] Consolidation: if #53 trie is implemented, the per-rule Strings feed the trie at parse time

## Files to Change
- `crates/collectors/src/web/scraper.rs` — add Vec::with_capacity at lines 105-132 before pushing parsed rules
