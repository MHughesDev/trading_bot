# Issue #068 — Manifest: dedup work done at runtime, not parse time

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | E |
| Pattern | Algorithm |
| Quick Win | Yes |
| Latency Impact | Full tree walk + 2 HashSet dedup ops per manifest compile |
| Location | `crates/strategy-runtime/src/manifest.rs:94-127` |

## Problem
Every call to `compile_manifest()` rebuilds deduplication structures and walks the entire definition tree. This includes building `seen_lanes` and `seen_features` HashSets from scratch. If `compile_manifest` is called per hot-reload or per-instance, this becomes significant. The dedup should be amortized at definition parse time.

## Root Cause
`compile_manifest()` performs three things that should be separated:
1. Tree walk to collect all lanes and features (should be at parse time)
2. Dedup via HashSets (should be at parse time)
3. Manifest construction (this alone should remain in compile_manifest)

## Implementation Plan
### Step 1 — Consolidated fix with #56 and #57
Issues #56, #57, and #68 all describe the same problem from different angles:
- #56: HashSets rebuilt on every compile
- #57: feature.clone() on insert
- #68: full tree walk on every compile

Fix all three together in one refactoring of the manifest compilation pipeline.

### Step 2 — Separate parse from compile
At definition parse time:
```rust
impl StrategyDefinition {
    fn parse(raw: &str) -> Result<Self, ParseError> {
        let mut def = parse_raw(raw)?;
        def.required_lanes = collect_lanes_deduped(&def);     // tree walk + dedup
        def.required_features = collect_features_deduped(&def); // tree walk + dedup
        Ok(def)
    }
}
```

### Step 3 — Make compile_manifest a cheap lookup
After the refactor, `compile_manifest` becomes:
```rust
fn compile_manifest(def: &StrategyDefinition) -> StrategyManifest {
    StrategyManifest {
        lanes: Arc::clone(&def.required_lanes),
        features: Arc::clone(&def.required_features),
        // ... other fields from def
    }
}
```
No tree walk, no HashSet, no clone — just Arc references and Copy fields.

### Step 4 — Verify correct results after refactor
Test: parse a definition with duplicate lanes and features. Verify the compiled manifest deduplicates correctly. Verify that multiple compile calls produce identical manifests (idempotent).

## Acceptance Criteria
- [ ] `compile_manifest()` performs no tree walk and no HashSet dedup
- [ ] Tree walk and dedup done once at definition parse time
- [ ] Multiple compile calls from the same definition are O(1) / O(field_count)
- [ ] Manifest correctness test: definitions with duplicates produce correct deduplicated manifests
- [ ] Consolidated with #56 and #57 in one PR

## Files to Change
- `crates/strategy-runtime/src/manifest.rs` — separate parse (tree walk + dedup) from compile (manifest construction) at lines 94-127; consolidate with #56, #57
