import { useState } from 'react'
import { RotateCcw, Loader2 } from 'lucide-react'
import type { AssetClassTile, PaperAccountInfo } from '@/hooks/useDashboardRollup'
import { paperApi } from '@/lib/api'
import { VenueTile } from './VenueTile'
import { cn, fmtCurrency, pnlClass } from '@/lib/utils'

const AC_LABELS: Record<string, string> = {
  crypto_spot_cex: 'Crypto Spot',
  equity: 'Equities',
  etf: 'ETF',
  bond: 'Bonds',
  fx: 'FX',
  prediction_market: 'Prediction',
  option: 'Options',
  crypto_spot_dex: 'DEX/AMM',
  perpetual_swap: 'Perpetuals',
  futures_expiring: 'Futures',
  nft: 'NFT',
}

interface AssetClassSliceProps {
  tile: AssetClassTile
  /** Paper mode only: called after this class's account is reset. */
  onReset?: () => void
}

export function AssetClassSlice({ tile, onReset }: AssetClassSliceProps) {
  const realized = parseFloat(tile.realized_pnl_usd)
  const unrealized = parseFloat(tile.unrealized_pnl_usd)
  const total = realized + unrealized
  const label = AC_LABELS[tile.asset_class] ?? tile.asset_class
  const [resetting, setResetting] = useState(false)

  // Reset is offered only in paper mode (account data present) with a refresh cb.
  const canReset = !!tile.account && !!onReset

  async function handleReset() {
    if (
      !window.confirm(
        `Reset the ${label} paper account? Starting cash/equity is restored and all positions and transactions for this asset class are wiped.`,
      )
    )
      return
    setResetting(true)
    try {
      await paperApi.resetAccount(tile.asset_class)
      onReset?.()
    } finally {
      setResetting(false)
    }
  }

  return (
    <div className="flex flex-col shrink-0 w-64 h-full border-r border-border">
      {/* Slice header — fixed */}
      <div className="px-4 py-4 shrink-0">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-text">{label}</h3>
          {canReset && (
            <button
              onClick={handleReset}
              disabled={resetting}
              className="flex items-center justify-center rounded-md p-1 text-text-dim hover:text-red-400 hover:bg-red-500/10 border border-transparent hover:border-red-500/40 transition-colors disabled:opacity-40"
              title={`Reset ${label} paper account`}
              aria-label={`Reset ${label} paper account`}
            >
              {resetting ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <RotateCcw className="h-3.5 w-3.5" />
              )}
            </button>
          )}
        </div>
        <div className="flex items-baseline gap-2 mt-1">
          <span className={cn('text-xl font-mono font-semibold tabular-nums', pnlClass(total))}>
            {(total >= 0 ? '+' : '') + fmtCurrency(total)}
          </span>
          <span className="text-xs text-text-dim">
            {Math.round(tile.win_rate * 100)}% win
          </span>
        </div>
      </div>

      {/* Body — paper shows the internal account, live shows venue tiles */}
      <div className="flex-1 overflow-y-auto px-4 pb-4">
        {tile.account ? (
          <PaperAccountPanel account={tile.account} />
        ) : tile.venues.length === 0 ? (
          <div className="text-xs text-text-dim">No trades</div>
        ) : (
          <div className="flex flex-col gap-2">
            {tile.venues.map((v) => (
              <VenueTile key={v.venue} tile={v} />
            ))}
          </div>
        )}
      </div>
    </div>
  )
}

// Paper mode: asset-class account data from the internal paper engine —
// there is no venue to pick; execution is fully in-process.
function PaperAccountPanel({ account }: { account: PaperAccountInfo }) {
  const rows: Array<[string, string]> = [
    ['Cash', fmtMoney(account.cash, account.currency)],
    ['Equity', fmtMoney(account.equity, account.currency)],
  ]
  const usedMargin = parseFloat(account.used_margin)
  if (usedMargin > 0) {
    rows.push(['Used margin', fmtMoney(account.used_margin, account.currency)])
    rows.push(['Free collateral', fmtMoney(account.free_collateral, account.currency)])
  }
  const fees = parseFloat(account.fees_paid)
  if (fees > 0) rows.push(['Fees paid', fmtMoney(account.fees_paid, account.currency)])

  return (
    <div className="flex flex-col gap-3">
      <div className="rounded-lg border border-border bg-surface-2 p-3 space-y-1">
        {rows.map(([label, value]) => (
          <div key={label} className="flex justify-between text-xs">
            <span className="text-text-dim">{label}</span>
            <span className="font-mono tabular-nums text-text">{value}</span>
          </div>
        ))}
      </div>

      {account.positions.length > 0 && (
        <div className="space-y-1.5">
          <div className="text-[10px] font-semibold uppercase tracking-wider text-text-dim">
            Positions
          </div>
          {account.positions.map((p) => {
            const upnl = parseFloat(p.unrealized_pnl)
            return (
              <div
                key={p.instrument_id}
                className="flex items-center justify-between rounded-md border border-border px-2.5 py-1.5 text-xs"
              >
                <div className="min-w-0">
                  <div className="font-mono text-text truncate">{p.instrument_id}</div>
                  <div className="text-text-dim">
                    {p.quantity} @ {p.average_entry_price}
                  </div>
                </div>
                <span className={cn('font-mono tabular-nums shrink-0 pl-2', pnlClass(upnl))}>
                  {(upnl >= 0 ? '+' : '') + fmtCurrency(upnl)}
                </span>
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}

function fmtMoney(value: string, currency: string): string {
  const n = parseFloat(value)
  if (currency === 'USD' || currency === 'USDC') return fmtCurrency(n)
  return `${n.toLocaleString()} ${currency}`
}
