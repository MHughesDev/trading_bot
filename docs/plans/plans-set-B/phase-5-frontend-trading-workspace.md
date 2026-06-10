---
Type: Formal
Status: Pending
Derived From: C-012, C-013, C-026, C-060, C-094, C-096, C-112, C-118, C-119, C-120, C-123, C-126, C-127, C-129
---

# Phase 5 — Frontend Trading Workspace

> **Self-contained execution doc.** You need only: this file, [`../../architecture.md`](../../architecture.md),
> the spec [`../../specs/COMP-003-ui-streaming-gateway.md`](../../specs/COMP-003-ui-streaming-gateway.md),
> and the existing `frontend/` SPA. The frontend is React + Vite with `src/pages/`, `src/panels/`
> (`ChartPanel`, `PositionsPanel`, `TradePanel`, and an existing `OrderBookPanel` to be **removed**),
> `src/components/`, `src/hooks/`, `src/store/` (Zustand: `auth`, `theme`, `watchlist`),
> `src/api/{rest,ws}.ts`, and a strategy builder. Read these before editing. Phase 2's NATS.ws subject
> mapping (`md.ohlcv.{venue}.{class}.{instrument}`) must exist for the live hook.

## Phase goal

After this phase the **Trading workspace and the global navigation shell match the end-state vision**:
an animated **glass-pill navigation** switches between the five top-level sections (Dashboard,
Trading, Automations, Strategy Creation, Settings) with proper routing; the **Trading** section is a
full-height **horizontal infinite scroll** of panels that are either **scanner** panels (tile-based
watch table with a strategy-populate dropdown) or **terminal** panels (1-min OHLCV chart with order/
TP/SL annotations, an asset-class-specific order ticket, positions, working orders, fills); there is
**no order book / DOM / tick list anywhere**; OHLCV streams live via a **NATS.ws hook**; each browser
window carries a **Paper/Live mode** toggle persisted in localStorage (default Paper); and a
`LayoutTemplateRegistry` defines reusable panel layouts.

## Prerequisites

- Phase 2 done: NATS.ws subjects published (`md.ohlcv.{venue}.{asset_class}.{instrument}`) and the
  NATS.ws port/authz available to the browser.
- Existing `frontend/` builds (`npm run build` in `frontend/`), even with pre-existing deferred lint
  errors.

## Invariants this phase must respect

- **No order book / DOM / depth / recent-trades tick list** anywhere in the UI (C-129/C-112). The
  existing `src/panels/OrderBookPanel.tsx` is **removed** and de-referenced. Minimum data baseline is
  1-minute OHLCV; everything shown is derived from bars (+ derived quotes/positions/fills).
- **No risk UI** (C-114): no risk controls in the order ticket or anywhere.
- **Paper/Live is per browser window**, applies to all panels in that window, persists in localStorage,
  and **defaults to Paper** (C-118/C-119/C-120/C-123).
- **`prefers-reduced-motion` honored** for all animations (C-013).

---

## Tasks

### P5-T01 — Five-section routing
- **Goal:** The app routes to five top-level sections at stable URLs.
- **Files:** `frontend/src/App.tsx`, a new `frontend/src/routes.tsx`, and section page shells
  `frontend/src/pages/{DashboardPage,TradingPage,AutomationsPage,StrategyCreationPage,SettingsPage}.tsx`
  (reuse/rename existing pages where they fit; `AssetPage`/`TransactionsPage` fold into Trading/
  Dashboard).
- **Context:** Per C-012/C-094. Routes: `/dashboard`, `/trading`, `/automations`, `/strategy`,
  `/settings`. Automations is a dedicated section at its own URL. Use the existing router setup
  (react-router). Create shells now; rich content lands in this phase (Trading) and Phase 6
  (Dashboard/Automations/Settings).
- **Acceptance:** navigating to each of the five URLs renders its section shell without error;
  `npm run build` succeeds; a route test (or manual check documented in the PR) confirms all five
  resolve.
- **Depends on:** none.

### P5-T02 — Glass-pill animated navigation
- **Goal:** A glass-pill nav button that animates between the five sections with the specified motion.
- **Files:** `frontend/src/components/layout/GlassPillNav.tsx` (new), styles colocated;
  mount in the app shell.
- **Context:** Per C-013. Use **Framer Motion** with **380 ms asymmetric easing** for the pill
  transition. Honor `prefers-reduced-motion` (skip/shorten animation when set). The pill highlights
  the active section and navigates on select. Glassmorphism styling (backdrop blur, translucent).
