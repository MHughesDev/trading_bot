import { useMemo } from 'react'
import { useQuery } from '@tanstack/react-query'
import { api } from '@/lib/api'
import { useModeStore, type TradingMode } from '@/store/mode'
import { ASSET_CLASSES, assetClassInfo, type PlatformAssetClass } from '@/lib/assetClass'

/* =============================================================================
   The dashboard rollup, reshaped once, consumed everywhere.

   MUST (spec §4.1): the hero equity figure equals the sum of the table's
   Equity column and the P&L figure equals the sum of the P&L column. Both are
   computed here from the same array so they cannot drift; if the server totals
   disagree with the row sum, `reconciles` goes false and the UI SURFACES it
   rather than hiding it.
   ============================================================================= */

export interface RollupVenue {
  venue: string
  realized_pnl_usd: string
  unrealized_pnl_usd: string
  win_rate: number
  trade_count: number
}

export interface RollupPosition {
  instrument_id: string
  quantity: string
  average_entry_price: string
  mark_price: string | null
  unrealized_pnl: string
  notional: string
}

export interface RollupAccount {
  currency: string
  cash: string
  equity: string
  used_margin: string
  free_collateral: string
  fees_paid: string
  open_positions: number
  positions: RollupPosition[]
}

export interface RollupTile {
  asset_class: string
  realized_pnl_usd: string
  unrealized_pnl_usd: string
  win_rate: number
  venues: RollupVenue[]
  account?: RollupAccount
}

export interface RollupTotals {
  equity_usd: string
  cash_usd: string
  realized_pnl_usd: string
  unrealized_pnl_usd: string
  fees_paid_usd: string
  open_positions: number
  excluded_currencies: string[]
}

export interface DashboardRollup {
  mode: string
  realized_pnl_usd: string
  unrealized_pnl_usd: string
  win_rate: number
  by_asset_class: RollupTile[]
  account_totals?: RollupTotals
}

/** One row of the dashboard's asset-class table. */
export interface ClassRow {
  id: PlatformAssetClass
  label: string
  slot: number | null
  equity: number
  cash: number
  usedMargin: number
  freeCollateral: number
  feesPaid: number
  realized: number
  unrealized: number
  totalPnl: number
  winRate: number
  trades: number
  open: number
  /** Share of total equity, 0-100. */
  allocationPct: number
  currency: string
  positions: RollupPosition[]
  /** The class reports in a currency other than the account base. */
  foreignCurrency: boolean
}

export interface PortfolioSummary {
  loading: boolean
  error: string | null
  refresh: () => void
  rollup: DashboardRollup | null
  rows: ClassRow[]
  equity: number
  cash: number
  realized: number
  unrealized: number
  /** Total P&L. The platform does not bucket by day yet — see api/portfolio.ts. */
  dayPnl: number
  winRate: number
  openPositions: number
  feesPaid: number
  buyingPower: number
  marginUsedPct: number
  currency: string
  /** False when the server totals disagree with the sum of the rows. */
  reconciles: boolean
  reconcileDelta: number
  excludedCurrencies: string[]
}

/** Currencies that are the USD base. Must match the server's own list, or the
 *  hero figure and the totals row disagree and the reconcile check fires. */
const BASE_CURRENCIES = new Set(['USD', 'USDC', 'USDT'])

const n = (v: string | number | null | undefined): number => {
  if (v === null || v === undefined) return 0
  const x = typeof v === 'number' ? v : Number(v)
  return Number.isFinite(x) ? x : 0
}

export function useDashboardRollup(mode: TradingMode) {
  return useQuery({
    queryKey: ['dashboard-rollup', mode],
    queryFn: async () => {
      const { data } = await api.get<DashboardRollup>('/api/dashboard/rollup', { params: { mode } })
      return data
    },
    refetchInterval: 15_000,
    staleTime: 5_000,
  })
}

