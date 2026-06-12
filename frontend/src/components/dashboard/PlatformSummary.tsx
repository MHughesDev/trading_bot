import type { DashboardRollup } from '@/hooks/useDashboardRollup'
import { cn, fmtCurrency, pnlClass } from '@/lib/utils'

interface PlatformSummaryProps {
  rollup: DashboardRollup
}

export function PlatformSummary({ rollup }: PlatformSummaryProps) {
  const realized = parseFloat(rollup.realized_pnl_usd)
  const unrealized = parseFloat(rollup.unrealized_pnl_usd)
  const total = realized + unrealized

  return (
    <div className="flex items-center gap-8 px-6 py-4 border-b border-border bg-surface shrink-0">
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

      {/* Paper mode: bot-wide internal account totals */}
      {rollup.account_totals && (
        <div className="flex items-center gap-6 pl-6 border-l border-border">
          <div>
            <div className="text-xs text-text-dim mb-0.5">Account equity (USD)</div>
            <div className="text-lg font-mono font-semibold text-text tabular-nums">
              {fmtCurrency(parseFloat(rollup.account_totals.equity_usd))}
            </div>
          </div>
          <div className="text-sm text-text-muted">
            <div>
              <span className="text-text-dim mr-1">Cash</span>
              <span className="font-mono tabular-nums">
                {fmtCurrency(parseFloat(rollup.account_totals.cash_usd))}
              </span>
            </div>
            <div>
              <span className="text-text-dim mr-1">Fees paid</span>
              <span className="font-mono tabular-nums">
                {fmtCurrency(parseFloat(rollup.account_totals.fees_paid_usd))}
              </span>
            </div>
          </div>
          <div>
            <div className="text-xs text-text-dim mb-0.5">Open positions</div>
            <div className="text-lg font-mono font-semibold text-text tabular-nums">
              {rollup.account_totals.open_positions}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
