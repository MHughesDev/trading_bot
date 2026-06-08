# Phase 3 — See it (UI streaming gateway + React panels + demand manager)

> **Self-contained execution doc.** You need only: this file, [`../file-structure.md`](../file-structure.md),
> and the specs — especially [`06-ui-and-streaming.md`](../spec/06-ui-and-streaming.md) and the
> Demand Manager section of [`01-architecture.md`](../spec/01-architecture.md).

## Phase goal

After this phase, a human can **see the system**: the UI streaming gateway serves throttled,
intentionally-lossy, frontend-shaped live views over WebSocket; the React SPA shows a live chart
panel, a live order-book panel, and a manual-trade panel that submits through the Phase 2 risk gate;
and the Demand Manager starts/stops the underlying pipelines based on which panels (and later
strategies) are subscribed.

## Prerequisites

- Phase 1 (bus, bars on the bus, REST API + auth, the React SPA served) and Phase 2 (manual orders
  through the risk gate) complete.
- `frontend/` already exists (Vite+React). This phase re-points it at the live WS contract and adds
  the panels.

## Invariants this phase must respect

- **The UI feed is lossy and separate.** The UI gateway is its own consumer view; it may drop
  intermediate frames (e.g. 20 fps order book). The **strategy runtime must never consume the UI
  feed** — that is enforced structurally by keeping the gateway a distinct consumer.
- **Private vs public on the wire.** Public-ish lanes (trades, bars, order book, features) are
  shareable; private lanes (orders, fills, positions, balances, configs) are per-user scoped. The
  gateway authorizes **per subscription**.
- **Never `Raw L2 deltas → React directly`.** Order-book deltas pass through a state builder, then a
  throttled top-N snapshot, then to the panel.

---

## Tasks

### P3-T01 — Demand Manager
- **Goal:** Aggregate demand for `(lane, instrument)` across consumers and start/keepalive/downshift/
  stop the underlying pipelines.
- **Files:** `crates/demand-manager/src/{lib,registry,lifecycle}.rs`.
- **Context:** Per [`../spec/01-architecture.md`](../spec/01-architecture.md) §Demand Manager:
  consumers (UI panels now, strategies in Phase 4) **declare demand**; the manager keeps a pipeline
  active if ≥1 consumer needs its lane, else stops/pauses/downshifts it. This prevents every panel/
  strategy from spinning up duplicate streams.
- **Acceptance:** unit tests: two consumers needing the same lane keep one pipeline; when both
  unsubscribe the pipeline stops; partial unsubscribe keeps it alive.
- **Depends on:** Phase 1 (bus).

### P3-T02 — UI gateway core (subscriptions + authz + shaping)
- **Goal:** The gateway that manages panel subscriptions and shapes/authorizes payloads.
- **Files:** `crates/ui-gateway/src/{lib,subscriptions,shaping}.rs`.
- **Context:** Per [`../spec/06-ui-and-streaming.md`](../spec/06-ui-and-streaming.md): register/remove
  subscriptions, lane+instrument filtering, **per-subscription authorization** (a panel may only
  subscribe to private lanes scoped to its authenticated user), and UI-safe payload shaping (public
  vs private enforcement). On subscribe, decrement/increment Demand Manager demand.
- **Acceptance:** unit tests: a panel cannot subscribe to another user's private lane; subscribe/
  remove adjusts demand; public lanes are shareable.
- **Depends on:** P3-T01, Phase 1 (auth).

### P3-T03 — UI gateway throttling + snapshots + transport
- **Goal:** The lossy-by-design machinery and the WS transport.
- **Files:** `crates/ui-gateway/src/{throttle,snapshot,transport}.rs`.
- **Context:** Per [`../spec/06-ui-and-streaming.md`](../spec/06-ui-and-streaming.md): per-panel rate
  limits; order-book path = raw L2 deltas → state builder (reuse `builders::orderbook`) → throttled
  top-N snapshot every 50–150 ms → WS; **snapshot-on-connect** + reconnect recovery; heartbeat/
  ping-pong; batching; compression; backpressure detection. The gateway intentionally drops
  intermediate frames.
