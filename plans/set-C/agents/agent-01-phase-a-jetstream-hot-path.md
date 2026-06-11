# Agent Query — Remove JetStream from the Decision Path
## Covers Issues: #1
## Phase: A
## Estimated Effort: 3–4 weeks
## Prerequisites: None (but complete #10 build flags first so benchmarks are meaningful)

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

Every market tick in the trading-bot currently awaits a durable JetStream publish ACK before the strategy sees it. This means the strategy waits on a disk write (on a separate NATS server process) for every price update — adding 0.5–5 ms of latency where 100 ns is achievable. This is the single biggest bottleneck in the system and must be resolved before any other latency optimization is meaningful. The fix is to move the market-data producers in-process and connect stages with a lock-free SPSC ring, relegating JetStream to an async "tee" that never blocks the strategy path.

## Codebase Context

- `crates/event-bus/src/publish.rs` — `Publisher::publish` is called inline on the strategy-input path; it calls `js.publish_async(...).await` which blocks until NATS server acknowledges persistence.
- `apps/platform/src/main.rs` — the strategy evaluation loop; currently calls `Publisher::publish` directly.
- `crates/collectors/src/crypto/kraken.rs` — Kraken WS collector; currently a separate process that publishes via NATS.
- `docs/adr/0003-nats-jetstream-event-fabric.md` — ADR describing the current JetStream-first design.
- `Cargo.toml` — workspace manifest; `rtrb` is not yet a dependency.

The problematic pattern in `publish.rs`:
```rust
// publish.rs ~line 30 — blocks strategy path on server ACK
pub async fn publish(&self, subject: &str, payload: Bytes) -> Result<()> {
    let ack = self.js.publish(subject, payload).await?;
    ack.await?;   // ← waits for server disk-write ACK
    Ok(())
}
```

## Task

### Fix #1 — In-process hot path: SPSC ring pipeline

**Problem:** `Publisher::publish` in `crates/event-bus/src/publish.rs` (around line 30) is called inline on the strategy-input path. It issues a NATS JetStream publish and awaits the server ACK, which requires disk persistence on the NATS server before returning. This adds 0.5–5 ms per tick to the strategy latency.

**Solution:** Move the market-data producers in-process and wire pipeline stages together with bounded SPSC rings. JetStream becomes a background "tee" — it receives every event but never blocks the strategy path.

**Implementation steps:**

1. Add `rtrb = "0.3"` to the workspace `Cargo.toml` under `[workspace.dependencies]`. This is the bounded lock-free SPSC ring that will connect pipeline stages.

2. Move the Kraken collector (`crates/collectors/src/crypto/kraken.rs`) and any Coinbase user-channel collector into `apps/platform/src/` as supervised `tokio::spawn` tasks. These become in-process async tasks rather than separate processes communicating via NATS. The collector task owns the WS connection and writes raw parsed events into the ring.

3. Build the decision pipeline as a chain of bounded SPSC rings inside `apps/platform/src/`:
   - **Stage 1 — socket-reader task:** owns the WS connection; receives raw frames; pushes `RawTick` into `ring_raw` (capacity: 4096 items).
   - **Stage 2 — bar-builder/feature task:** reads `ring_raw`; computes bars and features; pushes `WorldEvent` into `ring_world` (capacity: 1024 items).
   - **Stage 3 — strategy-eval task:** reads `ring_world`; runs `process_event`; pushes `OrderIntent` into `ring_intent` (capacity: 256 items).
   - **Stage 4 — risk/exec task:** reads `ring_intent`; applies risk checks; submits orders.

4. Add a "tee" task: after the socket-reader pushes to `ring_raw`, it also sends to an unbounded `tokio::sync::mpsc::UnboundedSender<RawTick>`. A separate `tee_task` owns the `Publisher` and calls `js.publish(...)` without awaiting the ACK — fire-and-forget from the strategy's perspective. If the tee task falls behind, it drops events (the JetStream write is best-effort for replay, not for live trading decisions).

5. The strategy eval task (Stage 3) reads **only** from `ring_world`. Delete all `Publisher::publish` calls from every code path that the strategy evaluation touches. The strategy eval must never await a network call.

6. Satellites (web scraper collector, reddit collector, embedder) stay on NATS unchanged — they are low-frequency and not on the hot path.

7. Add a grep-based CI check in `.github/workflows/` or as a comment in the `Makefile`/`justfile`:
   ```
   grep -r "Publisher::publish" apps/platform/src/ | grep -v "tee_task\|tee.rs"
   ```
   This must return zero results. The `Publisher` may only appear in `tee_task`.

8. Amend `docs/adr/0003-nats-jetstream-event-fabric.md` with a new section titled **"Amendment: JetStream is the tail, not the spine"** explaining:
   - The tee architecture: hot path uses in-process rings; JetStream receives via async tee.
   - Why: JetStream ACK latency is incompatible with sub-millisecond strategy response times.
   - Replay guarantee: the tee task retries on publish failure with a local buffer; events are eventually durable.

**Acceptance test:**
- Run the platform binary with the Kraken WS collector in-process and measure `tick_to_intent_p99` via a tracing span from socket frame receipt to `OrderIntent` push. Target: < 50 µs p99.
- Run a replay test that counts events written to JetStream; verify the count matches the events processed by the strategy eval task (within the retry window).
- `grep -r "Publisher::publish" apps/platform/src/` returns zero results.

## Overall Acceptance Criteria
- [ ] tick-to-intent p99 < 50 µs measured from socket frame receipt to OrderIntent emit
- [ ] Zero `Publisher::publish` calls reachable from the strategy evaluation path (CI grep passes)
- [ ] JetStream still receives every market event (verified by replay test counting events)
- [ ] `docs/adr/0003-nats-jetstream-event-fabric.md` updated with "JetStream is the tail, not the spine" section
- [ ] `cargo test` passes
- [ ] `cargo build --release` succeeds with rtrb dependency

## Files to Touch
- `crates/event-bus/src/publish.rs` — add fire-and-forget publish variant; remove await on ACK from hot path
- `apps/platform/src/main.rs` — wire in-process pipeline with rtrb rings; spawn tee task
- `apps/platform/src/hot_path.rs` (new) — define pipeline stage tasks and ring types
- `apps/platform/src/tee.rs` (new) — tee task that forwards to JetStream asynchronously
- `crates/collectors/src/crypto/kraken.rs` — adapt collector to run as in-process task writing to rtrb ring
- `docs/adr/0003-nats-jetstream-event-fabric.md` — add amendment section
- `Cargo.toml` — add `rtrb = "0.3"` to workspace dependencies
