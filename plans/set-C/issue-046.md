# Issue #046 — Collector: repeated as_deref+unwrap_or

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Allocation |
| Quick Win | Yes |
| Latency Impact | 1 string literal clone per field |
| Location | `crates/collectors/src/equity/alpaca_data.rs:89-101` |

## Problem
`field.as_deref().unwrap_or("unknown")` is repeated many times. If `field` is `None` and the pattern falls back to `"unknown"`, this is correct and zero-cost. However, if the pattern is used with `.to_owned()` or `.to_string()` appended, it clones the static string literal unnecessarily.

## Root Cause
If `unwrap_or("unknown").to_owned()` is used, it allocates a heap String for the literal `"unknown"` on every None field. This occurs multiple times in the normalization code at lines 89-101.

## Implementation Plan
### Step 1 — Audit lines 89-101 for .to_owned() after unwrap_or
Check if the pattern is:
```rust
field.as_deref().unwrap_or("unknown").to_owned()
```
If so, this allocates "unknown" as a String on None.

### Step 2 — Change field types to &'static str for None defaults
If the downstream struct accepts `String`, use:
```rust
field.unwrap_or_else(|| "unknown".to_string())  // still allocates
```
Or better: change the struct field to `Option<String>` and preserve None rather than substituting a default. Only convert to String when required.

### Step 3 — Use &'static str for field name defaults
For field names that are static defaults (e.g., exchange name = "UNKNOWN" when not provided), use a `const` or `&'static str` in the domain type. Avoid `to_owned()` on static strings.

### Step 4 — Apply to all repeated patterns at 89-101
Fix all occurrences in the block, not just the first one.

## Acceptance Criteria
- [ ] No `.to_owned()` on static string literals at `alpaca_data.rs:89-101`
- [ ] `&'static str` used for static default values; no heap allocation for default fields
- [ ] Alpaca equity collector tests pass
- [ ] Normalized trade events have correct field values for both Some and None inputs

## Files to Change
- `crates/collectors/src/equity/alpaca_data.rs` — remove .to_owned() on static defaults at lines 89-101; use &'static str
