# Issue #001 — JetStream in the decision path

## Summary
| Field | Value |
|-------|-------|
| Severity | High |
| Phase | A |
| Pattern | Architecture |
| Quick Win | No |
| Latency Impact | 0.5–5 ms per event vs ~100 ns achievable |
| Location | `crates/event-bus/src/publish.rs:30` |

## Problem
Every market tick awaits a durable JetStream publish ack, then crosses collector-process → NATS server → platform-process before the strategy sees it. The strategy waits on a disk write to learn a price. This is the single largest architectural bottleneck in the decision path — all other latency wins are capped until this is resolved.

## Root Cause
The `Publisher::publish` call in `event-bus/src/publish.rs` issues a NATS JetStream `publish_async` and awaits the server ACK inline. Because JetStream requires the server to persist the message to disk before ACKing, the strategy cannot receive the event until after disk I/O completes on a separate process.

## Implementation Plan
### Step 1 — Move trade-critical collectors into `apps/platform`
Move Kraken and Coinbase user-channel collectors into `apps/platform` as supervised tokio tasks rather than separate processes. This eliminates the inter-process hop for the hot path.

### Step 2 — Add `rtrb` to workspace dependencies
Add the `rtrb` crate (bounded, lock-free SPSC ring buffer) to `Cargo.toml`. This is the in-process channel for the decision path.

### Step 3 — Build the in-process decision pipeline
Wire the pipeline: socket-reader thread → builders/features → strategy eval → risk/exec using bounded lock-free SPSC rings from `rtrb`. The strategy receives events from a ring, never from NATS.

### Step 4 — Add a tee task for JetStream
After the socket reader pushes to the ring, also send the event to a separate tee task via an unbounded mpsc channel. The tee task owns the `Publisher` and calls `js.publish` asynchronously — storage, UI, and replay still receive every event, but the strategy path never awaits the ACK.

### Step 5 — Audit all strategy-input code paths
Ensure zero `Publisher::publish` calls are reachable from the strategy-input path. Add a `#[forbid]` comment or CI lint to prevent regression.

### Step 6 — Write ADR-0003 amendment
Amend `docs/adr/0003-nats-jetstream-event-fabric.md` with a section titled "JetStream is the tail, not the spine" explaining that JetStream handles durability/replay but is not in the decision path.

### Step 7 — Leave slow satellites unchanged
Web scraper, Reddit, and embedder collectors remain on NATS unchanged — they are not latency-sensitive.

## Acceptance Criteria
- [ ] tick-to-intent p99 < 50 µs measured end-to-end in platform process
- [ ] Zero `Publisher::publish` calls reachable from the strategy-input code path (verified by grep or CI lint)
- [ ] JetStream still receives every market event (verified by replay test)
- [ ] ADR-0003 updated with the architectural rationale

## Files to Change
- `crates/event-bus/src/publish.rs` — remove from hot path; keep for tee task use only
- `apps/platform/src/main.rs` — add in-process collector tasks and SPSC ring pipeline
- `crates/collectors/src/crypto/kraken.rs` — refactor to run as supervised tokio task inside platform
- `docs/adr/0003-nats-jetstream-event-fabric.md` — add amendment section
