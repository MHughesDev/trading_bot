# 06 — UI & Streaming

## Subscription-based UI

A React panel does **not** own a data engine. It owns a **subscription**. When a panel opens it
subscribes; when it closes, the UI gateway removes the subscription and the Demand Manager
decrements demand. The underlying engine stops only if no other consumer needs it.

Example panels:

```json
{ "panel_id": "chart_1",
  "subscribe": [
    { "lane": "market.bars.1m", "instrument": "BTC-USDT" },
    { "lane": "features.technical", "instrument": "BTC-USDT",
      "features": ["ema_7", "ema_21"] }
  ] }
```

```json
{ "panel_id": "orderbook_1",
  "subscribe": [
    { "lane": "ui.orderbook.snapshot", "instrument": "AAPL",
      "depth": 20, "max_fps": 20 }
  ] }
```

## The UI gateway is not a dumb proxy

It handles: authentication, authorization, subscription register/remove, lane + instrument
filtering, **panel-level rate limits**, batching, compression, **snapshot-on-connect**,
reconnect recovery, heartbeat/ping-pong, backpressure detection, and UI-safe payload shaping.

### Lossy by design

The UI gateway **intentionally drops intermediate frames**. If 500 order-book updates arrive in a
second, it may send 20 snapshots per second. That is correct for human visualization. It is
**not** acceptable for strategy execution or storage — so the UI gateway is a **separate consumer
view**, never the canonical stream.

```
Raw L2 deltas
   → Order-book state builder
   → Throttled top-N-level snapshot every 50–150 ms
   → WebSocket → React order-book panel
```

Never `Raw L2 deltas → React directly`.

## Transport

- **REST** for control/config/history (see below).
- **WebSocket** for bidirectional live panel streams.
- **SSE** acceptable for simple one-way feeds.

Payload format: JSON for v1 (control plane and UI). Consider MessagePack for the UI stream later
if bandwidth becomes a concern; internal durable streams may move to Protobuf. None of this is
irreversible.

## REST is the control plane, not the data plane

Good for commands/config/history:

```
POST   /api/strategies/{id}/start
POST   /api/strategies/{id}/stop
POST   /api/strategies                 (create from JSON definition)
GET    /api/strategies/{id}/config
POST   /api/backtests
GET    /api/backtests/{id}
POST   /api/orders                     (manual order → risk gate)
DELETE /api/orders/{id}
GET    /api/assets
GET    /api/instruments/{id}
GET    /api/streams/available
POST   /api/ui/subscriptions
POST   /api/trading/kill               (kill switch)
```

Bad over REST (use WebSocket/SSE instead): live order book, live trades, live bars, live
indicators, strategy logs, order-fill updates.

Live endpoint:

```
GET /ws/live
```

## Private vs public data on the wire

Never leak private streams through shared UI subscriptions.

- **Public-ish (shareable):** trades, bars, order book, indicators/features.
- **Private (per user):** orders, fills, positions, balances, strategy configs/decisions, risk
  limits, broker credentials.

The gateway enforces authorization per subscription; a panel can only subscribe to private lanes
scoped to its authenticated user.
