# Issue #032 — Account source: credential parsing with clones

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | C |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 1 Vec + 2 String clones per fetch |
| Location | `crates/execution/src/account/alpaca.rs:43,48,51` |

## Problem
Credentials are cloned from config into a Vec, then individual strings are cloned again for the HTTP request. Each account fetch (periodic reconciliation or startup sync) allocates an intermediate Vec plus two full String copies that are immediately used and dropped.

## Root Cause
The account source reads credentials from a config struct by cloning them into an intermediate vector, then clones them again to insert into request headers. This multi-step copy was likely a workaround for borrow checker constraints that has a cleaner solution.

## Implementation Plan
### Step 1 — Store credentials as Arc<str> at startup
At adapter initialization, parse credentials once and store as `Arc<str>`:
```rust
struct AlpacaAccount {
    api_key: Arc<str>,
    api_secret: Arc<str>,
}
```

### Step 2 — Pass Arc<str> clones to HTTP requests
When building request headers, clone the `Arc<str>` (atomic increment). No String allocation.

### Step 3 — Remove intermediate Vec allocation
The Vec at line 43 that collects credentials is unnecessary. Pass the credential strings directly to the request builder.

### Step 4 — Apply to all account adapters
Apply the same pattern to `kraken.rs` and `kalshi.rs` account sources.

## Acceptance Criteria
- [ ] Zero String allocation for credentials on each account fetch
- [ ] Credentials stored as `Arc<str>` on the adapter struct
- [ ] No intermediate Vec allocation at `alpaca.rs:43`
- [ ] Account fetch (Alpaca, Kraken, Kalshi) tests pass with new credential handling

## Files to Change
- `crates/execution/src/account/alpaca.rs` — store Arc<str> credentials; remove Vec + clone at lines 43, 48, 51
- `crates/execution/src/account/kraken.rs` — same pattern
- `crates/execution/src/account/kalshi.rs` — same pattern
