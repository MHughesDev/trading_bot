# Issue #056 — Manifest: HashSet rebuilt on every compile

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | E |
| Pattern | Allocation |
| Quick Win | Yes |
| Latency Impact | Per manifest compile: 2 HashSets + insert checks + collect |
| Location | `crates/strategy-runtime/src/manifest.rs:95,119` |

## Problem
`seen_lanes` and `seen_features` HashSets are built to deduplicate inputs on every call to `compile_manifest()`. If manifests are compiled at startup and cached (which they should be), this is a one-time cost. But if `compile_manifest` is called repeatedly (e.g., on hot-reload or per-tick), this becomes significant.

## Root Cause
Deduplication logic is embedded in the compile step rather than the definition-load step. Definitions are parsed once but the dedup work runs every time a manifest is compiled from a definition.

## Implementation Plan
### Step 1 — Move dedup to definition load time
When a strategy definition is first parsed (not when it is compiled into a manifest), run the dedup logic:
```rust
impl StrategyDefinition {
    pub fn load(raw: &str) -> Result<Self, ParseError> {
        let mut def = parse_definition(raw)?;
        def.deduplicate_lanes();   // runs once at load
        def.deduplicate_features(); // runs once at load
        Ok(def)
    }
}
```

### Step 2 — Remove HashSet dedup from compile_manifest
After #56, `compile_manifest` receives a pre-deduplicated definition. Remove the HashSet construction at lines 95 and 119. `compile_manifest` becomes a cheap transformation (linear scan, no HashSet needed).

### Step 3 — Consolidate with #57 and #68
Issues #56, #57, and #68 all describe aspects of the same problem: dedup work done at compile time rather than definition-load time. Resolve all three together.

## Acceptance Criteria
- [ ] `compile_manifest()` contains no HashSet construction
- [ ] Dedup of lanes and features done once at definition load
- [ ] Manifest compile is a pure, allocation-light transformation
- [ ] All strategy manifest tests pass
- [ ] Hot-reload: re-compiling a manifest does not re-dedup (dedup state cached on definition)

## Files to Change
- `crates/strategy-runtime/src/manifest.rs` — move dedup logic from compile_manifest to definition load; remove HashSets at lines 95, 119
