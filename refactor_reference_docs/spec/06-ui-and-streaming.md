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

---

## User onboarding and account setup

The system is hosted locally for a small trusted group. On first visit, a user creates an account.
The signup flow should immediately ask: **what do you want to trade?**

```
Step 1 — Create account (username, password)
Step 2 — What do you want to trade? (multi-select)
          ☐ Stocks & ETFs (Alpaca)
          ☐ Crypto Spot — Coinbase
          ☐ Crypto Spot — Kraken
          ☐ (more exchanges added later)
          [ Skip for now ]
Step 3 — For each selected asset class, provide API credentials
          e.g. Alpaca: API Key + Secret
               Coinbase Advanced Trade: API Key + Passphrase
          Each set saved encrypted per user, per venue.
          [ Skip this venue ]
```

**Skip is always allowed.** A user who skips credential entry during signup lands on the
dashboard. From account settings they can return at any time to add or update API credentials for
any venue. The list of supported venues and which ones have credentials configured is visible in
account settings.

Until credentials are provided for a venue, the system:
- Shows the asset class in the instrument list (read-only, for browsing).
- Prevents strategy initialization and order submission on that venue.
- Shows a "connect account" prompt inline wherever an action would require credentials.

## Dashboard design

The dashboard is the landing page after login. It shows a **unified view of the user's trading
activity across all connected venues**, not a single-venue view.

### Performance breakdown (not a single aggregate)

Because users trade across multiple asset classes on multiple venues, performance metrics must be
**broken down by asset type**. Do not flatten everything into one win rate number — that conflates
radically different trade durations and market behaviors.

The dashboard should display:
- **Total P&L** (all venues combined, converted to a common accounting currency).
- **P&L breakdown by asset class** (e.g. Equities +$X, Crypto CEX −$Y).
- **Win rate overall** and **win rate per asset class**.
- **Active strategies** count and list (grouped by asset class).
- **Open positions** summary (grouped by venue).
- **Recent fills / activity feed**.

There is **no tab-switching to a separate UI per asset type**. All asset classes live in the same
dashboard with breakout sections. This avoids context-switching overhead and lets a user see their
full book at a glance.

### Future-facing layout note

As more asset classes are added (DEX, options, NFTs, etc.), each gets a breakout row on the
dashboard. The layout scales horizontally — a user who only trades equities sees one row; a user
active across five asset classes sees five.

## Asset detail / instrument view

When a user clicks on an instrument (e.g. BTC-USDT on Coinbase), they navigate to the
**instrument detail view** for that asset. This view contains:

- Live price chart (OHLCV bars, 1m for MVP — see [03-data-engineering.md](./03-data-engineering.md) §11).
- Order book panel (disabled/hidden for MVP; placeholder indicating future availability).
- Manual trade panel (buy/sell → REST → risk gate).
- **Strategy panel** — shows whether a strategy is currently initialized on this instrument for
  this user. If no strategy is running:
  - Shows an "Initialize" button.
  - User selects a strategy definition from their library (or opens the builder to create one).
  - User clicks "Initialize" → a strategy instance is created and started in the runtime bound to
    this instrument.
  - The UI updates to show the strategy as active, with its signals and recent decisions.
- If a strategy IS running: shows it with a "Stop" button and a decision log.

**One strategy per instrument per user** at a time (MVP constraint). The underlying runtime is
multi-instance capable; the UI simply enforces this one-at-a-time UX at initialization time.

## Multi-venue account management

Users have separate API credentials per venue. Account settings surfaces a venue list:

```
Connected Accounts
  ✓ Alpaca (paper)           [Edit] [Disconnect]
  ✓ Coinbase Advanced Trade  [Edit] [Disconnect]
  ✗ Kraken                   [Connect]
```

Switching between paper/live mode per venue is surfaced here (once live adapters are built).
Credentials are stored encrypted at rest; never exposed through any API response.

## REST additions (account + credential management)

```
POST   /api/auth/signup                       (username + password; returns session)
POST   /api/auth/login
POST   /api/auth/logout
GET    /api/account/venues                    (list venues + connection status for this user)
POST   /api/account/venues/{venue_id}         (save/update API credentials for a venue)
DELETE /api/account/venues/{venue_id}         (disconnect / remove credentials)
GET    /api/dashboard/summary                 (P&L + win rate + active strategies, all venues)
GET    /api/dashboard/breakdown               (same, broken down by asset_class)
```
