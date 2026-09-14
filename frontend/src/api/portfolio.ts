/* =============================================================================
   Portfolio / execution data contracts.

   All of these are served by the Rust platform. Where a figure genuinely cannot
   be computed yet (a one-day VaR needs a return history the platform does not
   retain), the server returns `null` and the screen says so rather than
   inventing a number.
   ============================================================================= */

import { api, alertsApi, executionApi, portfolioApi } from '@/lib/api'
import type { AssetClass } from '@/lib/format'

export interface Unavailable {
  unavailable: true
  /** Why, in plain language, for the empty state to show. */
  reason: string
}

export type Result<T> = T | Unavailable

export function isUnavailable<T>(v: Result<T> | undefined | null): v is Unavailable {
  return !!v && typeof v === 'object' && (v as Unavailable).unavailable === true
}

function unavailable<T>(reason: string): Result<T> {
  return { unavailable: true, reason }
}

/* -----------------------------------------------------------------------------
   Equity curve — GET /api/portfolio/equity-curve
   Fed by the platform's sampler, which writes one snapshot per asset class per
   interval (EQUITY_SAMPLE_SECS, default 60s).
   -------------------------------------------------------------------------- */

export interface EquityPoint {
  /** Unix seconds. */
  t: number
  /** Account equity at that instant, decimal string. */
  equity: string
  /** External cash movement at that instant, decimal string, signed. */
  deposit?: string
}

export interface EquityCurve {
  points: EquityPoint[]
  currency: string
  maxDrawdownPct?: number
  sharpe?: number
  bestDay?: string
  worstDay?: string
}

interface EquityCurveWire {
  points: EquityPoint[]
  currency: string
  max_drawdown_pct?: number
  sharpe?: number
  best_day?: string
  worst_day?: string
}

export type EquityRange = '1D' | '1W' | '1M' | '3M' | 'YTD' | 'ALL'
export const EQUITY_RANGES: EquityRange[] = ['1D', '1W', '1M', '3M', 'YTD', 'ALL']

export async function fetchEquityCurve(range: EquityRange, mode: string): Promise<Result<EquityCurve>> {
  try {
    const { data } = await portfolioApi.equityCurve({ range, mode })
    const wire = data as EquityCurveWire
    if (!Array.isArray(wire?.points) || wire.points.length === 0) {
      return unavailable<EquityCurve>(
        'No equity has been recorded for this range yet. The platform samples account equity every minute; a new account takes a few minutes to draw a curve.',
      )
    }
    return {
      points: wire.points,
      currency: wire.currency ?? 'USD',
      maxDrawdownPct: wire.max_drawdown_pct,
      sharpe: wire.sharpe,
      bestDay: wire.best_day,
      worstDay: wire.worst_day,
    }
  } catch {
    return unavailable<EquityCurve>('The portfolio service did not answer. Equity history will appear once it is reachable.')
  }
}

/* -----------------------------------------------------------------------------
   Open positions — GET /api/portfolio/positions
   -------------------------------------------------------------------------- */

export interface OpenPosition {
  instrumentId: string
  assetClass: string
  currency: string
  side: 'long' | 'short'
  size: string
  entry: string
  mark: string | null
  unrealized: string
  notional: string
  returnPct: number | null
}

interface PositionWire {
  instrument_id: string
  asset_class: string
  currency: string
  side: 'long' | 'short'
  size: string
  entry: string
  mark: string | null
  unrealized: string
  notional: string
  return_pct: number | null
}

export async function fetchPositions(): Promise<OpenPosition[]> {
  try {
    const { data } = await portfolioApi.positions()
    const rows = (data as { positions?: PositionWire[] })?.positions ?? []
    return rows.map((p) => ({
      instrumentId: p.instrument_id,
      assetClass: p.asset_class,
      currency: p.currency,
      side: p.side,
      size: p.size,
      entry: p.entry,
      mark: p.mark,
      unrealized: p.unrealized,
      notional: p.notional,
      returnPct: p.return_pct,
    }))
  } catch {
    return []
  }
}

/* -----------------------------------------------------------------------------
   Fills — GET /api/execution/fills
   -------------------------------------------------------------------------- */

export interface Fill {
  id: string
  ts: string
  instrumentId: string
  assetClass: AssetClass | string
  side: 'buy' | 'sell'
  qty: string
  price: string | null
  value: string | null
  status: string
}

interface FillWire {
  id: string
  ts: string
  instrument_id: string
  asset_class: string
  side: string
  qty: string
  price: string | null
  value: string | null
  status: string
}

export async function fetchFills(params: { limit?: number; instrument?: string } = {}): Promise<Result<Fill[]>> {
  try {
    const { data } = await executionApi.fills(params)
    const rows = (data as { fills?: FillWire[] })?.fills ?? []
    if (rows.length === 0) return unavailable<Fill[]>('No fills recorded yet. Executions appear here the moment an order fills.')
    return rows.map((f) => ({
      id: f.id,
      ts: f.ts,
      instrumentId: f.instrument_id,
      assetClass: f.asset_class,
      side: f.side === 'sell' ? 'sell' : 'buy',
      qty: f.qty,
      price: f.price,
      value: f.value,
      status: f.status,
    }))
  } catch {
    return unavailable<Fill[]>('The execution service did not answer.')
  }
}

/* -----------------------------------------------------------------------------
   Working orders — GET /api/execution/orders
   -------------------------------------------------------------------------- */

