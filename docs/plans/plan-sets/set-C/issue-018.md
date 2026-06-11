# Issue #018 — Node ID and universe filtering with string compares

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | B |
| Pattern | Search |
| Quick Win | No |
| Latency Impact | String hashing per lookup |
| Location | `crates/strategy-runtime/src/nodes/mod.rs:43-78` |

## Problem
Node IDs are stored and compared as Strings for every pipeline lookup. Each pipeline stage must hash a String to find its configuration, rather than doing a direct array index lookup. This adds unnecessary hashing cost proportional to the number of pipeline lookups per tick.

## Root Cause
Node IDs are arbitrary strings defined in the strategy manifest. There is no interning or compile-time assignment of numeric IDs, so all lookups must use String-keyed HashMaps.

## Implementation Plan
### Step 1 — Intern node IDs at compile time
During the strategy compile step (coordinated with #3), assign a `NodeId(u32)` to each node defined in the manifest. Build a registry `HashMap<&str, NodeId>` at compile time; the numeric ID is used at runtime.

### Step 2 — Replace String-keyed node map with Vec or array
Change the pipeline node storage from `HashMap<String, Node>` to `Vec<Node>` indexed by `NodeId`. Node lookup becomes `nodes[node_id.0 as usize]` — a direct array access with no hashing.

### Step 3 — Update all pipeline stage references
Change all pipeline stage code that looks up nodes by String to use `NodeId` index lookup. This includes filter, rank, limit, and any other node types in `nodes/mod.rs:43-78`.

### Step 4 — Coordinate with #3 compile step
NodeId assignment happens in the same compile step as #3 (bytecode compilation). These changes should land together.

## Acceptance Criteria
- [ ] No String hashing for node lookup during pipeline execution
- [ ] Node storage is `Vec<Node>` indexed by `NodeId(u32)`
- [ ] NodeId interned at compile time; never re-created per tick
- [ ] All node lookup sites in nodes/mod.rs use direct index access

## Files to Change
- `crates/strategy-runtime/src/nodes/mod.rs` — intern node IDs; replace String-keyed map with Vec; update lines 43-78
