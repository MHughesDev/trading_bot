# Issue #059 — Graph: dt.as_key().to_owned per data type

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | ~30 type clones at init |
| Location | `crates/graph/src/populate.rs:90` |

## Problem
`.as_key().to_owned()` is called on every DataType at graph initialization, allocating a String for every data type in the registry. With ~30 data types, this is 30 String allocations that could be avoided by returning `&'static str` instead of a computed String.

## Root Cause
The `as_key()` method on `DataType` (or similar) returns either a `String` or a `&str` that is then `.to_owned()`. If the key is a fixed string per variant, it should return `&'static str` from a match expression — zero allocation.

## Implementation Plan
### Step 1 — Check what as_key() returns
Read the DataType implementation to see if `as_key()` returns a `String` (computed) or `&str` (static). If it computes the key dynamically (e.g., by formatting), move to static.

### Step 2 — Change as_key() to return &'static str
```rust
impl DataType {
    pub fn as_key(&self) -> &'static str {
        match self {
            Self::Trade => "trade",
            Self::Bar => "bar",
            Self::Feature => "feature",
            // ... all variants
        }
    }
}
```

### Step 3 — Remove .to_owned() at populate.rs:90
With a `&'static str` return, `.to_owned()` is unnecessary if the caller can store `&'static str`. If the graph storage requires an owned key, change the storage type to accept `&'static str` keys (using `HashMap<&'static str, ...>`).

### Step 4 — Update graph storage type
If the graph uses `HashMap<String, Node>`, change to `HashMap<&'static str, Node>` for data type keys. This allows direct use of `as_key()` without `.to_owned()`.

## Acceptance Criteria
- [ ] No `.to_owned()` at `populate.rs:90` for data type keys
- [ ] `DataType::as_key()` returns `&'static str`
- [ ] Graph initialization: zero String allocations for data type keys
- [ ] Graph population test passes: all data types registered correctly

## Files to Change
- `crates/graph/src/populate.rs` — remove .to_owned() at line 90; use &'static str key
