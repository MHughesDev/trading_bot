# Issue #020 — Error messages formatted unnecessarily

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Allocation |
| Quick Win | Yes |
| Latency Impact | 1–2 allocs per error (rare) |
| Location | `crates/api/src/routes/dashboard.rs:41` |

## Problem
Error messages are eagerly formatted with `format!()` on API error paths. While errors are rare, eagerly formatting means the allocation happens even when the error will be swallowed or logged at a lower severity that never renders the message.

## Root Cause
`format!("some error: {}", e)` is called at the point the error is detected, before it is known whether the formatted string will be used. This is a common Rust anti-pattern — `format!` always allocates even if the result is never displayed.

## Implementation Plan
### Step 1 — Replace format! with thiserror or anyhow
Replace:
```rust
Err(format!("DB error: {}", e))
```
with:
```rust
Err(DashboardError::Db(e))  // using thiserror variant
```
or:
```rust
Err(anyhow::Error::from(e).context("DB error"))
```
`anyhow::Error` stores the cause chain lazily; the error string is only formatted when `Display` is called.

### Step 2 — Define a typed error enum for the dashboard route
Using `thiserror`:
```rust
#[derive(Debug, thiserror::Error)]
pub enum DashboardError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("not found: {0}")]
    NotFound(String),
}
```
This gives typed matching, lazy formatting, and no allocation on construction.

### Step 3 — Apply pattern consistently
Scan all routes in `crates/api/src/routes/` for `format!` on error paths. Apply the same thiserror pattern.

## Acceptance Criteria
- [ ] No `format!()` calls on error construction paths in dashboard.rs
- [ ] Typed error enum defined with thiserror
- [ ] Error formatting deferred to Display impl
- [ ] API error response tests pass

## Files to Change
- `crates/api/src/routes/dashboard.rs` — replace format! with typed error at line 41
