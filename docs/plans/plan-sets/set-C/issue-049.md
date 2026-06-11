# Issue #049 — Format errors in health checks

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Allocation |
| Quick Win | Yes |
| Latency Impact | Per error (rare) |
| Location | `crates/api/src/routes/venue_health.rs:129-140` |

## Problem
Error messages are eagerly `format!()`-ed in health check routes. While health check errors are rare (only occur when a venue is unreachable), the pattern of eagerly formatting errors is inconsistent with the rest of the codebase direction (towards thiserror/anyhow).

## Root Cause
At `venue_health.rs:129-140`, error conditions use `format!("venue {}: {}", name, e)` which allocates a String even when the error will be wrapped in a response type that only formats on serialization.

## Implementation Plan
### Step 1 — Define a VenueHealthError type with thiserror
```rust
#[derive(Debug, thiserror::Error)]
pub enum VenueHealthError {
    #[error("venue {venue}: connection failed: {source}")]
    ConnectionFailed { venue: String, #[source] source: reqwest::Error },
    #[error("venue {venue}: timeout")]
    Timeout { venue: String },
}
```

### Step 2 — Replace format!() with typed error construction
Replace:
```rust
Err(format!("venue {}: {}", venue_name, e))
```
with:
```rust
Err(VenueHealthError::ConnectionFailed { venue: venue_name.to_string(), source: e })
```
The error string is only formatted when the error is displayed or serialized.

### Step 3 — Update the route handler to return typed errors
Update the handler return type from `Result<_, String>` to `Result<_, VenueHealthError>`. Axum can map typed errors to HTTP responses.

## Acceptance Criteria
- [ ] No `format!()` on error construction in `venue_health.rs:129-140`
- [ ] Typed error enum with thiserror Display impl
- [ ] Health check endpoint returns correct HTTP status on venue error
- [ ] Health check test passes for both healthy and failed venue states

## Files to Change
- `crates/api/src/routes/venue_health.rs` — replace format! error construction at lines 129-140 with thiserror typed errors
