# Issue #015 — Panel/instrument IDs cloned in loop

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 2 String clones per subscription |
| Location | `crates/api/src/ws/live.rs:110,113` |

## Problem
Panel and instrument ID Strings are cloned inside the subscription loop on every WS frame. While individually small, this occurs in the inner loop of the WS dispatch path, generating unnecessary allocations proportional to the number of active subscriptions per frame.

## Root Cause
The subscription loop at `live.rs:110,113` clones panel and instrument ID strings to pass them to a function that could accept `&str` or `Arc<str>` instead.

## Implementation Plan
### Step 1 — Identify the exact clone sites
Read `crates/api/src/ws/live.rs` at lines 110 and 113. Determine whether the clones are `.clone()` calls on String fields or `to_owned()` on &str.

### Step 2 — Change to Arc<str> or pass by reference
Option A (preferred): Change panel_id and instrument_id fields in the Subscription struct to `Arc<str>`. The Arc clone is cheap (atomic increment); no heap allocation.

Option B: Change the function receiving the cloned strings to accept `&str` parameters. Remove the `.clone()` calls.

### Step 3 — Apply consistently with #2 and #5
This change should use the same `Arc<str>` or interned `u32` approach as the rest of the codebase. If #2 is landed, instrument IDs are already `InstrumentId(u32)` — use those.

## Acceptance Criteria
- [ ] Zero `.clone()` calls on panel_id or instrument_id strings at `live.rs:110,113`
- [ ] Subscription struct uses `Arc<str>` or interned u32 for IDs
- [ ] WS subscription test passes

## Files to Change
- `crates/api/src/ws/live.rs` — replace String clone at lines 110, 113 with Arc<str> or reference passing
