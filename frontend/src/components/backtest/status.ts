import type { BacktestStatus } from '@/api/backtests'
import type { BadgeProps } from '@/components/ui/badge'

/** Human label + badge variant + whether a spinner should show, per status. */
export function statusPresentation(status: BacktestStatus): {
  label: string
  variant: NonNullable<BadgeProps['variant']>
  busy: boolean
} {
  switch (status) {
    case 'queued':
      return { label: 'Queued', variant: 'outline', busy: true }
    case 'checking_data':
      return { label: 'Checking data', variant: 'default', busy: true }
    case 'collecting_data':
      return { label: 'Collecting data', variant: 'warning', busy: true }
    case 'loading_data':
      return { label: 'Loading data', variant: 'default', busy: true }
    case 'simulating':
      return { label: 'Simulating', variant: 'default', busy: true }
    case 'completed':
      return { label: 'Completed', variant: 'active', busy: false }
    case 'failed':
      return { label: 'Failed', variant: 'destructive', busy: false }
    case 'cancelled':
      return { label: 'Cancelled', variant: 'inactive', busy: false }
    default:
      return { label: status, variant: 'outline', busy: false }
  }
}

export function isActive(status: BacktestStatus): boolean {
  return !['completed', 'failed', 'cancelled'].includes(status)
}

/** Pulls a few headline numbers out of the simulator result document. */
export function resultHighlights(
  result: NonNullable<import('@/api/backtests').BacktestResult> | null,
): Array<{ label: string; value: string }> {
  if (!result) return []
  const out: Array<{ label: string; value: string }> = []

  if (result.total_orders !== undefined)
    out.push({ label: 'Orders', value: String(result.total_orders) })
  if (result.total_positions !== undefined)
    out.push({ label: 'Positions', value: String(result.total_positions) })

  // PnL: first currency's total if present.
  const pnls = result.stats_pnls
  if (pnls) {
    const firstCcy = Object.keys(pnls)[0]
    const total = firstCcy ? pnls[firstCcy]?.['PnL (total)'] : undefined
    if (firstCcy && typeof total === 'number') {
      out.push({ label: `PnL (${firstCcy})`, value: total.toFixed(2) })
    }
  }

  const general = result.stats_general
  if (general && typeof general['Sharpe Ratio (252 days)'] === 'number') {
    out.push({
      label: 'Sharpe',
      value: general['Sharpe Ratio (252 days)'].toFixed(2),
    })
  }
  const returns = result.stats_returns
  if (returns && typeof returns['Max Drawdown (%)'] === 'number') {
    out.push({
      label: 'Max DD',
      value: `${returns['Max Drawdown (%)'].toFixed(1)}%`,
    })
  }
  return out
}
