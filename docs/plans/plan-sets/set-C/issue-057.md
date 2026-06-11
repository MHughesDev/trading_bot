# Issue #057 — Manifest: feature.clone() on insert

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | Feature count clones per manifest |
| Location | `crates/strategy-runtime/src/manifest.rs:123` |

## Problem
`feature.clone()` is called when inserting features into the `required_features` Vec, cloning the feature name string per feature per manifest compile. For a strategy with 50 features, each manifest compile allocates 50 feature name Strings.

## Root Cause
The `required_features` field on the compiled manifest stores owned feature name Strings. Each feature from the definition is cloned into the Vec during compilation.

## Implementation Plan
### Step 1 — Coordinate with #56 (move dedup to load time)
If #56 is implemented, the dedup structures (including the required_features list) are built once at definition load time. The manifest compile step then references the pre-built feature list without cloning.

### Step 2 — Use Arc<str> for feature names
If feature names are stored on the definition as `Arc<str>`, cloning them into the manifest is a cheap atomic increment:
```rust
required_features: Vec<Arc<str>>,
// ...
required_features.push(Arc::clone(&feature.name));
```

### Step 3 — Coordinate with #4 (u16 slot IDs)
Long-term: after #4 lands, feature names are interned as `u16` slot IDs. The required_features list becomes `Vec<u16>` — no String storage or cloning at all.

### Step 4 — Consolidate with #56 and #68
Fix all three manifest issues (#56, #57, #68) in a single PR.

## Acceptance Criteria
- [ ] No `feature.clone()` (String clone) at `manifest.rs:123`
- [ ] Feature names stored as `Arc<str>` or (after #4) as `u16` slot IDs
- [ ] Manifest compile allocates O(1) not O(feature_count)
- [ ] All manifest tests pass

## Files to Change
- `crates/strategy-runtime/src/manifest.rs` — replace feature.clone() at line 123 with Arc::clone or u16 slot ID