export interface WorkingOrder {
  id: string
  ts: string
  instrumentId: string
  assetClass: string
  side: 'buy' | 'sell'
  type: string
  qty: string
  filledQty: string
  price: string | null
  status: string
}

interface OrderWire {
  order_id: string
  instrument_id: string
  asset_class: string
  side: string
  order_type: string
  qty: string
  filled_qty: string
  avg_fill_price: string | null
  status: string
  created_at: string
}

export async function fetchWorkingOrders(
  params: { instrument?: string; limit?: number } = {},
): Promise<Result<WorkingOrder[]>> {
  try {
    const { data } = await executionApi.orders(params)
    const rows = (data as { orders?: OrderWire[] })?.orders ?? []
    if (rows.length === 0) return unavailable<WorkingOrder[]>('No orders are resting at a venue.')
    return rows.map((o) => ({
      id: o.order_id,
      ts: o.created_at,
      instrumentId: o.instrument_id,
      assetClass: o.asset_class,
      side: o.side === 'sell' ? 'sell' : 'buy',
      type: o.order_type,
      qty: o.qty,
      filledQty: o.filled_qty,
      price: o.avg_fill_price,
      status: o.status,
    }))
  } catch {
    return unavailable<WorkingOrder[]>('The execution service did not answer.')
  }
}

export async function cancelOrder(id: string): Promise<boolean> {
  try {
    await executionApi.cancel(id)
    return true
  } catch {
    return false
  }
}

/* -----------------------------------------------------------------------------
   Risk & exposure — GET /api/risk/summary
   -------------------------------------------------------------------------- */

export interface RiskSummary {
  grossExposure: string
  netExposure: string
  netExposurePctOfEquity: number
  var95OneDay: string | null
  marginUsedPct: number
  marginLimitPct: number
  concentrationTopPct: number | null
  withinLimits: boolean
  breaches: { label: string; detail: string }[]
  killSwitchTripped: boolean
}

interface RiskWire {
  gross_exposure: string
  net_exposure: string
  net_exposure_pct_of_equity: number
  var95_one_day: string | null
  margin_used_pct: number
  margin_limit_pct: number
  concentration_top_pct: number | null
  within_limits: boolean
  breaches: { label: string; detail: string }[]
  kill_switch_tripped: boolean
}

export async function fetchRisk(mode: string): Promise<Result<RiskSummary>> {
  try {
    const { data } = await portfolioApi.risk({ mode })
    const w = data as RiskWire
    if (!w || typeof w.gross_exposure !== 'string') {
      return unavailable<RiskSummary>('The risk engine returned nothing for this account.')
    }
    return {
      grossExposure: w.gross_exposure,
      netExposure: w.net_exposure,
      netExposurePctOfEquity: w.net_exposure_pct_of_equity,
      var95OneDay: w.var95_one_day,
      marginUsedPct: w.margin_used_pct,
      marginLimitPct: w.margin_limit_pct,
      concentrationTopPct: w.concentration_top_pct,
      withinLimits: w.within_limits,
      breaches: w.breaches ?? [],
      killSwitchTripped: w.kill_switch_tripped,
    }
  } catch {
    return unavailable<RiskSummary>('The risk service did not answer.')
  }
}

/* -----------------------------------------------------------------------------
   Order book — streamed over `ui.orderbook.snapshot`. No REST fallback by
   design: a book is a stream, and a stale snapshot of one is worse than none.
   -------------------------------------------------------------------------- */

export interface BookLevelView {
  price: string
  size: string
  total: string
}

export interface Trade {
  ts: number
  price: string
  size: string
  side: 'buy' | 'sell'
}

/* -----------------------------------------------------------------------------
   Price alerts — /api/alerts. Evaluated server-side on the platform's sampler
   tick, so an alert fires with the tab closed.
   -------------------------------------------------------------------------- */

export interface PriceAlert {
  id: string
  instrumentId: string
  kind: 'price_above' | 'price_below' | 'pct_change' | 'indicator_cross'
  value: string
  note?: string
  active: boolean
  createdAt: string
  triggeredAt?: string
  triggeredPrice?: string
  channels: string[]
}

export async function fetchAlerts(): Promise<PriceAlert[]> {
  const { data } = await alertsApi.list()
  return ((data as { alerts?: PriceAlert[] })?.alerts ?? []).map((a) => ({
    ...a,
    channels: a.channels ?? ['inapp'],
  }))
}

export async function createAlert(input: {
  instrumentId: string
  kind: PriceAlert['kind']
  value: string
  note?: string
}): Promise<void> {
  await alertsApi.create({ ...input, channels: ['inapp'] })
}

export async function deleteAlert(id: string): Promise<void> {
  await alertsApi.remove(id)
}

/* -----------------------------------------------------------------------------
   Instrument metadata — drives per-instrument decimal precision (spec N2).
   -------------------------------------------------------------------------- */

export interface InstrumentRow {
  id: string
  symbol: string
  name?: string
  asset_class?: string
  venue?: string
  price_dp?: number
  qty_dp?: number
  quote?: string
  tick_size?: string
}

export async function fetchInstruments(): Promise<InstrumentRow[]> {
  try {
    const { data } = await api.get<{ assets: InstrumentRow[] } | InstrumentRow[]>('/api/assets')
    if (Array.isArray(data)) return data
    if (Array.isArray((data as { assets?: InstrumentRow[] })?.assets)) {
      return (data as { assets: InstrumentRow[] }).assets
    }
    return []
  } catch {
    return []
  }
}
