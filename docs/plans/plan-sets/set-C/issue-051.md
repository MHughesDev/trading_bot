# Issue #051 — HashMap rebuilt per post in reddit collector

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | C |
| Pattern | Allocation |
| Quick Win | No |
| Latency Impact | Per post: 1 HashMap init + (symbol_count × hashing) |
| Location | `crates/collectors/src/social/reddit.rs:71-105` |

## Problem
`extract_mentions()` builds a fresh `HashMap<String, f32>` for every Reddit post to track mention scores. With hundreds of posts per second during market hours and a symbol list of 100+ tickers, this is hundreds of HashMap allocations per second plus per-symbol string hashing.

## Root Cause
The mention extraction function takes no persistent state — it initializes a new HashMap for every call. The HashMap must be initialized, populated, and then iterated to construct the output, with all HashMap allocation overhead per post.

## Implementation Plan
### Step 1 — Move scores to a pre-allocated stable array indexed by symbol index
Create a `Vec<f32>` of length `symbol_count` initialized to 0.0, stored persistently on the `RedditCollector` struct (or passed in as a scratchpad buffer):
```rust
struct RedditCollector {
    known_symbols: Vec<String>,   // sorted list
    scores_buf: Vec<f32>,         // reusable scratchpad, len = known_symbols.len()
}
```

### Step 2 — Reset the scratchpad before each post
Before processing each post, reset the scratchpad:
```rust
self.scores_buf.iter_mut().for_each(|s| *s = 0.0);
```
No allocation — just writing zeros to an existing Vec.

### Step 3 — Binary search for symbol lookup
Instead of HashMap lookup, use binary search on the sorted symbol list:
```rust
if let Ok(idx) = self.known_symbols.binary_search(&symbol) {
    self.scores_buf[idx] += score;
}
```
O(log n) per lookup, no hashing, no allocation.

### Step 4 — Collect only non-zero scores for output
At the end of processing each post, iterate the scratchpad and collect only non-zero entries into the output:
```rust
let mentions: Vec<Mention> = self.scores_buf.iter().enumerate()
    .filter(|(_, &s)| s > 0.0)
    .map(|(i, &s)| Mention { symbol: &self.known_symbols[i], score: s })
    .collect();
```

## Acceptance Criteria
- [ ] Zero HashMap allocations in `extract_mentions()` per post
- [ ] Persistent scratchpad Vec reused across posts
- [ ] Binary search used for symbol lookup
- [ ] Reddit mention extraction tests pass with correct scores
- [ ] Performance: 100 posts/sec with 100 symbols: < 1 ms CPU per second

## Files to Change
- `crates/collectors/src/social/reddit.rs` — replace per-post HashMap at lines 71-105 with persistent Vec scratchpad and binary search
