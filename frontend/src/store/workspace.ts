// Trading workspace persistence — scoped per trading mode.
//
// The trading page's panel layout (which charts / terminals / scanners are
// open and in what order) and each chart's view settings (timeframe + active
// indicators) used to live in component `useState`, so a page refresh reset
// everything back to the default template.
//
// State is keyed by trading mode (PAPER | LIVE) so the multi-window model from
// `store/mode.ts` stays coherent: a window pinned to LIVE (`?mode=live`) keeps
// its own panels independent of a PAPER window, and two windows in the same
// mode share one layout. Everything is persisted to localStorage and mirrored
// across tabs via the `storage` event (same approach as the mode store), so a
// change in one window is reflected in others and windows never clobber each
// other's slice.

import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { layoutTemplates, type PanelSpec } from '@/config/layoutTemplates'
import type { IndicatorInstance } from '@/components/charts/MultiPaneChart'
import type { TradingMode } from '@/store/mode'

/** Per-chart view settings, keyed by panel id. */
export interface ChartSettings {
  tfSecs: number
  indicators: IndicatorInstance[]
}

/** One mode's complete workspace. */
interface ModeWorkspace {
  panels: PanelSpec[]
  chartSettings: Record<string, ChartSettings>
}

interface WorkspaceState {
  byMode: Record<TradingMode, ModeWorkspace>
  setPanels: (mode: TradingMode, updater: (prev: PanelSpec[]) => PanelSpec[]) => void
  setChartSettings: (mode: TradingMode, key: string, settings: ChartSettings) => void
  removeChartSettings: (mode: TradingMode, key: string) => void
}

const STORAGE_KEY = 'tb-trading-workspace'

/** A fresh workspace seeded from the default layout template. */
const freshWorkspace = (): ModeWorkspace => ({
  panels: layoutTemplates.default.panels,
  chartSettings: {},
})

export const useWorkspaceStore = create<WorkspaceState>()(
  persist(
    (set) => ({
      byMode: {
        PAPER: freshWorkspace(),
        LIVE: freshWorkspace(),
      },
      setPanels: (mode, updater) =>
        set((s) => ({
          byMode: {
            ...s.byMode,
            [mode]: { ...s.byMode[mode], panels: updater(s.byMode[mode].panels) },
          },
        })),
      setChartSettings: (mode, key, settings) =>
        set((s) => ({
          byMode: {
            ...s.byMode,
            [mode]: {
              ...s.byMode[mode],
              chartSettings: { ...s.byMode[mode].chartSettings, [key]: settings },
            },
          },
        })),
      removeChartSettings: (mode, key) =>
        set((s) => {
          const ws = s.byMode[mode]
          if (!(key in ws.chartSettings)) return s
          const nextSettings = { ...ws.chartSettings }
          delete nextSettings[key]
          return { byMode: { ...s.byMode, [mode]: { ...ws, chartSettings: nextSettings } } }
        }),
    }),
    {
      name: STORAGE_KEY,
      version: 2,
      // Persist data only, not the action functions.
      partialize: (s) => ({ byMode: s.byMode }),
      // v1 was a single flat workspace `{ panels, chartSettings }` (mode-agnostic).
      // Adopt it as the PAPER workspace and seed LIVE fresh.
      migrate: (persisted, version) => {
        if (
          version < 2 &&
          persisted &&
          typeof persisted === 'object' &&
          'panels' in (persisted as Record<string, unknown>)
        ) {
          const old = persisted as { panels: PanelSpec[]; chartSettings?: Record<string, ChartSettings> }
          return {
            byMode: {
              PAPER: { panels: old.panels, chartSettings: old.chartSettings ?? {} },
              LIVE: freshWorkspace(),
            },
          }
        }
        return persisted as { byMode: Record<TradingMode, ModeWorkspace> }
      },
    },
  ),
)

// Mirror workspace changes across tabs/windows. Without this, each window holds
// its own in-memory copy of *both* modes and the last writer would clobber the
// other mode's slice in localStorage. Re-reading on the `storage` event keeps
// every window's `byMode` in sync (the event fires only in *other* tabs).
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (e) => {
    if (e.key === STORAGE_KEY) void useWorkspaceStore.persist.rehydrate()
  })
}
