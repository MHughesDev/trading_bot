/* =============================================================================
   format.ts — spec §5.3
   The ONLY place in the codebase where a number becomes a string.

   Rules enforced here (spec §2.2.5):
     N2  decimal places are fixed per instrument, from instrument metadata
     N3  thousands separators on every figure >= 1000, locale-aware
     N4  abbreviation only outside the execution path
     N5  2dp returns, 2dp allocation, 0dp win rate
     N6  minus is U+2212, plus is a literal '+'
   Callers must never re-format the result.
   ============================================================================= */

export const MINUS = '−' // U+2212 MINUS SIGN — never a hyphen
export const UP = '▲'
export const DOWN = '▼'
export const MIDDOT = '·'
export const TIMES = '×'

export type Numish = number | string | null | undefined

/** Instrument metadata that drives precision. Supplied by the instrument feed. */
export interface InstrumentMeta {
  symbol: string
  /** decimals used for every price of this instrument */
  priceDp: number
  /** decimals used for every size/quantity of this instrument */
  qtyDp: number
  /** quote currency for money figures derived from this instrument */
  quote?: string
  assetClass?: AssetClass
}

export type AssetClass =
  | 'crypto_spot'
  | 'equities'
  | 'etf'
  | 'fx'
  | 'perpetuals'
  | 'options'
  | 'dex'
  | 'prediction'
  | 'other'

/* -----------------------------------------------------------------------------
   Asset-class → fixed chart slot (spec §2.1.7). Slot order NEVER changes.
   -------------------------------------------------------------------------- */
export const ASSET_CLASS_SLOT: Record<AssetClass, number> = {
  crypto_spot: 1,
  equities: 2,
  etf: 3,
  fx: 4,
  perpetuals: 5,
  options: 6,
  dex: 7,
  prediction: 8,
  other: 8,
}

export const ASSET_CLASS_LABEL: Record<AssetClass, string> = {
  crypto_spot: 'Crypto Spot',
  equities: 'Equities',
  etf: 'ETF',
  fx: 'FX',
  perpetuals: 'Perpetuals',
  options: 'Options',
  dex: 'DEX / AMM',
  prediction: 'Prediction',
  other: 'Other',
}

/** The CSS variable that paints a given asset class in a chart. */
export function assetClassColor(cls: AssetClass | undefined): string {
  const slot = ASSET_CLASS_SLOT[cls ?? 'other'] ?? 8
  return `var(--chart-${slot})`
}

/** The CSS variable for a fixed categorical slot (1-8). Never cycled (V7). */
export function seriesColor(slot: number): string {
  const s = Math.min(8, Math.max(1, Math.round(slot)))
  return `var(--chart-${s})`
}

/* -----------------------------------------------------------------------------
   Instrument precision resolution
   -------------------------------------------------------------------------- */

const DEFAULT_META: InstrumentMeta = { symbol: '', priceDp: 2, qtyDp: 4, quote: 'USD' }

/** Precision fallbacks by asset class, used only when metadata is absent. */
function fallbackMeta(symbol: string): InstrumentMeta {
  const s = symbol.toUpperCase()
  // FX majors quote to 4dp (5dp pip display handled by pipPrice()).
  if (/^[A-Z]{3}[/-]?[A-Z]{3}$/.test(s.replace(/[-/]/g, '')) && !s.includes('USD-') && /USD|EUR|JPY|GBP|CHF|AUD|NZD|CAD/.test(s)) {
    const noSep = s.replace(/[-/]/g, '')
    if (noSep.length === 6 && !/BTC|ETH|SOL/.test(noSep)) {
      return { symbol, priceDp: noSep.endsWith('JPY') ? 3 : 4, qtyDp: 0, quote: noSep.slice(3) }
    }
  }
  if (/BTC|ETH|SOL|AVAX|DOGE|XRP|ADA|LINK|MATIC|USDT|USDC/.test(s)) {
    return { symbol, priceDp: 2, qtyDp: 4, quote: 'USD' }
  }
  return { symbol, priceDp: 2, qtyDp: 0, quote: 'USD' }
}

const metaRegistry = new Map<string, InstrumentMeta>()

/** Register instrument metadata once, from the instrument feed. */
export function registerInstrument(meta: InstrumentMeta): void {
  metaRegistry.set(meta.symbol.toUpperCase(), meta)
}

export function registerInstruments(metas: InstrumentMeta[]): void {
  for (const m of metas) registerInstrument(m)
}

export function instrumentMeta(symbol: string | InstrumentMeta | undefined): InstrumentMeta {
  if (!symbol) return DEFAULT_META
  if (typeof symbol !== 'string') return symbol
  return metaRegistry.get(symbol.toUpperCase()) ?? fallbackMeta(symbol)
}

