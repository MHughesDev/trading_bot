// Trading workspace persistence.
//
// The trading page's panel layout (which charts / terminals / scanners are
// open and in what order) and each chart's view settings (timeframe + active
// indicators) used to live in component `useState`, so a page refresh reset
// everything back to the default template. This store persists both to
// localStorage so the workspace survives reloads.

import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { layoutTemplates, type PanelSpec } from '@/config/layoutTemplates'
import type { IndicatorInstance } from '@/components/charts/MultiPaneChart'

/** Per-chart view settings, keyed by panel id. */
export interface ChartSettings {
  tfSecs: number
  indicators: IndicatorInstance[]
}

interface WorkspaceState {
  panels: PanelSpec[]
  chartSettings: Record<string, ChartSettings>
  setPanels: (updater: (prev: PanelSpec[]) => PanelSpec[]) => void
  setChartSettings: (key: string, settings: ChartSettings) => void
  removeChartSettings: (key: string) => void
}

export const useWorkspaceStore = create<WorkspaceState>()(
  persist(
    (set) => ({
      panels: layoutTemplates.default.panels,
      chartSettings: {},
      setPanels: (updater) => set((s) => ({ panels: updater(s.panels) })),
      setChartSettings: (key, settings) =>
        set((s) => ({ chartSettings: { ...s.chartSettings, [key]: settings } })),
      removeChartSettings: (key) =>
        set((s) => {
          if (!(key in s.chartSettings)) return s
          const next = { ...s.chartSettings }
          delete next[key]
          return { chartSettings: next }
        }),
    }),
    {
      name: 'tb-trading-workspace',
      version: 1,
      // Only persist data, not the action functions.
      partialize: (s) => ({ panels: s.panels, chartSettings: s.chartSettings }),
    },
  ),
)
