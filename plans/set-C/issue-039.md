# Issue #039 — Reddit: title cloned then chained

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 1 Option + 1 field clone |
| Location | `crates/collectors/src/social/reddit.rs:131,134-135` |

## Problem
Post title is optionally cloned before method chaining when `as_deref()` would suffice. This allocates a String copy of the title just to call a method on it.

## Root Cause
The code at lines 131 and 134-135 likely does:
```rust
let title = post.title.clone();
let lower = title.as_ref().map(|t| t.to_lowercase());
```
when it could do:
```rust
let lower = post.title.as_deref().map(str::to_lowercase);
```
or simply borrow without cloning.

## Implementation Plan
### Step 1 — Replace clone + chain with as_deref()
Replace:
```rust
post.title.clone().map(|t| t.some_method())
```
with:
```rust
post.title.as_deref().map(|t| t.some_method())
```
`as_deref()` converts `Option<String>` to `Option<&str>` without allocation.

### Step 2 — Check line 131 and 134-135 for all clone sites
Read both lines and ensure both clones are addressed.

### Step 3 — Verify no borrow checker issues
If the title field is later moved or used elsewhere in the function, the borrow-based approach may conflict. In that case, use `as_deref()` for the specific chain and avoid cloning the whole String.

## Acceptance Criteria
- [ ] No `.clone()` on post.title at `reddit.rs:131,134-135`
- [ ] `as_deref()` or equivalent borrow-based pattern used
- [ ] Reddit collector tests pass
- [ ] Social event normalization test passes

## Files to Change
- `crates/collectors/src/social/reddit.rs` — replace title clone at lines 131, 134-135 with as_deref() or borrowed pattern
