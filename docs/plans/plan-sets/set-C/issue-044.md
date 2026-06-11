# Issue #044 — Vec<Vec<>> nested allocations

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | E |
| Pattern | Data structure |
| Quick Win | Yes |
| Latency Impact | Per query |
| Location | `crates/graph/src/schema.rs` |

## Problem
Nested `Vec<Vec<...>>` structures in graph queries cause multiple allocations per query. Each inner Vec is a separate heap allocation; with a large number of graph nodes or edges, this creates significant allocation pressure during query execution.

## Root Cause
The graph schema uses `Vec<Vec<T>>` for adjacency lists, relationship lists, or query result rows. Each row is a separate Vec, causing N+1 allocations per N-row result set.

## Implementation Plan
### Step 1 — Identify the specific nested Vec uses in schema.rs
Read `crates/graph/src/schema.rs` to find all `Vec<Vec<...>>` fields and their usage patterns. Determine which are query results and which are schema definitions.

### Step 2 — Flatten to a single Vec with index-based access
For adjacency lists or similar fixed-structure data:
```rust
// Instead of Vec<Vec<Edge>>:
struct AdjacencyList {
    edges: Vec<Edge>,       // flat storage
    offsets: Vec<usize>,    // offsets[node] = start index in edges
}
// edges[offsets[node]..offsets[node+1]] gives all edges for that node
```
This is the CSR (Compressed Sparse Row) format — one allocation, cache-friendly.

### Step 3 — Use arena allocation for query results
For per-query result sets, use a bump allocator (`bumpalo`) scoped to the query lifetime. All query allocations come from one contiguous block; freed all at once when the query completes.

### Step 4 — Use SmallVec for small result sets
If graph queries typically return few results (e.g., < 8 items), use `SmallVec<[T; 8]>` to keep small results on the stack.

## Acceptance Criteria
- [ ] No `Vec<Vec<...>>` for data with known-at-query-time structure (replaced by flat Vec + offsets)
- [ ] Graph query allocation count reduced vs baseline
- [ ] Graph query tests produce correct results
- [ ] CSR or arena pattern documented in a comment

## Files to Change
- `crates/graph/src/schema.rs` — flatten Vec<Vec<>> to single Vec + offsets or SmallVec where appropriate
