# Agent Query — Warm HTTP/2 Order Pools + Atomic Rate Budget + Atomic WS Throttle
## Covers Issues: #9, #43, #45
## Phase: D
## Estimated Effort: 2–3 days
## Prerequisites: None (fully independent)

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

Broker order submission uses per-call `reqwest` clients with no guarantee of a warm TCP+TLS connection. The most latency-sensitive operation in the system (order submission) can pay a full TCP+TLS handshake of 100–300 ms on every order. Additionally, the rate budget tracker and WS throttle both use `Mutex<u32>` to protect a counter — a lock that is contended on every incoming event and every outgoing WS frame, respectively. Both can be replaced with `AtomicU32` for lock-free increments.

## Codebase Context

- `crates/execution/src/alpaca.rs` — around line 123, order submission creates or uses a `reqwest::Client` per call, with no HTTP/2 keep-alive configuration.
- `crates/execution/src/venues/kalshi.rs`, `oanda.rs`, `tradier.rs`, `tradovate.rs` — same pattern; each venue adapter should be audited.
- `crates/demand-manager/src/rate_budget.rs` — around lines 76, 92, 100: `Mutex<u32>` locked on every rate-limit check (every incoming event or subscription request).
- `crates/ui-gateway/src/throttle.rs` — around lines 69, 81, 98, 104: `Mutex<u32>` locked on every outgoing WS frame to enforce per-connection frame rate limits.

The problematic per-call client pattern:
```rust
// execution/src/alpaca.rs ~line 123
pub async fn submit_order(&self, intent: &OrderIntent) -> Result<OrderId> {
    let client = reqwest::Client::new();  // ← new client per call = possible TLS handshake
    let resp = client.post(&self.order_url).json(intent).send().await?;
    // ...
}
```

The problematic Mutex counter patterns:
```rust
// rate_budget.rs ~line 76
pub fn check(&self) -> bool {
    let mut count = self.count.lock().unwrap();  // ← Mutex lock per event
    if *count < self.limit { *count += 1; true } else { false }
}

// throttle.rs ~line 69
pub fn allow_frame(&self) -> bool {
    let mut n = self.frames.lock().unwrap();  // ← Mutex lock per WS frame
    if *n < self.limit { *n += 1; true } else { false }
}
```

## Task

### Fix #9 — Warm HTTP/2 per-broker connection pool

**Problem:** Order submission may create a new `reqwest::Client` per call, or uses a client without HTTP/2 keep-alive configured. A new TCP+TLS handshake costs 100–300 ms — unacceptable for order submission where every millisecond matters.

**Solution:** Create one shared `reqwest::Client` per broker at platform startup with HTTP/2 keep-alive, connection pool, and TCP_NODELAY configured. Share it across all submission calls via `Arc<reqwest::Client>`.

**Implementation steps:**

1. In the `AlpacaExecutor` struct (or equivalent per-venue executor), change the client field:
   ```rust
   pub struct AlpacaExecutor {
       client: Arc<reqwest::Client>,   // shared, never re-created
       // ...
   }
   ```

2. In `AlpacaExecutor::new()` (or an equivalent constructor called once at startup), build the shared client:
   ```rust
   let client = Arc::new(
       reqwest::Client::builder()
           .http2_prior_knowledge()                              // H2 where Alpaca supports it
           .http2_keep_alive_interval(std::time::Duration::from_secs(30))
           .http2_keep_alive_while_idle(true)
           .tcp_nodelay(true)                                    // disable Nagle — critical for latency
           .pool_idle_timeout(None)                             // keep connections forever
           .pool_max_idle_per_host(4)                           // 4 warm connections per broker host
           .timeout(std::time::Duration::from_millis(5_000))    // 5 s hard timeout per request
           .build()
           .expect("failed to build reqwest client")
   );
   ```
   Note: `http2_prior_knowledge()` requires the broker to support H2 on the order submission endpoint. If Alpaca uses HTTPS with H1.1 only, use `.use_rustls_tls()` with H1.1 keep-alive instead. Check the broker's API documentation.

3. In `submit_order`, use `self.client.post(...)` — the shared client is already configured with a warm connection pool:
   ```rust
   pub async fn submit_order(&self, intent: &OrderIntent) -> Result<OrderId> {
       let resp = self.client
           .post(&self.order_url)
           .json(&order_body)
           .send()
           .await?;
       // ...
   }
   ```

4. Audit all venue adapters for the same pattern. Each should use a shared client, never `reqwest::Client::new()`:
   - `crates/execution/src/venues/kalshi.rs` — add `client: Arc<reqwest::Client>` field; configure at startup.
   - `crates/execution/src/venues/oanda.rs` — same.
   - `crates/execution/src/venues/tradier.rs` — same.
   - `crates/execution/src/venues/tradovate.rs` — same; note that Tradovate uses both REST and WS; confirm order submission endpoint.
   Add a CI grep to catch regressions: `grep -rn "reqwest::Client::new\(\)" crates/execution/` must return zero results after this fix.

5. Pre-serialize order-body templates per instrument at strategy-start time. For each (instrument, side) combination, pre-serialize the static portions of the order body (symbol, account ID, order type) as a base `serde_json::Value`. At submit time, only the price and quantity fields need to be patched:
   ```rust
   // At strategy-start (init time, not per-order):
   let template = OrderTemplate::new(symbol, account_id, "limit")?;
   // At submit time (hot path):
   let body_bytes = template.apply(price, qty)?;  // patches two fields only
   ```
   This reduces per-order serialization from a full `serde_json::to_vec` of the entire struct to two field patches.