- **Acceptance:** the nav switches sections with the animated pill; toggling OS reduced-motion removes
  the animation (verify via the `prefers-reduced-motion` media query branch); the active section is
  visually indicated.
- **Depends on:** P5-T01.

### P5-T03 — Remove order-book / DOM surfaces
- **Goal:** Eliminate every order-book/DOM/tick-list surface from the UI.
- **Files:** delete `frontend/src/panels/OrderBookPanel.tsx`; remove all imports/usages of it; remove
  any depth/DOM/recent-trades components.
- **Context:** Per C-129/C-112. The minimum data baseline is 1-min OHLCV; order books are neither
  ingested nor shown. Replace any layout slot that referenced the order book with chart/positions
  content.
- **Acceptance:** a repo grep for `OrderBook`, `DOM`, `depth`, `recent-trades` in `frontend/src`
  returns no live component usage; `npm run build` succeeds.
- **Depends on:** none.

### P5-T04 — Paper/Live per-window mode store
- **Goal:** A persisted per-window Paper/Live mode that all panels in the window read.
- **Files:** `frontend/src/store/mode.ts` (new Zustand store), a `ModeBadge` component in
  `frontend/src/components/layout/ModeBadge.tsx`.
- **Context:** Per C-118/C-119/C-120/C-123. The mode (`'PAPER' | 'LIVE'`) is stored in `localStorage`,
  **defaults to `'PAPER'`**, and is shown as a badge/toggle top-right of the Trading window. Changing
  it applies to **all panels** in that window. Two browser windows can hold the same account but are
  independent windows (localStorage is per-origin; document the multi-window expectation — both
  windows of the same origin share localStorage, so the toggle reflects across them, which is the
  intended same-account behavior).
- **Acceptance:** toggling the badge updates the store, persists across reload (localStorage), and
  defaults to `PAPER` on first load; panels read the mode from the store.
- **Depends on:** P5-T01.

### P5-T05 — NATS.ws live OHLCV subscription hook
- **Goal:** A React hook that subscribes to live 1-min OHLCV over NATS.ws and yields a streaming bar
  series.
- **Files:** `frontend/src/hooks/useOhlcvStream.ts` (new); add `nats.ws` dependency;
  `frontend/src/api/nats.ts` (connection singleton).
- **Context:** Per C-060/C-112. Connect to the NATS.ws endpoint (Phase 2) and subscribe to
  `md.ohlcv.{venue}.{asset_class}.{instrument}`. The hook `useOhlcvStream(venue, assetClass,
  instrument)` returns the latest bars + a live-updating last bar, handling reconnect and subject
  authz token. This is the lowest-latency path — **no SSE bridge**. Acquiring the hook signals demand
  (the backend DemandRegistry ref-counts the lane); unmounting releases it.
- **Acceptance:** mounting a component with the hook subscribes and renders incoming bars; unmounting
  unsubscribes; a published bar (from a test publisher) appears in the hook's output. Document the
  manual verification in the PR if no automated NATS.ws harness exists.
- **Depends on:** Phase 2 NATS.ws subjects, P5-T01.

### P5-T06 — Horizontal infinite-scroll Trading workspace
- **Goal:** The Trading section is a full-height horizontal infinite scroll hosting panels.
- **Files:** `frontend/src/pages/TradingPage.tsx`, `frontend/src/components/trading/WorkspaceScroll.tsx`
  (new), `frontend/src/components/trading/Panel.tsx` (new panel frame).
- **Context:** Per C-026. Panels are laid out left-to-right in a horizontal infinite scroll that fills
  the viewport height. Each panel is either a scanner or a terminal (P5-T07/P5-T08). Multi-display:
  two browser windows can scroll independently over the same conceptual workspace. The Paper/Live
  badge (P5-T04) sits top-right of the window and governs all panels.
- **Acceptance:** the Trading page renders a horizontal scroll that adds panels on demand and fills the
  viewport height; scrolling is smooth; the mode badge is visible top-right.
- **Depends on:** P5-T04.

### P5-T07 — Scanner panel (tile watch table + populate control)
- **Goal:** A scanner panel showing a tile-based watch table populated by a chosen discovery strategy.
- **Files:** `frontend/src/components/trading/ScannerPanel.tsx` (new),
  `frontend/src/components/trading/WatchTile.tsx` (new).
