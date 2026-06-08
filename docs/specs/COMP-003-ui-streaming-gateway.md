# COMP-003: UI Streaming Gateway

**Status:** Implemented
**Version:** 1.0
**ADR(s):** ADR-0001, ADR-0011
**Success Conditions:** SC-3

## 1. Purpose

Defines the UI streaming gateway: the component that translates internal bus events into WebSocket streams for the React frontend. The gateway is intentionally lossy, never canonical. Its dropped frames are correct for human visualization and would be catastrophic for strategy execution. The strategy runtime never reads from the UI feed; this separation is a hard invariant.

## 2. Scope & Non-Goals

**In scope:**
- WebSocket streaming to React, including panel subscription lifecycle.
- Lossy-by-design policy: intermediate frames may be dropped; the UI feed is not the canonical stream.
- Panel-level throttling policies (e.g. 20fps order book cap).
- Snapshot-on-connect behavior.
- Demand Manager integration: UI panel subscriptions declare demand; the gateway manages registration and removal.
- Authentication and per-subscription authorization.
- Public vs private lane separation (trades/bars = shareable; orders/positions = per-user).
- REST control plane endpoints (as distinct from the live WebSocket data plane).
- User onboarding flow (account creation, venue credential setup).
- Dashboard and instrument detail view data requirements.

**Not in scope (deliberate):**
- Strategy runtime data subscriptions — strategies consume canonical bus events directly, never via this gateway (FEAT-001).
- Order submission via WebSocket — orders use the REST control plane.
- Storage of UI subscription state — ephemeral; lost on gateway restart is acceptable.
- The visual builder node graph — frontend React concern.
- WebRTC or other transports — WebSocket is sufficient for v1.

## 3. Design

### 3.1 The Gateway Is Not a Dumb Proxy

The UI gateway is responsible for:
- Authentication and authorization per subscription.
- Subscription register/remove — clients declare panels; the gateway adds/removes them from the Demand Manager.
- Lane and instrument filtering — a panel only receives events it subscribed to.
- Panel-level rate limiting / throttling.
- Batching and compression.
- Snapshot-on-connect — new subscribers receive the current state before the live stream starts.
- Reconnect recovery — client-side reconnects receive a fresh snapshot.
- Heartbeat / ping-pong.
- Backpressure detection — slow clients are managed rather than allowed to grow unbounded queues.
- UI-safe payload shaping — strip private fields, format for frontend consumption.

### 3.2 Lossy by Design

The UI gateway **intentionally drops intermediate frames**. This is a feature, not a deficiency:

```
Raw L2 deltas (many per second)
   → Order-book state builder
   → Throttled top-N-level snapshot every 50–150ms
   → WebSocket → React order-book panel
```

Not:
```
Raw L2 deltas → React directly   ← WRONG: too fast, no state reconstruction
```

If 500 order-book updates arrive in one second, the gateway may send 20 snapshots per second (20fps). That is correct for human visualization. It is not acceptable for strategy execution or storage — which is why the UI gateway is a **separate consumer view**, never the canonical stream. The canonical bus events flow directly to the strategy runtime and storage writers through their own consumer paths.

**The strategy runtime NEVER reads the UI feed.** This is a hard invariant enforced by design: strategy runtime instances subscribe to the bus directly via their declared demand, not to any gateway-shaped view.

### 3.3 Panel Subscriptions

A React panel subscribes by sending a JSON subscription request over the WebSocket connection:

```json
{
  "panel_id": "chart_1",
  "subscribe": [
    { "lane": "market.bars.1m", "instrument": "BTC-USDT" },
    { "lane": "features.technical", "instrument": "BTC-USDT",
      "features": ["ema_7", "ema_21"] }
  ]
}
```

```json
{
  "panel_id": "orderbook_1",
  "subscribe": [
    { "lane": "ui.orderbook.snapshot", "instrument": "AAPL",
      "depth": 20, "max_fps": 20 }
  ]
}
```

On subscription registration, the gateway:
1. Validates authentication and authorization for the requested lanes.
2. Registers the panel's demand with the Demand Manager.
3. Sends a snapshot of current state for each subscribed lane (snapshot-on-connect).
4. Begins streaming new events as they arrive, subject to throttling.

On panel close / WebSocket disconnect:
1. Remove the panel's subscriptions.
2. Notify the Demand Manager to decrement demand for each subscribed lane.
3. If no other consumer needs that lane, the Demand Manager may stop or deprioritize the pipeline.

### 3.4 Throttling Policies (v1)

| Lane | Default max fps | Behavior |
|------|----------------|----------|
| `market.bars.1m` | 1 fps (bars arrive at most 1/min) | Pass-through; no throttling needed |
| `features.technical` | 1 fps | Pass-through with bar events |
| `ui.orderbook.snapshot` | 20 fps (configurable per panel) | Reconstruct L2 state, emit top-N snapshots |
| `market.trades` | 10 fps | Batch recent trades; emit as array |
| `orders.*` / `positions.*` | 5 fps | Private; authorized per user only |

Throttled lanes are shaped via the order-book state builder or a sliding-window batcher. The raw underlying events are not slowed — only the UI view is rate-limited.

### 3.5 Public vs Private Lanes

The gateway enforces authorization per subscription:

- **Public-ish (shareable across authenticated users):** `market.trades`, `market.bars.*`, `market.orderbook.*`, `features.technical`.
- **Private (per authenticated user only):** `orders.*`, `positions.*`, `balances.*`, strategy configs, strategy decisions, risk limits.

A panel may only subscribe to private lanes scoped to its authenticated user. The gateway rejects cross-user private lane subscriptions with a structured authorization error.

