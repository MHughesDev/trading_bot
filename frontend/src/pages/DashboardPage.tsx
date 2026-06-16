// Dashboard — three-tier account rollup.
// Loads on-demand (no polling); Paper/Live mode from mode store.
// Platform P&L → horizontal asset-class slider → per-venue tiles.

import { useState } from 'react'
import { useModeStore } from '@/store/mode'
import { useDashboardRollup } from '@/hooks/useDashboardRollup'
import { PlatformSummary } from '@/components/dashboard/PlatformSummary'
import { AssetClassSlider } from '@/components/dashboard/AssetClassSlider'
import { paperApi } from '@/lib/api'
import { RefreshCw, Loader2, RotateCcw } from 'lucide-react'

export function DashboardPage() {
  const { mode } = useModeStore()
  const { rollup, loading, error, refresh } = useDashboardRollup(mode)
  const [resetting, setResetting] = useState(false)

  const isPaper = mode === 'PAPER'

  async function handleResetAll() {
    if (
      !window.confirm(
        'Reset ALL paper accounts? This restores starting cash/equity and wipes every position and transaction across all asset classes.',
      )
    )
      return
    setResetting(true)
    try {
      await paperApi.resetAll()
      refresh()
    } finally {
      setResetting(false)
    }
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
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
          {/* Top tier: platform summary + refresh */}
          <div className="relative">
            <PlatformSummary rollup={rollup} />
            <div className="absolute top-3 right-4 flex items-center gap-2">
              {isPaper && (
                <button
                  onClick={handleResetAll}
                  disabled={resetting}
                  className="flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs text-text-dim hover:text-red-400 hover:bg-red-500/10 border border-border hover:border-red-500/40 transition-colors disabled:opacity-40"
                  title="Reset all paper accounts to starting balances"
                >
                  {resetting ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <RotateCcw className="h-3.5 w-3.5" />
                  )}
                  Reset all
                </button>
              )}
              <button
                onClick={refresh}
                disabled={loading}
                className="rounded-lg p-1.5 text-text-dim hover:text-text hover:bg-border border border-border transition-colors disabled:opacity-40"
                aria-label="Refresh"
              >
                {loading ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <RefreshCw className="h-3.5 w-3.5" />
                )}
              </button>
            </div>
          </div>

          {/* Middle + bottom tier: asset-class slider with venue tiles */}
          <div className="flex-1 overflow-hidden">
            <AssetClassSlider
              tiles={rollup.by_asset_class}
              onReset={isPaper ? refresh : undefined}
            />
          </div>
        </div>
      )}
    </div>
  )
}
