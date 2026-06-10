import type { VenueTile as VenueTileData } from '@/hooks/useDashboardRollup'
import { cn, fmtCurrency, pnlClass } from '@/lib/utils'

interface VenueTileProps {
  tile: VenueTileData
}

function formatPnl(val: string): string {
  const n = parseFloat(val)
  return (n >= 0 ? '+' : '') + fmtCurrency(n)
}

export function VenueTile({ tile }: VenueTileProps) {
  const realized = parseFloat(tile.realized_pnl_usd)
  const unrealized = parseFloat(tile.unrealized_pnl_usd)
  const total = realized + unrealized

  return (
    <div className="rounded-xl border border-border bg-surface-2 px-4 py-3 space-y-2 min-w-[160px]">
      <div className="flex items-center justify-between">
        <span className="text-xs font-semibold uppercase tracking-wider text-text-muted">
          {tile.venue}
        </span>
        <span className="text-xs text-text-dim">{Math.round(tile.win_rate * 100)}% win</span>
      </div>
      <div className={cn('text-lg font-mono font-semibold tabular-nums', pnlClass(total))}>
        {formatPnl(String(total))}
      </div>
      <div className="text-xs text-text-dim tabular-nums">
        {tile.trade_count} trade{tile.trade_count !== 1 ? 's' : ''}
      </div>
    </div>
  )
}
