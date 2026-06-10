---
Type: Formal
Status: Pending
Derived From: C-015, C-053, C-079, C-080, C-081, C-084, C-093, C-095, C-098
---

# Phase 6 — Frontend Dashboard & Automations

> **Self-contained execution doc.** You need only: this file, [`../../architecture.md`](../../architecture.md),
> the spec [`../../specs/COMP-003-ui-streaming-gateway.md`](../../specs/COMP-003-ui-streaming-gateway.md),
> and the existing `frontend/` SPA. Phase 5 built the five-section routing, glass-pill nav, Paper/Live
> mode store, and the section shells (`DashboardPage`, `AutomationsPage`, `SettingsPage`). Phase 4
> built the USD rollup endpoint (`GET /api/dashboard/rollup`) and per-venue `AccountSource`. Phase 3
> built the `AutomationPlan` model + pipeline runtime + apply-list filtering. Read those shells and
> `src/api/rest.ts` before editing.

## Phase goal

After this phase the **Dashboard, Automations, and Settings sections are fully built**: the Dashboard
is a **three-tier account rollup** (platform-wide P&L + win rate → horizontal infinite slider of
per-asset-class slices → per-venue tiles) that loads **on-demand** with no background polling, with
Paper and Live as separate account levels; the **Automations** section offers both creation flows
(Single Instrument and Pipeline) with a live **stage board** showing membership counts and enter/exit
deltas; and **Settings** includes a **Venue Credentials** section with connect/disconnect/verify using
the verify-before-save flow.

## Prerequisites

- Phase 5 done: routing, glass-pill nav, Paper/Live mode store, section shells.
- Phase 4 done: `GET /api/dashboard/rollup?mode=PAPER|LIVE`, per-venue `AccountSource`.
- Phase 3 done: `AutomationPlan` model + schema + pipeline runtime, apply-list endpoint, credential
  service (verify-before-save).

## Invariants this phase must respect

- **Dashboard loads on-demand** (C-015): data fetches when the user navigates to Dashboard; **no
  background polling**.
- **Paper and Live are separate account levels** everywhere (C-118 family) — the dashboard mode comes
  from the Phase 5 mode store / an explicit Paper|Live selector.
- **No risk UI** (C-114): Settings has no risk section.
- **Credentials verify-before-save** (C-093/C-095): the UI must run a venue health check and only
  persist on success; plaintext credentials are never echoed back from the API.
- **No order book / minimum data baseline** carries over (C-129).

---

## Tasks

### P6-T01 — Dashboard data hook (on-demand rollup)
- **Goal:** A hook that fetches the three-tier rollup once on navigation, for the selected mode.
- **Files:** `frontend/src/hooks/useDashboardRollup.ts` (new), `frontend/src/api/rest.ts` (add
  `getDashboardRollup(mode)`).
- **Context:** Per C-015/C-079/C-080/C-081. Calls `GET /api/dashboard/rollup?mode=PAPER|LIVE` when the
  Dashboard mounts (and on explicit refresh / mode change) — **not** on an interval. Returns the
  platform total, per-asset-class entries, and per-venue tiles. Handle loading/empty/error states.
- **Acceptance:** navigating to Dashboard triggers exactly one fetch (verify no polling timer);
  switching Paper/Live refetches; the hook exposes the three tiers.
- **Depends on:** Phase 4 rollup endpoint.

### P6-T02 — Top tier: platform P&L + win rate
- **Goal:** The top-tier summary card.
- **Files:** `frontend/src/components/dashboard/PlatformSummary.tsx` (new).
- **Context:** Per C-079/C-081. Shows platform-wide P&L (USD baseline) and win rate for the selected
  mode, with a clear Paper/Live indicator. Numbers come from the rollup hook's top tier.
- **Acceptance:** the card renders the platform total P&L and win rate in USD and reflects the current
  Paper/Live mode.
- **Depends on:** P6-T01.

### P6-T03 — Middle tier: horizontal infinite slider of asset-class slices
- **Goal:** A horizontal infinite slider with one vertical slice per asset class.
- **Files:** `frontend/src/components/dashboard/AssetClassSlider.tsx` (new),
  `frontend/src/components/dashboard/AssetClassSlice.tsx` (new).
- **Context:** Per C-015/C-080. A horizontal infinite slider of vertical slices for the 8 classes
  (Crypto Spot, Equities, FX, Prediction Markets, Options, DEX/AMM, Perpetuals, Futures). Each slice
  shows that class's P&L + win rate and hosts its per-venue tiles (P6-T04). On-demand data only.
- **Acceptance:** the slider renders all 8 asset-class slices and scrolls horizontally (infinite);
  each slice shows its class-level P&L/win rate from the rollup.
- **Depends on:** P6-T01.

