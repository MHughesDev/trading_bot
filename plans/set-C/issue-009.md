# Issue #009 — Cold REST order egress

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | D |
| Pattern | Network |
| Quick Win | No |
| Latency Impact | TCP+TLS handshake (~100–300 ms) possible on order submission |
| Location | `crates/execution/src/alpaca.rs:123` |

## Problem
Broker adapters submit orders via per-call `reqwest` with no guarantee of a warm connection. The most latency-sensitive message in the system — the order — can pay a full TCP+TLS handshake of 100–300 ms if the connection pool has gone idle. This is unacceptable for any execution adapter.

## Root Cause
`reqwest::Client` is either created per-request or the shared client is not configured to keep connections alive. The default idle timeout causes connections to close after a period of inactivity, meaning the next order after a quiet period pays the full handshake cost.

## Implementation Plan
### Step 1 — Create a shared reqwest Client per broker at startup
In each broker adapter (`alpaca.rs`, `kalshi.rs`, `oanda.rs`), build one `reqwest::Client` at adapter initialization:
```rust
let client = reqwest::Client::builder()
    .http2_prior_knowledge()
    .http2_keep_alive_interval(Duration::from_secs(30))
    .http2_keep_alive_while_idle(true)
    .tcp_nodelay(true)
    .pool_idle_timeout(None)
    .build()?;
```
Store this in an `Arc<reqwest::Client>` shared across all requests from that adapter.

### Step 2 — Pre-serialize order-body templates per instrument
For each commonly-traded instrument, pre-serialize the order request body template at startup. At order time, substitute only the dynamic fields (quantity, price, side) using string replacement or a pre-built template. Avoid re-constructing the JSON body from scratch per order.

### Step 3 — Implement Kraken order entry over WS v2 authenticated socket
Kraken v2 WS supports `add_order` messages over the authenticated WebSocket connection. Implement order submission via the already-open WS connection using the `add_order` command. This eliminates the HTTP round trip entirely for Kraken orders.

### Step 4 — Send a heartbeat on idle
If the broker's API does not keep the connection warm, send a lightweight authenticated ping every 20 seconds to prevent TCP keepalive issues and ensure the connection is ready for the next order.

## Acceptance Criteria
- [ ] Zero TLS handshakes on order submission under steady state (measured with Wireshark or tcpdump over 5-minute idle + order)
- [ ] Submit-to-wire latency < 1 ms for Alpaca and Kalshi (excluding network RTT)
- [ ] Kraken orders use WS v2 add_order instead of HTTP
- [ ] Shared reqwest::Client persists for lifetime of adapter process

## Files to Change
- `crates/execution/src/alpaca.rs` — shared client with HTTP/2 keep-alive; pre-serialized templates
- `crates/execution/src/venues/kalshi.rs` — same shared client pattern
- `crates/execution/src/venues/oanda.rs` — same shared client pattern
