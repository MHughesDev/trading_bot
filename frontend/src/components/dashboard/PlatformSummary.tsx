import type { DashboardRollup } from '@/hooks/useDashboardRollup'
import type { TradingMode } from '@/store/mode'
import { cn, fmtCurrency, pnlClass } from '@/lib/utils'

interface PlatformSummaryProps {
  rollup: DashboardRollup
  mode: TradingMode
}

export function PlatformSummary({ rollup, mode }: PlatformSummaryProps) {
  const realized = parseFloat(rollup.realized_pnl_usd)
  const unrealized = parseFloat(rollup.unrealized_pnl_usd)
  const total = realized + unrealized

  return (
    <div className="flex items-center gap-8 px-6 py-4 border-b border-border bg-surface shrink-0">
      {/* Mode badge */}
      <div
        className={cn(
          'rounded-full px-3 py-1 text-xs font-semibold border',
          mode === 'PAPER'
            ? 'bg-amber-500/10 text-amber-400 border-amber-500/30'
            : 'bg-green-500/10 text-green-400 border-green-500/30',
        )}
      >
        {mode}
      </div>

      {/* Platform P&L */}
      <div>
        <div className="text-xs text-text-dim mb-0.5">Total P&L (USD)</div>
        <div className={cn('text-2xl font-mono font-bold tabular-nums', pnlClass(total))}>
          {(total >= 0 ? '+' : '') + fmtCurrency(total)}
        </div>
      </div>

      {/* Realized / Unrealized breakdown */}
      <div className="text-sm text-text-muted">
        <div className="flex gap-4">
          <div>
            <span className="text-text-dim mr-1">Realized</span>
            <span className={cn('font-mono tabular-nums', pnlClass(realized))}>
              {(realized >= 0 ? '+' : '') + fmtCurrency(realized)}
            </span>
          </div>
          <div>
            <span className="text-text-dim mr-1">Unrealized</span>
            <span className={cn('font-mono tabular-nums', pnlClass(unrealized))}>
              {(unrealized >= 0 ? '+' : '') + fmtCurrency(unrealized)}
            </span>
          </div>
        </div>
      </div>

      {/* Win rate */}
      <div>
        <div className="text-xs text-text-dim mb-0.5">Win rate</div>
        <div className="text-lg font-mono font-semibold text-text">
          {Math.round(rollup.win_rate * 100)}%
        </div>
      </div>
    </div>
  )
}