Credentials (broker API keys) are never sent through any WebSocket or REST response body.

### 3.6 Demand Manager Integration

Panel subscriptions translate to Demand Manager entries:

```json
{
  "consumer_id": "ui_panel_chart_1_user42",
  "consumer_type": "ui_panel",
  "needs": [
    { "lane": "market.bars.1m", "instrument": "BTC-USDT" },
    { "lane": "features.technical", "instrument": "BTC-USDT" }
  ]
}
```

If `market.bars.1m` for `BTC-USDT` is already running (because a strategy also needs it), the Demand Manager does not start a duplicate pipeline. The UI panel receives events from the same pipeline the strategy consumes, shaped by the gateway into the UI-safe lossy view.

### 3.7 Transport

- **REST** — control plane: strategy management, order submission, history, auth, account setup.
- **WebSocket** (`GET /ws/live`) — bidirectional live panel subscriptions.
- **SSE** — acceptable for simple one-way feeds as a fallback.

Payload format: JSON for v1. MessagePack for the UI stream is a future optimization if bandwidth becomes a concern.

### 3.8 User Onboarding Flow

On first visit, the signup flow collects:
1. Username and password.
2. Which asset classes the user wants to trade (multi-select: Stocks & ETFs via Alpaca, Crypto via Coinbase, etc.).
3. API credentials per selected venue (Alpaca Key+Secret, Coinbase Advanced Trade Key+Passphrase). Skip is always allowed.

Until credentials are provided for a venue, the gateway:
- Shows the venue's instruments in read-only browse mode.
- Prevents strategy initialization and order submission on that venue.
- Shows a "connect account" prompt inline wherever credentials would be required.

### 3.9 Dashboard Data Shape

The dashboard REST response groups all activity across connected venues:
- Total P&L (all venues, common accounting currency).
- P&L breakdown by `AssetClass`.
- Win rate overall and per asset class.
- Active strategies count and list (grouped by asset class).
- Open positions summary (grouped by venue).
- Recent fills / activity feed.

There is no tab-switching to a separate UI per asset type. All asset classes live in the same dashboard with breakout rows. As asset classes are added, each gets a new row — a user trading only equities sees one row; a user active across five asset classes sees five.

## 4. Interfaces

**WebSocket endpoint:**
```
GET /ws/live
```
Accepts subscription JSON messages; emits event JSON messages.

**REST control plane endpoints:**
```
POST   /api/auth/signup
POST   /api/auth/login
POST   /api/auth/logout
POST   /api/ui/subscriptions           — panel subscribe/unsubscribe
GET    /api/streams/available          — list available lanes + instruments
GET    /api/assets                     — instruments grouped by asset class
GET    /api/instruments/{id}           — single instrument detail
POST   /api/strategies                 — create from JSON definition
GET    /api/strategies/{id}/config
POST   /api/strategies/{id}/start
POST   /api/strategies/{id}/stop
POST   /api/backtests
GET    /api/backtests/{id}
POST   /api/orders                     — manual order → risk gate
DELETE /api/orders/{id}
POST   /api/trading/kill               — kill switch
GET    /api/account/venues             — connection status per venue
POST   /api/account/venues/{venue_id}  — save/update API credentials
DELETE /api/account/venues/{venue_id}  — disconnect venue
GET    /api/dashboard/summary
GET    /api/dashboard/breakdown
```

**Subscription message format (client → gateway):**
```json
{ "panel_id": "<string>", "subscribe": [ { "lane": "...", "instrument": "...", ...options } ] }
{ "panel_id": "<string>", "unsubscribe": true }
```

**Event message format (gateway → client):** lane-specific JSON payloads shaped for UI consumption.

## 5. Dependencies

- DATA-001 — `EventEnvelope<T>` payloads received from the bus and shaped for the UI.
- DATA-002 — `Instrument` metadata for display (trading hours, asset class, venue).
- FEAT-001 — Demand Manager that the gateway registers/deregisters panel demand with.
- COMP-002 — Risk gate that manual orders pass through (REST path, not WebSocket).
- COMP-004 — Historical data queries for snapshot-on-connect and dashboard history.
- Axum — HTTP and WebSocket server framework.

## 6. Acceptance Criteria

- [x] AC-1: A strategy runtime instance does not subscribe to any gateway-shaped UI lane — it consumes canonical bus events directly — Verified by: `ui-gateway` unit tests, 2026-06-08
- [x] AC-2: A panel subscribing to `ui.orderbook.snapshot` with `max_fps: 20` receives no more than 20 snapshots per second regardless of how many raw L2 deltas arrive — Verified by: `ui-gateway` unit tests, 2026-06-08
- [x] AC-3: On panel open, the client receives a current-state snapshot before any live stream events — Verified by: `ui-gateway` unit tests, 2026-06-08
- [x] AC-4: When a panel closes and it was the last consumer of a lane, the Demand Manager is notified and the pipeline is stopped or deprioritized — Verified by: `ui-gateway` unit tests, 2026-06-08
- [x] AC-5: A WebSocket subscription to a private lane (`orders.*`, `positions.*`) for user A cannot return data belonging to user B — Verified by: `ui-gateway` unit tests, 2026-06-08
- [x] AC-6: A new panel subscribing to `market.bars.1m` for `BTC-USDT` when a strategy already needs that lane does not start a duplicate pipeline — Verified by: `ui-gateway` unit tests, 2026-06-08
- [x] AC-7: Broker API credentials are not present in any WebSocket message or REST response body — Verified by: `ui-gateway` unit tests, 2026-06-08

## 7. Open Questions

None at this revision. MessagePack encoding for the UI stream is a post-v1 performance optimization.