- **Acceptance:** integration test: a burst of 500 order-book deltas/sec yields a bounded snapshot
  rate (≈20 fps), not 500 frames; a reconnecting client receives a fresh snapshot first.
- **Depends on:** P3-T02, Phase 1 (`builders::orderbook` — implement here if not yet present).

### P3-T04 — WS endpoint wiring
- **Goal:** Expose `GET /ws/live` and `POST /api/ui/subscriptions`, bridging WS connections to the
  gateway.
- **Files:** `crates/api/src/ws/{mod,live}.rs`, `crates/api/src/routes/streams.rs` (extend),
  wiring in `apps/platform/src/main.rs`.
- **Context:** `GET /ws/live` upgrades and bridges the socket to ui-gateway subscriptions;
  `POST /api/ui/subscriptions` registers panel subscriptions (per
  [`../spec/06-ui-and-streaming.md`](../spec/06-ui-and-streaming.md) REST list).
- **Acceptance:** a WS client subscribes to `market.bars.1m` for an instrument and receives live
  bars; closing the socket removes the subscription and decrements demand.
- **Depends on:** P3-T03.

### P3-T05 — Frontend API/WS clients
- **Goal:** Re-point the React SPA at the Rust REST + WS contracts.
- **Files:** `frontend/src/api/rest.ts`, `frontend/src/api/ws.ts`, `frontend/src/lib/types.ts`,
  `frontend/src/lib/format.ts`, `frontend/src/state/`.
- **Context:** `rest.ts` typed client for `/api/*`; `ws.ts` subscribe/remove panel client for
  `/ws/live` with reconnect + snapshot handling; `types.ts` mirrors `domain` payloads + lane names
  (keep in sync); `format.ts` does **decimal-safe** display formatting (no float math on money).
- **Acceptance:** the SPA connects, authenticates, lists assets, and opens a live subscription.
- **Depends on:** P3-T04.

### P3-T06 — Live panels
- **Goal:** The chart, order-book, trade, and positions panels.
- **Files:** `frontend/src/panels/{ChartPanel,OrderBookPanel,TradePanel,PositionsPanel}.tsx`,
  shared bits in `frontend/src/components/`.
- **Context:** Per [`../spec/06-ui-and-streaming.md`](../spec/06-ui-and-streaming.md) example panels:
  `ChartPanel` subscribes to `market.bars.1m` + `features.technical` (features land in Phase 4 —
  render gracefully without them now); `OrderBookPanel` subscribes to `ui.orderbook.snapshot`
  (depth N, max_fps 20); `TradePanel` submits manual orders via `POST /api/orders` (Phase 2 gate) and
  shows rejections; `PositionsPanel` shows the user's private positions/balances.
- **Acceptance:** chart and order-book panels stream live; the trade panel places an order that flows
  through the risk gate and the position panel updates; closing a panel stops its subscription.
- **Depends on:** P3-T05.

### P3-T07 — UI streaming integration check
- **Goal:** Verify the live path visually + programmatically.
- **Files:** extend `tests/` with a WS-throttle assertion if not covered by P3-T03; otherwise verify
  via the running app.
- **Context:** Confirm lossy throttling, snapshot-on-connect, per-user privacy, and demand
  start/stop together. Use the preview/verify workflow for the visual confirmation.
- **Acceptance:** documented evidence (screenshot or log) that live panels work and the UI feed is
  bounded/lossy and per-user scoped.
- **Depends on:** P3-T06.

---

## Phase exit criteria

- [ ] `crates/{demand-manager,ui-gateway}` implemented; `crates/api` exposes `/ws/live` and
      `/api/ui/subscriptions`.
- [ ] The UI feed is lossy/throttled (≈20 fps order book), snapshot-on-connect works, and per-
      subscription authorization scopes private lanes per user.
- [ ] The Demand Manager starts pipelines on first demand and stops them on last unsubscribe.
- [ ] The React SPA shows live chart + order-book panels and a manual-trade panel that routes through
      the Phase 2 risk gate; positions update live.
- [ ] The strategy runtime path is untouched by the UI feed (gateway is a separate consumer).
