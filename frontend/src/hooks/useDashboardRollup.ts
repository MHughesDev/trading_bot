// On-demand dashboard rollup hook — fetches exactly once on mount (or mode change),
// never on an interval.  Per C-015: Dashboard loads on-demand.

import { useState, useEffect, useCallback } from 'react'
import { api } from '@/lib/api'
import type { TradingMode } from '@/store/mode'

export interface VenueTile {
  venue: string
  realized_pnl_usd: string
  unrealized_pnl_usd: string
  win_rate: number
  trade_count: number
}

export interface PaperPositionInfo {
  instrument_id: string
  quantity: string
  average_entry_price: string
  mark_price: string | null
  unrealized_pnl: string
  notional: string
}

/** Paper mode only: internal account data for one asset class. */
export interface PaperAccountInfo {
  currency: string
  cash: string
  equity: string
  used_margin: string
  free_collateral: string
  fees_paid: string
  open_positions: number
  positions: PaperPositionInfo[]
}

/** Paper mode only: bot-wide account totals across all asset classes. */
export interface PaperAccountTotals {
  equity_usd: string
  cash_usd: string
  realized_pnl_usd: string
  unrealized_pnl_usd: string
  fees_paid_usd: string
  open_positions: number
  excluded_currencies: string[]
}

export interface AssetClassTile {
  asset_class: string
  realized_pnl_usd: string
  unrealized_pnl_usd: string
  win_rate: number
  venues: VenueTile[]
  account?: PaperAccountInfo
}

export interface DashboardRollup {
  mode: string
  realized_pnl_usd: string
  unrealized_pnl_usd: string
  win_rate: number
  by_asset_class: AssetClassTile[]
  account_totals?: PaperAccountTotals
}

interface UseDashboardRollupResult {
  rollup: DashboardRollup | null
  loading: boolean
  error: string | null
  refresh: () => void
}

export function useDashboardRollup(mode: TradingMode): UseDashboardRollupResult {
  const [rollup, setRollup] = useState<DashboardRollup | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const fetch = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const res = await api.get<DashboardRollup>('/api/dashboard/rollup', {
        params: { mode },
      })
      setRollup(res.data)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load dashboard')
    } finally {
      setLoading(false)
    }
  }, [mode])

  // Fetch on mount and on mode change — never on an interval.
  useEffect(() => {
    void fetch()
  }, [fetch])

  return { rollup, loading, error, refresh: fetch }
}