### P6-T04 — Bottom tier: per-venue tiles within a slice
- **Goal:** Per-venue tiles inside each asset-class slice.
- **Files:** `frontend/src/components/dashboard/VenueTile.tsx` (new).
- **Context:** Per C-053/C-081. Inside each slice, one tile per venue that serves that class (from the
  rollup's bottom tier), showing that venue's P&L/win rate/positions summary for the selected mode.
- **Acceptance:** a slice with multiple venues shows one tile per venue with that venue's numbers; the
  tiles sum to the slice total.
- **Depends on:** P6-T03.

### P6-T05 — Dashboard page assembly + on-demand load animation
- **Goal:** Compose the three tiers into `DashboardPage` with an on-demand load animation.
- **Files:** `frontend/src/pages/DashboardPage.tsx`.
- **Context:** Per C-015. Assemble PlatformSummary (top) + AssetClassSlider (middle, each slice with
  VenueTiles) (bottom). Show a load animation while the single on-demand fetch resolves. A Paper/Live
  selector (from the Phase 5 mode store) drives the mode. No background polling.
- **Acceptance:** the Dashboard renders all three tiers from one on-demand fetch with a load animation;
  switching Paper/Live updates all tiers; no polling timer exists.
- **Depends on:** P6-T02, P6-T03, P6-T04.

### P6-T06 — Automations: Single-Instrument flow
- **Goal:** The single-instrument automation creation flow.
- **Files:** `frontend/src/pages/AutomationsPage.tsx`,
  `frontend/src/components/automations/SingleInstrumentFlow.tsx` (new), `src/api/rest.ts` (add
  automation CRUD calls).
- **Context:** Per C-084/C-098. Flow: pick **asset class** → **instrument** → **one execution
  strategy** (apply-list filtered to compatible execution strategies) → **time window** (asset-class-
  aware: 24/7 vs sessioned). Arming the automation POSTs an `AutomationPlan{kind: SingleInstrument}`;
  once armed, orders flow through the runtime with no per-order confirmation.
- **Acceptance:** the flow lets the user build and arm a single-instrument automation; only compatible
  execution strategies appear; the time-window selector adapts to the asset class (24/7 vs sessioned);
  arming persists the plan.
- **Depends on:** P5-T01 (Automations route), Phase 3 apply-list + automation API.

### P6-T07 — Automations: Pipeline flow + stage board
- **Goal:** The pipeline automation builder with an ordered filter-stage board.
- **Files:** `frontend/src/components/automations/PipelineFlow.tsx` (new),
  `frontend/src/components/automations/StageBoard.tsx` (new),
  `frontend/src/components/automations/StageColumn.tsx` (new).
- **Context:** Per C-084/C-098. Flow: pick **asset class + universe** → add **ordered filter stages**
  as columns → attach **one final execution action**. The **stage board** shows, per column, live
  **membership counts** and **enter/exit deltas**, with instruments flowing in/out continuously
  (driven by the Phase 3 pipeline runtime, surfaced over the API/stream). Arming POSTs an
  `AutomationPlan{kind: Pipeline}`.
- **Acceptance:** the user can add ordered stage columns and a final action and arm the pipeline; the
  stage board shows per-stage counts and enter/exit deltas that update as membership changes; arming
  persists the plan.
- **Depends on:** P6-T06.

### P6-T08 — Settings: Venue Credentials section
- **Goal:** A Settings section to connect/disconnect/verify venue credentials with verify-before-save.
- **Files:** `frontend/src/pages/SettingsPage.tsx`,
  `frontend/src/components/settings/VenueCredentials.tsx` (new), `src/api/rest.ts` (add credential
  connect/verify/disconnect calls).
- **Context:** Per C-093/C-095. For each `SupportedVenue`, show connection status. **Connect** collects
  credentials, calls the venue **health check** (Phase 2 `GET /api/venues/{venue}/health` or a
  connect-with-verify endpoint), and **only saves on success** (AES-256-GCM envelope encryption on the
  backend). **Disconnect** removes stored credentials. Credentials are **never echoed back** from the
  API (the UI shows masked/status only). Settings also has Profile, Data & Privacy, Notifications,
  Appearance, Keyboard Shortcuts sections — but **no risk section** (C-114).
- **Acceptance:** entering bad credentials shows a failed verification and does **not** save; valid
  credentials verify then save and the venue shows connected; disconnect clears it; the API never
  returns plaintext credentials (the form does not pre-fill secrets).
- **Depends on:** P5-T01 (Settings route), Phase 2 health check, Phase 1 credential service.

---

## Phase exit criteria
- [ ] Dashboard loads the three-tier rollup on-demand (no polling) for the selected Paper/Live mode.
- [ ] Top tier shows platform P&L + win rate (USD); middle tier is a horizontal infinite slider of all
      8 asset-class slices; bottom tier shows per-venue tiles that sum to the slice total.
- [ ] Automations Single-Instrument flow builds + arms a plan with apply-list-filtered execution
      strategies and an asset-class-aware time window.
- [ ] Automations Pipeline flow builds ordered stage columns + a final action and shows a live stage
      board with counts and enter/exit deltas.
- [ ] Settings has a Venue Credentials section that verifies before save, never echoes secrets, and
      supports disconnect; no risk section exists.
- [ ] `frontend/` builds (`npm run build`).