/* -----------------------------------------------------------------------------
   Core numeric helpers
   -------------------------------------------------------------------------- */

export function toNum(v: Numish): number | null {
  if (v === null || v === undefined || v === '') return null
  const n = typeof v === 'number' ? v : Number(v)
  return Number.isFinite(n) ? n : null
}

/** Group + fix decimals. Uses U+2212 for negatives. */
function fixed(n: number, dp: number): string {
  const abs = Math.abs(n)
  const body = abs.toLocaleString(undefined, {
    minimumFractionDigits: dp,
    maximumFractionDigits: dp,
  })
  return (n < 0 ? MINUS : '') + body
}

/** Placeholder for a value that does not exist. Never renders as 0. */
export const DASH = '—'

/* -----------------------------------------------------------------------------
   Public API (spec §5.3)
   -------------------------------------------------------------------------- */

/** A price for a specific instrument. Decimals come from metadata, not the call site. */
export function price(instrument: string | InstrumentMeta | undefined, v: Numish): string {
  const n = toNum(v)
  if (n === null) return DASH
  return fixed(n, instrumentMeta(instrument).priceDp)
}

/** A size / quantity for a specific instrument. */
export function qty(instrument: string | InstrumentMeta | undefined, v: Numish): string {
  const n = toNum(v)
  if (n === null) return DASH
  return fixed(n, instrumentMeta(instrument).qtyDp)
}

/** A money figure. Full precision — this is execution-path safe. */
export function money(v: Numish, ccy = 'USD', dp = 2): string {
  const n = toNum(v)
  if (n === null) return DASH
  const sym = currencySymbol(ccy)
  const abs = Math.abs(n).toLocaleString(undefined, {
    minimumFractionDigits: dp,
    maximumFractionDigits: dp,
  })
  return `${n < 0 ? MINUS : ''}${sym}${abs}`
}

/** A money figure that always carries its sign (spec §2.1.5). Zero is neutral. */
export function signedMoney(v: Numish, ccy = 'USD', dp = 2): string {
  const n = toNum(v)
  if (n === null) return DASH
  if (n === 0) return money(0, ccy, dp)
  const sym = currencySymbol(ccy)
  const abs = Math.abs(n).toLocaleString(undefined, {
    minimumFractionDigits: dp,
    maximumFractionDigits: dp,
  })
  return `${n > 0 ? '+' : MINUS}${sym}${abs}`
}

/** A bare number that always carries its sign. Zero is neutral, unsigned. */
export function signed(v: Numish, dp = 2): string {
  const n = toNum(v)
  if (n === null) return DASH
  if (n === 0) return fixed(0, dp)
  const abs = Math.abs(n).toLocaleString(undefined, {
    minimumFractionDigits: dp,
    maximumFractionDigits: dp,
  })
  return `${n > 0 ? '+' : MINUS}${abs}`
}

/** A percentage. 2dp for returns and allocation, 0dp for win rate (N5). */
export function pct(v: Numish, dp = 2): string {
  const n = toNum(v)
  if (n === null) return DASH
  return `${fixed(n, dp)}%`
}

/** A percentage that always carries its sign. */
export function signedPct(v: Numish, dp = 2): string {
  const n = toNum(v)
  if (n === null) return DASH
  if (n === 0) return `${fixed(0, dp)}%`
  return `${signed(n, dp)}%`
}

/** Win rate — 0dp per N5. Accepts either 0-1 or 0-100. */
export function winRate(v: Numish): string {
  const n = toNum(v)
  if (n === null) return DASH
  const asPct = Math.abs(n) <= 1 ? n * 100 : n
  return `${fixed(asPct, 0)}%`
}

/**
 * Abbreviated aggregate. N4: MUST NOT be used in order tickets, fills,
 * positions or balances — only in headline/aggregate chrome.
 */
export function compact(v: Numish, dp = 2): string {
  const n = toNum(v)
  if (n === null) return DASH
  const abs = Math.abs(n)
  const sign = n < 0 ? MINUS : ''
  if (abs >= 1e12) return `${sign}${(abs / 1e12).toFixed(dp)}T`
  if (abs >= 1e9) return `${sign}${(abs / 1e9).toFixed(dp)}B`
  if (abs >= 1e6) return `${sign}${(abs / 1e6).toFixed(dp)}M`
  if (abs >= 1e3) return `${sign}${(abs / 1e3).toFixed(abs >= 1e5 ? 0 : 1)}k`
  return `${sign}${abs.toFixed(dp)}`
}

