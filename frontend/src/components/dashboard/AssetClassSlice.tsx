import type { AssetClassTile } from '@/hooks/useDashboardRollup'
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
}

export function AssetClassSlice({ tile }: AssetClassSliceProps) {
  const realized = parseFloat(tile.realized_pnl_usd)
  const unrealized = parseFloat(tile.unrealized_pnl_usd)
  const total = realized + unrealized
  const label = AC_LABELS[tile.asset_class] ?? tile.asset_class

  return (
    <div className="flex flex-col gap-3 shrink-0 w-64 border-r border-border px-4 py-4">
      {/* Slice header */}
      <div>
        <h3 className="text-sm font-semibold text-text">{label}</h3>
        <div className="flex items-baseline gap-2 mt-1">
          <span className={cn('text-xl font-mono font-semibold tabular-nums', pnlClass(total))}>
            {(total >= 0 ? '+' : '') + fmtCurrency(total)}
          </span>
          <span className="text-xs text-text-dim">
            {Math.round(tile.win_rate * 100)}% win
          </span>
        </div>
      </div>

      {/* Venue tiles */}
      {tile.venues.length === 0 ? (
        <div className="text-xs text-text-dim">No trades</div>
      ) : (
        <div className="flex flex-col gap-2">
          {tile.venues.map((v) => (
            <VenueTile key={v.venue} tile={v} />
          ))}
        </div>
      )}
    </div>
  )
}
