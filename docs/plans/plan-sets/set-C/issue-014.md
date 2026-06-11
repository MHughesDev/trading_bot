# Issue #014 — WS JSON per message

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | E |
| Pattern | Serialization |
| Quick Win | Yes |
| Latency Impact | 1 JSON encode per WS frame |
| Location | `crates/api/src/ws/live.rs:141` |

## Problem
Every WS push to the UI serializes the full message as JSON, even for frequent delta updates. High-frequency price ticks, P&L updates, and position changes all pay a full JSON encode cycle per frame. For a UI receiving 100 updates/sec across 20 instruments, this is 2,000 JSON encodes per second on the API server.

## Root Cause
`live.rs:141` calls `serde_json::to_string()` on the full message for every WS send. There is no binary encoding option and no delta mechanism — every frame contains the full current state.

## Implementation Plan
### Step 1 — Use postcard for binary WS encoding
Add `postcard` to workspace dependencies. For hot WS streams (price ticks, P&L deltas, position updates), encode with `postcard::to_allocvec(msg)` instead of `serde_json::to_string(msg)`. The React frontend decodes using a postcard WASM decoder or a matching JSON equivalent.

### Step 2 — Implement delta frames for high-frequency streams
For price tick streams, send only changed fields rather than the full message. Define a `PriceDelta` type with `instrument_id: u32`, `price: f64`, `timestamp: i64`. Encode as postcard (12 bytes per frame vs ~200 bytes JSON).

### Step 3 — Keep JSON for initial state and rare updates
Subscriptions, initial snapshot, and UI control messages remain JSON for human-readability and debuggability. Only the high-frequency streaming path uses binary.

### Step 4 — Update the React WS client
Update the frontend WS client to detect message type and use the appropriate decoder. Binary frames use postcard/WASM decoder; JSON frames use standard JSON.parse.

## Acceptance Criteria
- [ ] Hot price tick WS frames encoded as postcard binary, not JSON
- [ ] Frame size for price tick < 20 bytes (vs ~200 bytes JSON)
- [ ] Initial state / subscription messages still use JSON
- [ ] React client decodes binary frames correctly
- [ ] No JSON encode on WS hot path at `live.rs:141`

## Files to Change
- `crates/api/src/ws/live.rs` — replace serde_json::to_string with postcard encoding on hot streams
