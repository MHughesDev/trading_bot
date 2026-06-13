# Phase 5 — Frontend

**Completion: 0% (0 / 4 tasks complete)**

**Goal:** Clean up the UI integration and remove the rough edges.
**Addresses:** #27, #28, #29

---

## Tasks

### ☐ 5.1 Consolidate API clients — S
**Addresses #27.** There are now three axios instances
(`lib/api.ts`, `api/rest.ts`, `api/backtests.ts`).
- Fold `api/backtests.ts` into one shared instance/interceptor pattern (single
  token-attaching client); keep the typed `backtestsApi` surface.
- **Files:** `frontend/src/api/backtests.ts`, shared client module.

### ☐ 5.2 Reconcile the strategy picker — M
**Addresses #28.** The picker lists from the Rust `/api/strategies` in-memory
store, but visual-builder strategies may live on the legacy `:8001` surface.
- Confirm where authored strategies actually live; source the picker from the
  correct backend (or unify the two surfaces).
- **Files:** `frontend/src/components/backtest/CreateBacktestDialog.tsx`,
  `frontend/src/api/backtests.ts`.
- **Verify:** a strategy created in the builder appears in the picker.

### ☐ 5.3 Code-split the bundle — S
**Addresses #29.** The build emits a single >500 kB chunk.
- Lazy-load the Back Testing page (and other heavy routes) via `React.lazy` +
  `Suspense` so the page isn't in the main chunk.
- **Files:** `frontend/src/App.tsx`.
- **Verify:** `npm run build` shows the page split into its own chunk.

### ☐ 5.4 Optional: custom date-range picker — S
- Add absolute start/end date inputs alongside the duration presets so runs can
  target a fixed historical window rather than always ending "today."
- **Files:** `frontend/src/components/backtest/CreateBacktestDialog.tsx`.

---

## Definition of Done
One API client; the strategy picker reflects the user's real strategies; the
Back Testing route is code-split; (optionally) absolute date ranges are
selectable. `tsc -b`, `vite build`, and `eslint` stay clean.