export function usePortfolioSummary(): PortfolioSummary {
  const mode = useModeStore((s) => s.mode)
  const q = useDashboardRollup(mode)
  const rollup = q.data ?? null

  return useMemo(() => {
    const tiles = rollup?.by_asset_class ?? []
    const byId = new Map(tiles.map((t) => [t.asset_class, t]))

    // Declaration order is display order — the same order on every screen.
    const raw = ASSET_CLASSES.map((meta) => {
      const tile = byId.get(meta.id)
      const acct = tile?.account
      const trades = (tile?.venues ?? []).reduce((s, v) => s + (v.trade_count ?? 0), 0)
      const realized = n(tile?.realized_pnl_usd)
      const unrealized = n(tile?.unrealized_pnl_usd)
      return {
        id: meta.id,
        label: meta.label,
        slot: meta.slot,
        equity: n(acct?.equity),
        cash: n(acct?.cash),
        usedMargin: n(acct?.used_margin),
        freeCollateral: n(acct?.free_collateral),
        feesPaid: n(acct?.fees_paid),
        realized,
        unrealized,
        totalPnl: realized + unrealized,
        winRate: tile?.win_rate ?? 0,
        trades,
        open: acct?.open_positions ?? 0,
        allocationPct: 0,
        currency: acct?.currency ?? 'USD',
        positions: acct?.positions ?? [],
        foreignCurrency: !BASE_CURRENCIES.has(acct?.currency ?? 'USD'),
      } satisfies ClassRow
    })

    // Allocation is a share of the base-currency equity only — a class reporting
    // in ETH cannot be summed into a USD denominator without a rate.
    const baseEquity = raw.reduce((s, r) => s + (r.foreignCurrency ? 0 : r.equity), 0)
    const rows = raw.map((r) => ({
      ...r,
      allocationPct: baseEquity > 0 && !r.foreignCurrency ? (r.equity / baseEquity) * 100 : 0,
    }))

    const totals = rollup?.account_totals
    const serverEquity = totals ? n(totals.equity_usd) : baseEquity
    const rowEquity = baseEquity
    const delta = Math.abs(serverEquity - rowEquity)

    const realized = rows.reduce((s, r) => s + r.realized, 0)
    const unrealized = rows.reduce((s, r) => s + r.unrealized, 0)
    const cash = rows.reduce((s, r) => s + (r.foreignCurrency ? 0 : r.cash), 0)
    const usedMargin = rows.reduce((s, r) => s + (r.foreignCurrency ? 0 : r.usedMargin), 0)
    const free = rows.reduce((s, r) => s + (r.foreignCurrency ? 0 : r.freeCollateral), 0)
    const traded = rows.filter((r) => r.trades > 0)
    const winRate = traded.length
      ? traded.reduce((s, r) => s + r.winRate * r.trades, 0) / traded.reduce((s, r) => s + r.trades, 0)
      : (rollup?.win_rate ?? 0)

    return {
      loading: q.isLoading,
      error: q.error ? 'Could not load the portfolio rollup.' : null,
      refresh: () => void q.refetch(),
      rollup,
      rows,
      equity: rowEquity,
      cash,
      realized,
      unrealized,
      dayPnl: realized + unrealized,
      winRate,
      openPositions: rows.reduce((s, r) => s + r.open, 0),
      feesPaid: rows.reduce((s, r) => s + (r.foreignCurrency ? 0 : r.feesPaid), 0),
      buyingPower: free,
      marginUsedPct: rowEquity > 0 ? (usedMargin / rowEquity) * 100 : 0,
      currency: 'USD',
      reconciles: delta < 0.01,
      reconcileDelta: serverEquity - rowEquity,
      excludedCurrencies: totals?.excluded_currencies ?? [],
    }
  }, [rollup, q.isLoading, q.error, q])
}

/** Every open position across every class, flattened for the positions tab. */
export function flattenPositions(rows: ClassRow[]) {
  return rows.flatMap((r) =>
    r.positions.map((p) => {
      const qty = n(p.quantity)
      const entry = n(p.average_entry_price)
      const mark = p.mark_price === null ? null : n(p.mark_price)
      const cost = Math.abs(qty) * entry
      return {
        key: `${r.id}:${p.instrument_id}`,
        instrumentId: p.instrument_id,
        assetClass: r.id,
        assetClassLabel: assetClassInfo(r.id).label,
        slot: r.slot,
        side: qty >= 0 ? ('long' as const) : ('short' as const),
        qty,
        entry,
        mark,
        unrealized: n(p.unrealized_pnl),
        notional: n(p.notional),
        returnPct: cost > 0 ? (n(p.unrealized_pnl) / cost) * 100 : null,
        currency: r.currency,
      }
    }),
  )
}

export type FlatPosition = ReturnType<typeof flattenPositions>[number]