- **Context:** Per C-026/C-126. The watch table is **tile-based**: rounded tiles, **no visible column
  dividers**. A **strategy dropdown** at the panel top selects a discovery strategy to populate the
  scanner (only compatible discovery strategies appear — apply-list filtering from Phase 3). Rows
  support **hover-to-remove**. Each tile shows derived fields (last, change, the ranked metric) from
  the OHLCV stream — no order book.
- **Acceptance:** selecting a discovery strategy populates the scanner with instruments; hovering a
  tile reveals a remove control that removes it; the table renders as rounded tiles with no column
  dividers.
- **Depends on:** P5-T05, P5-T06.

### P5-T08 — Terminal panel (chart + order ticket + positions)
- **Goal:** A terminal panel with the 1-min chart, an asset-class-specific order ticket, positions,
  working orders, fills, and account context — no order book.
- **Files:** `frontend/src/components/trading/TerminalPanel.tsx` (new); reuse/refactor
  `src/panels/ChartPanel.tsx`, `src/panels/TradePanel.tsx`, `src/panels/PositionsPanel.tsx`.
- **Context:** Per C-026/C-129. The terminal contains: a **1-min OHLCV chart** (P5-T09 adds
  annotations), an **order ticket** with full asset-class-specific controls (buy/sell, sizing, order
  types, TIF, **bracket orders where supported** — e.g. CLOB classes; prediction markets are YES/NO),
  a **positions summary**, **working orders**, **fills**, and **account context**. It reads the
  window's Paper/Live mode. **No order book, no DOM, no tick list.** The ticket's available controls
  derive from the instrument's asset class.
- **Acceptance:** the terminal renders chart + ticket + positions + working orders + fills; the ticket
  shows asset-class-appropriate controls (e.g. bracket fields for crypto, YES/NO for a prediction
  market) and no risk fields; submitting routes to the mode-appropriate backend path.
- **Depends on:** P5-T05, P5-T06, P5-T04.

### P5-T09 — Chart trade annotations
- **Goal:** Draw order markers and TP/SL lines on the candle chart.
- **Files:** `frontend/src/panels/ChartPanel.tsx` (extend),
  `frontend/src/components/charts/Annotations.tsx` (new).
- **Context:** Per C-127. Overlay on the 1-min candles: **order entry markers**, **take-profit** and
  **stop-loss** horizontal lines (for bracket/working orders), and fill markers — all drawn on the
  bars. Annotations update live from working-order/fill state. No depth overlay.
- **Acceptance:** placing/showing a working bracket order draws its TP and SL lines and entry marker on
  the chart at the right price levels; a fill draws a fill marker; removing the order removes its
  annotations.
- **Depends on:** P5-T08.

### P5-T10 — LayoutTemplateRegistry
- **Goal:** A registry of reusable panel-layout templates for the workspace.
- **Files:** `frontend/src/config/layoutTemplates.ts` (new).
- **Context:** Per C-096. Define a typed `LayoutTemplateRegistry` mapping template ids to ordered panel
  specs (scanner/terminal + instrument/strategy bindings). The Trading workspace can instantiate a
  template to seed its panel list. Templates are data, not components.
- **Acceptance:** the registry exports at least a default template (e.g. one scanner + one terminal);
  `TradingPage` can instantiate it to render the initial workspace; a unit test (or type-level check)
  confirms each template entry references a valid panel kind.
- **Depends on:** P5-T06, P5-T07, P5-T08.

---

## Phase exit criteria
- [ ] Five sections route at `/dashboard`, `/trading`, `/automations`, `/strategy`, `/settings`.
- [ ] The glass-pill nav animates between sections (380 ms asymmetric easing) and honors
      `prefers-reduced-motion`.
- [ ] No order-book/DOM/tick-list surface remains anywhere (`OrderBookPanel` deleted; grep clean).
- [ ] Paper/Live mode is per-window, persisted in localStorage, defaults to Paper, and governs all
      panels.
- [ ] The NATS.ws hook streams live 1-min OHLCV and signals/releases demand.
- [ ] Trading is a full-height horizontal infinite scroll hosting scanner and terminal panels.
- [ ] Scanner panels are tile-based (no column dividers), populated by a compatible discovery-strategy
      dropdown, with hover-to-remove.
- [ ] Terminal panels show chart + asset-class-specific order ticket (bracket where supported, no risk
      fields) + positions + working orders + fills; no order book.
- [ ] Chart annotations draw order markers and TP/SL lines on the candles.
- [ ] `LayoutTemplateRegistry` exists and seeds the workspace.
- [ ] `frontend/` builds (`npm run build`).
