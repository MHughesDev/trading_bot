# Issue #038 — Debug formatting per strategy request

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Allocation |
| Quick Win | Yes |
| Latency Impact | 1 Debug derive + lowercase per call |
| Location | `crates/api/src/routes/strategies.rs:241-242` |

## Problem
`{:?}` formatting is applied to enums and `.to_lowercase()` is called on the result on every strategy list request. Both operations allocate: Debug formatting allocates the enum string, and `.to_lowercase()` allocates a new String. These are not hot-path operations but are trivially fixable.

## Root Cause
Enum variants are formatted using the Debug trait (`{:?}`) which produces `"VariantName"`. Then `.to_lowercase()` is called to get `"variantname"` for the API response. Both are unnecessary if the enum implements a direct `as_str()` method.

## Implementation Plan
### Step 1 — Add serde rename_all = "lowercase" to the enum
If the enum is serialized as part of a JSON response, add:
```rust
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrategyStatus { Active, Paused, Stopped }
```
serde handles the lowercase conversion at serialization time with no allocation.

### Step 2 — Add an as_str() method for non-serde contexts
```rust
impl StrategyStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
        }
    }
}
```
Replace `format!("{:?}", status).to_lowercase()` with `status.as_str()`. Returns a `&'static str` — zero allocation.

### Step 3 — Remove Debug format + to_lowercase at lines 241-242
Replace the two-step `{:?}` + `.to_lowercase()` pattern with `status.as_str()` or the serde serializer.

## Acceptance Criteria
- [ ] No `{:?}` formatting + `.to_lowercase()` at `strategies.rs:241-242`
- [ ] Enum serializes to lowercase via serde or `as_str()` with no allocation
- [ ] Strategy list API returns correct lowercase status strings
- [ ] API test for strategy listing passes

## Files to Change
- `crates/api/src/routes/strategies.rs` — replace Debug format + to_lowercase at lines 241-242 with as_str() or serde rename_all
