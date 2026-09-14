/* =============================================================================
   Asset-class identity.

   Spec §2.1.7 — policy:
     - identity in CHROME is a neutral chip + label, never a unique hue. This is
       what lets a twelfth class ship for free.
     - identity in DATA VISUALISATION uses the Instrument scale in a FIXED slot
       order that never changes. Slot 1 is Crypto Spot forever.
     - a class beyond the eight validated slots does NOT get an invented hue.
       It folds into "Other" (V8).

   The platform serves eleven classes. Eight map to slots; bonds, NFTs and
   expiring futures fold into Other and carry the neutral chip colour.
   ============================================================================= */

import type { AssetClass } from './format'

/** The identifiers the Rust platform emits. */
export type PlatformAssetClass =
  | 'crypto_spot_cex'
  | 'equity'
  | 'etf'
  | 'fx'
  | 'perpetual_swap'
  | 'option'
  | 'crypto_spot_dex'
  | 'prediction_market'
  | 'futures_expiring'
  | 'bond'
  | 'nft'

export interface AssetClassInfo {
  id: PlatformAssetClass
  label: string
  short: string
  /** 1-8 for a slotted class, null when it folds into Other. */
  slot: number | null
  /** Markets trade continuously. Drives the automation time-window default. */
  alwaysOpen: boolean
  /** Books are central-limit; enables stop orders and bracket attachments. */
  clob: boolean
  description: string
}

/** Declaration order IS display order, everywhere in the product. */
export const ASSET_CLASSES: AssetClassInfo[] = [
  { id: 'crypto_spot_cex', label: 'Crypto Spot', short: 'Crypto', slot: 1, alwaysOpen: true, clob: true, description: 'Centralised-exchange spot crypto.' },
  { id: 'equity', label: 'Equities', short: 'Equity', slot: 2, alwaysOpen: false, clob: true, description: 'Listed common stock.' },
  { id: 'etf', label: 'ETF', short: 'ETF', slot: 3, alwaysOpen: false, clob: true, description: 'Exchange-traded funds.' },
  { id: 'fx', label: 'FX', short: 'FX', slot: 4, alwaysOpen: false, clob: false, description: 'Spot foreign exchange.' },
  { id: 'perpetual_swap', label: 'Perpetuals', short: 'Perp', slot: 5, alwaysOpen: true, clob: true, description: 'Perpetual swaps with funding.' },
  { id: 'option', label: 'Options', short: 'Option', slot: 6, alwaysOpen: false, clob: false, description: 'Listed options contracts.' },
  { id: 'crypto_spot_dex', label: 'DEX / AMM', short: 'DEX', slot: 7, alwaysOpen: true, clob: true, description: 'On-chain spot via AMM pools.' },
  { id: 'prediction_market', label: 'Prediction', short: 'Predict', slot: 8, alwaysOpen: true, clob: false, description: 'Event contracts settling to 0 or 1.' },
  { id: 'futures_expiring', label: 'Futures', short: 'Futures', slot: null, alwaysOpen: false, clob: true, description: 'Dated futures contracts.' },
  { id: 'bond', label: 'Bonds', short: 'Bond', slot: null, alwaysOpen: false, clob: false, description: 'Fixed-income instruments.' },
  { id: 'nft', label: 'NFT', short: 'NFT', slot: null, alwaysOpen: true, clob: false, description: 'Non-fungible token collections.' },
]

const BY_ID = new Map(ASSET_CLASSES.map((a) => [a.id, a]))

export function assetClassInfo(id: string | undefined | null): AssetClassInfo {
  return (
    (id ? BY_ID.get(id as PlatformAssetClass) : undefined) ?? {
      id: 'crypto_spot_cex',
      label: id ? id.replace(/_/g, ' ') : 'Unknown',
      short: id ? id.replace(/_/g, ' ') : 'Unknown',
      slot: null,
      alwaysOpen: false,
      clob: false,
      description: 'Unrecognised asset class.',
    }
  )
}

export function assetClassLabel(id: string | undefined | null): string {
  return assetClassInfo(id).label
}

/**
 * The colour a class wears in a chart. Slotted classes get their fixed slot;
 * everything else gets the neutral chip colour — never an invented hue.
 */
export function assetClassChartColor(id: string | undefined | null): string {
  const slot = assetClassInfo(id).slot
  return slot === null ? 'var(--chip-line)' : `var(--chart-${slot})`
}

/** Map a platform class onto the spec's canonical vocabulary. */
export function toSpecAssetClass(id: string | undefined | null): AssetClass {
  switch (id) {
    case 'crypto_spot_cex': return 'crypto_spot'
    case 'equity': return 'equities'
    case 'etf': return 'etf'
    case 'fx': return 'fx'
    case 'perpetual_swap': return 'perpetuals'
    case 'option': return 'options'
    case 'crypto_spot_dex': return 'dex'
    case 'prediction_market': return 'prediction'
    default: return 'other'
  }
}

/** Guess a class from a symbol, used when metadata has not arrived. */
export function inferAssetClass(symbol: string): PlatformAssetClass {
  const s = symbol.toUpperCase()
  if (/-PERP$|PERP$|\.P$/.test(s)) return 'perpetual_swap'
  if (/-(USD|USDT|USDC|BTC|ETH|EUR)$/.test(s)) return 'crypto_spot_cex'
  if (/^[A-Z]{6}$/.test(s.replace(/[-/]/g, ''))) return 'fx'
  return 'equity'
}