/** Abbreviated money for chart axes and headline chrome only (N4). */
export function compactMoney(v: Numish, ccy = 'USD'): string {
  const n = toNum(v)
  if (n === null) return DASH
  const sym = currencySymbol(ccy)
  const body = compact(Math.abs(n), 2)
  return `${n < 0 ? MINUS : ''}${sym}${body}`
}

/** A plain integer count with separators. */
export function count(v: Numish): string {
  const n = toNum(v)
  if (n === null) return DASH
  return fixed(n, 0)
}

/** A ratio like Sharpe or profit factor — fixed 2dp, signed only when negative. */
export function ratio(v: Numish, dp = 2): string {
  const n = toNum(v)
  if (n === null) return DASH
  return fixed(n, dp)
}

export function currencySymbol(ccy: string): string {
  switch (ccy?.toUpperCase()) {
    case 'USD': case 'USDT': case 'USDC': return '$'
    case 'EUR': return '€'
    case 'GBP': return '£'
    case 'JPY': return '¥'
    case 'BTC': return '₿'
    default: return ccy ? `${ccy} ` : ''
  }
}

/* -----------------------------------------------------------------------------
   Direction helpers — colour is NEVER the only cue (spec §1.3.3 / A4)
   -------------------------------------------------------------------------- */

export type Direction = 'pos' | 'neg' | 'flat'

export function direction(v: Numish): Direction {
  const n = toNum(v)
  if (n === null || n === 0) return 'flat'
  return n > 0 ? 'pos' : 'neg'
}

/** Semantic class for a directional figure. `flat` is deliberately neutral. */
export function dirClass(v: Numish): string {
  const d = direction(v)
  return d === 'pos' ? 'pos' : d === 'neg' ? 'neg' : 'sec'
}

/** The arrow glyph that accompanies a percentage shown without a sign. */
export function dirArrow(v: Numish): string {
  const d = direction(v)
  return d === 'pos' ? UP : d === 'neg' ? DOWN : ''
}

/* -----------------------------------------------------------------------------
   Time
   -------------------------------------------------------------------------- */

export function clockTime(d: Date | string | number | undefined): string {
  if (d === undefined || d === null) return DASH
  const dt = d instanceof Date ? d : new Date(d)
  if (Number.isNaN(dt.getTime())) return DASH
  return dt.toLocaleTimeString(undefined, { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

export function shortDate(d: Date | string | number | undefined): string {
  if (d === undefined || d === null) return DASH
  const dt = d instanceof Date ? d : new Date(d)
  if (Number.isNaN(dt.getTime())) return DASH
  return dt.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
}

export function dateTime(d: Date | string | number | undefined): string {
  if (d === undefined || d === null) return DASH
  const dt = d instanceof Date ? d : new Date(d)
  if (Number.isNaN(dt.getTime())) return DASH
  return `${dt.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: '2-digit' })} ${clockTime(dt)}`
}

/** "12s old" / "4 min ago" — used by stale badges and meta lines. */
export function age(sinceMs: number | undefined | null, now = Date.now()): string {
  if (sinceMs === undefined || sinceMs === null) return DASH
  const s = Math.max(0, Math.round((now - sinceMs) / 1000))
  if (s < 60) return `${s}s`
  const m = Math.round(s / 60)
  if (m < 60) return `${m} min`
  const h = Math.round(m / 60)
  if (h < 24) return `${h}h`
  return `${Math.round(h / 24)}d`
}

export function relativeTime(d: Date | string | number | undefined): string {
  if (d === undefined || d === null) return DASH
  const dt = d instanceof Date ? d : new Date(d)
  if (Number.isNaN(dt.getTime())) return DASH
  const diff = Date.now() - dt.getTime()
  if (diff < 0) return 'in ' + age(Date.now() - Math.abs(diff))
  return `${age(dt.getTime())} ago`
}

export function duration(ms: Numish): string {
  const n = toNum(ms)
  if (n === null) return DASH
  const s = Math.round(n / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  const rs = s % 60
  if (m < 60) return `${m}m ${rs}s`
  const h = Math.floor(m / 60)
  return `${h}h ${m % 60}m`
}

/* -----------------------------------------------------------------------------
   Back-compat shims — pre-Meridian call sites. Do not add new usages.
   -------------------------------------------------------------------------- */

/** @deprecated use price(instrument, v) */
export function formatPrice(value: Numish, decimals = 2): string {
  const n = toNum(value)
  if (n === null) return DASH
  return fixed(n, decimals)
}

/** @deprecated use qty(instrument, v) */
export function formatSize(value: Numish, decimals = 4): string {
  return formatPrice(value, decimals)
}

/** @deprecated use signed(v) */
export function formatPnl(value: Numish): string {
  return signed(value, 2)
}

/** @deprecated use dirClass(v) */
export function pnlClass(value: Numish): string {
  return dirClass(value)
}
