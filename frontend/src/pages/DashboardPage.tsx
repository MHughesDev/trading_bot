// Dashboard — three-tier account rollup.
// Loads on-demand (no polling); Paper/Live mode from mode store.
// Platform P&L → horizontal asset-class slider → per-venue tiles.

import { useModeStore } from '@/store/mode'
import { useDashboardRollup } from '@/hooks/useDashboardRollup'
import { PlatformSummary } from '@/components/dashboard/PlatformSummary'
import { AssetClassSlider } from '@/components/dashboard/AssetClassSlider'
import { ModeBadge } from '@/components/layout/ModeBadge'
import { RefreshCw, Loader2 } from 'lucide-react'

export function DashboardPage() {
  const { mode } = useModeStore()
  const { rollup, loading, error, refresh } = useDashboardRollup(mode)

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Header bar */}
      <div className="flex items-center gap-3 px-6 py-3 border-b border-border bg-surface shrink-0">
        <h1 className="text-sm font-semibold text-text">Dashboard</h1>
        <div className="ml-auto flex items-center gap-2">
          <ModeBadge />
          <button
            onClick={refresh}
            disabled={loading}
            className="flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs text-text-muted hover:text-text hover:bg-border border border-border transition-colors disabled:opacity-40"
          >
            {loading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
            Refresh
          </button>
        </div>
      </div>

      {/* Loading state */}
      {loading && !rollup && (
        <div className="flex flex-1 items-center justify-center">
          <div className="flex flex-col items-center gap-3 text-text-dim">
            <Loader2 className="h-8 w-8 animate-spin" />
            <span className="text-sm">Loading portfolio…</span>
          </div>
        </div>
      )}

      {/* Error state */}
      {error && !loading && (
        <div className="flex flex-1 items-center justify-center">
          <div className="flex flex-col items-center gap-3 text-red-400">
            <span className="text-sm">{error}</span>
            <button
              onClick={refresh}
              className="text-xs underline hover:no-underline"
            >
              Retry
            </button>
          </div>
        </div>
      )}

      {/* Content */}
      {rollup && (
        <div className="flex flex-col flex-1 overflow-hidden">
          {/* Top tier: platform summary */}
          <PlatformSummary rollup={rollup} mode={mode} />

          {/* Middle + bottom tier: asset-class slider with venue tiles */}
          <div className="flex-1 overflow-hidden">
            <AssetClassSlider tiles={rollup.by_asset_class} />
          </div>
        </div>
      )}
    </div>
  )
}
