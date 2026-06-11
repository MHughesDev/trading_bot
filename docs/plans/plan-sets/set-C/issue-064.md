# Issue #064 — Account source: repeated map_err formatting

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | G |
| Pattern | Error handling |
| Quick Win | Yes |
| Latency Impact | 50+ error string allocations (on error paths only) |
| Location | `crates/execution/src/account/alpaca.rs:44+` and equivalents in kraken/kalshi/oanda/coinbase |

## Problem
`.map_err(|e| AccountSourceError::Http(e.to_string()))` appears 50+ times across 5 account adapters. Each invocation allocates a String for the error message — even though these are error paths (infrequent), the repetition is a code hygiene problem that makes errors harder to type-match and diagnose.

## Root Cause
The `AccountSourceError` type wraps errors as `String` (i.e., `Http(String)`) rather than retaining the original error type. This forces `.to_string()` at the point of error construction, losing type information and allocating eagerly.

## Implementation Plan
### Step 1 — Redefine AccountSourceError with thiserror
```rust
#[derive(Debug, thiserror::Error)]
pub enum AccountSourceError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("authentication failed: {0}")]
    Auth(String),
}
```
Using `#[from]`, errors convert automatically via `?` — no `.map_err()` or `.to_string()` needed.

### Step 2 — Remove all .map_err() chains
Replace:
```rust
client.get(url).send().await.map_err(|e| AccountSourceError::Http(e.to_string()))
```
with:
```rust
client.get(url).send().await?  // auto-converts via #[from]
```

### Step 3 — Apply to all 5 account adapters
Apply the thiserror refactor to: alpaca.rs, kraken.rs, kalshi.rs, oanda.rs, coinbase.rs. Each likely has 8-15 `.map_err()` calls to remove.

### Step 4 — Update error handling at call sites
Callers that match on `AccountSourceError` may need to update match arms to use the new typed variants instead of `Http(string)`.

## Acceptance Criteria
- [ ] Zero `.map_err(|e| AccountSourceError::Http(e.to_string()))` patterns across all 5 adapters
- [ ] `AccountSourceError` uses thiserror with typed `#[from]` conversions
- [ ] Error formatting deferred to Display impl; no allocation on error construction
- [ ] Account source error integration tests pass (error cases handled correctly)

## Files to Change
- `crates/execution/src/account/alpaca.rs` — thiserror AccountSourceError; remove map_err chains
- `crates/execution/src/account/kraken.rs` — same
- `crates/execution/src/account/kalshi.rs` — same
- `crates/execution/src/account/oanda.rs` — same
- `crates/execution/src/account/coinbase.rs` — same
