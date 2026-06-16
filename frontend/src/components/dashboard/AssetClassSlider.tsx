// Horizontal infinite slider of per-asset-class slices.
// One vertical slice per asset class, each containing per-venue tiles.

import type { AssetClassTile } from '@/hooks/useDashboardRollup'
import { AssetClassSlice } from './AssetClassSlice'

// All 11 asset classes the paper engine opens accounts for (domain order).
const ALL_CLASSES = [
  'crypto_spot_cex',
  'equity',
  'etf',
  'fx',
  'prediction_market',
  'option',
  'crypto_spot_dex',
  'perpetual_swap',
  'futures_expiring',
  'bond',
  'nft',
]

interface AssetClassSliderProps {
  tiles: AssetClassTile[]
  /** Paper mode only: called after a per-class reset to refresh the rollup. */
  onReset?: () => void
}

export function AssetClassSlider({ tiles, onReset }: AssetClassSliderProps) {
  const tileMap = new Map(tiles.map((t) => [t.asset_class, t]))

  return (
    <div className="flex overflow-x-auto overflow-y-hidden h-full border-t border-border">
      {ALL_CLASSES.map((ac) => {
        const tile = tileMap.get(ac) ?? {
          asset_class: ac,
          realized_pnl_usd: '0',
          unrealized_pnl_usd: '0',
          win_rate: 0,
          venues: [],
        }
        return <AssetClassSlice key={ac} tile={tile} onReset={onReset} />
      })}
    </div>
  )
}