6. For Kraken specifically: implement order entry over the Kraken WS v2 authenticated socket using the `add_order` message. The WS connection is already open for market data. This eliminates the HTTP round-trip entirely for Kraken orders. The `add_order` WS message is sent as a JSON frame on the authenticated private channel:
   ```json
   {"method": "add_order", "params": {"order_type": "limit", "side": "buy", "symbol": "BTC/USD", "limit_price": "50000", "order_qty": "0.001"}}
   ```
   Add `crates/execution/src/venues/kraken_ws.rs` to handle this.

### Fix #43 — AtomicU32 for RateBudget

**Problem:** `crates/demand-manager/src/rate_budget.rs` (around lines 76, 92, 100): `Mutex<u32>` is locked on every rate-limit check. The rate-budget check is called on every subscription or event — a lock contended by all collectors simultaneously.

**Solution:** Replace `Mutex<u32>` with `AtomicU32`. Use `fetch_add` with `Ordering::Relaxed` for the counter (the rate limit window resets periodically, so sequential consistency is not required — only the correct final count matters).

**Implementation steps:**

1. Change the counter field in the `RateBudget` struct:
   ```rust
   use std::sync::atomic::{AtomicU32, Ordering};
   // Before:
   count: Mutex<u32>,
   // After:
   count: AtomicU32,
   limit: u32,
   ```

2. Replace `check()` and `decrement()`:
   ```rust
   /// Returns true if the budget allows one more unit, and consumes it.
   /// Returns false if the limit is reached.
   pub fn check_and_consume(&self) -> bool {
       // Optimistic increment: add 1 and check if we exceeded the limit
       let prev = self.count.fetch_add(1, Ordering::Relaxed);
       if prev >= self.limit {
           // Exceeded — undo the increment
           self.count.fetch_sub(1, Ordering::Relaxed);
           false
       } else {
           true
       }
   }

   pub fn release(&self) {
       self.count.fetch_sub(1, Ordering::Relaxed);
   }

   /// Called by the periodic rate-limit reset timer.
   pub fn reset(&self) {
       self.count.store(0, Ordering::Relaxed);
   }
   ```

3. Remove all `self.count.lock().unwrap()` calls. The entire `Mutex` is gone.

4. Update callers: replace `rate_budget.check()` with `rate_budget.check_and_consume()`. Replace `rate_budget.decrement()` with `rate_budget.release()`.

### Fix #45 — AtomicU32 for WS throttle

**Problem:** `crates/ui-gateway/src/throttle.rs` (around lines 69, 81, 98, 104): `Mutex<u32>` locked on every outgoing WS frame to enforce per-connection frame rate limits. With multiple concurrent WebSocket connections each sending frames at high frequency, this Mutex becomes a bottleneck.

**Solution:** Same pattern as #43 — replace `Mutex<u32>` with `AtomicU32`.

**Implementation steps:**

1. Change the throttle struct:
   ```rust
   use std::sync::atomic::{AtomicU32, Ordering};
   // Before:
   frames: Mutex<u32>,
   // After:
   frames: AtomicU32,
   frame_limit: u32,
   ```

2. Replace `allow_frame()`:
   ```rust
   pub fn allow_frame(&self) -> bool {
       let prev = self.frames.fetch_add(1, Ordering::Relaxed);
       if prev >= self.frame_limit {
           self.frames.fetch_sub(1, Ordering::Relaxed);
           false
       } else {
           true
       }
   }

   pub fn reset_window(&self) {
       self.frames.store(0, Ordering::Relaxed);
   }
   ```

3. Remove all `self.frames.lock().unwrap()` calls.

4. If the throttle has multiple counters (e.g., per-connection and global), convert each `Mutex<u32>` to a separate `AtomicU32`.

**Acceptance test:**
- Write an integration test that creates an `AlpacaExecutor`, submits 10 sequential orders, and verifies (via `RUST_LOG=reqwest=debug` log capture) that zero TLS handshakes occur after the first order — the connection is reused.
- Verify `grep -rn "reqwest::Client::new\(\)" crates/execution/` returns zero results.
- Write a concurrent test for `RateBudget`: spawn 50 tasks each calling `check_and_consume()` concurrently. Verify the total successful acquisitions is exactly equal to the configured `limit`, with no over-counting.
- Write a concurrent test for `Throttle`: same pattern.

## Overall Acceptance Criteria
- [ ] Zero TLS handshakes on order submission under steady-state (first order after startup is acceptable)
- [ ] Shared `Arc<reqwest::Client>` used in all venue execution adapters (`grep "Client::new" crates/execution/` returns zero)
- [ ] `RateBudget` uses `AtomicU32` — no `Mutex` in `rate_budget.rs`
- [ ] `Throttle` uses `AtomicU32` — no `Mutex` in `throttle.rs`
- [ ] Concurrent rate-budget test: exactly `limit` successful acquisitions from 50 concurrent tasks
- [ ] Order submission submit-to-wire time < 1 ms measured with a tracing span (excludes network RTT)
- [ ] `cargo test` passes

## Files to Touch
- `crates/execution/src/alpaca.rs` — Arc<reqwest::Client> with H2 keep-alive; pre-serialized templates
- `crates/execution/src/venues/kalshi.rs` — shared Arc<reqwest::Client>
- `crates/execution/src/venues/oanda.rs` — shared Arc<reqwest::Client>
- `crates/execution/src/venues/tradier.rs` — shared Arc<reqwest::Client>
- `crates/execution/src/venues/tradovate.rs` — shared Arc<reqwest::Client>
- `crates/execution/src/venues/kraken_ws.rs` (new, optional) — WS order entry for Kraken
- `crates/demand-manager/src/rate_budget.rs` — AtomicU32; remove Mutex; update check/release API
- `crates/ui-gateway/src/throttle.rs` — AtomicU32; remove Mutex; update allow_frame/reset API
