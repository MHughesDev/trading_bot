import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import {
  AlertTriangle,
  Download,
  Inbox,
  LineChart,
  Plus,
  RotateCw,
  ShieldCheck,
  Table2,
  Zap,
} from 'lucide-react'
import { api } from '@/lib/api'
import { useModeStore } from '@/store/mode'
import {
  flattenPositions,
  usePortfolioSummary,
  type ClassRow,
} from '@/hooks/usePortfolioSummary'
import {
  EQUITY_RANGES,
  fetchEquityCurve,
  fetchFills,
  fetchRisk,
  fetchWorkingOrders,
  isUnavailable,
  type EquityRange,
  type RiskSummary,
} from '@/api/portfolio'
import { assetClassChartColor, assetClassInfo } from '@/lib/assetClass'
import {
  DASH,
  count as fmtCount,
  money,
  pct,
  ratio,
  signedMoney,
  signedPct,
  winRate as fmtWinRate,
  relativeTime,
} from '@/lib/format'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { Button, IconButton } from '@/components/primitives/Button'
import { Badge, Chip, KeyValue, Swatch } from '@/components/primitives/Badge'
import { Label, Money, Pnl, Pct, StatTile } from '@/components/primitives/Num'
import { Segmented, Tabs } from '@/components/primitives/Segmented'
import { Table, IdentityCell, TableScroll, type Column } from '@/components/primitives/Table'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { Tooltip } from '@/components/primitives/Overlay'
import { AllocationBar, AreaChart, ChartTableView, Legend, Meter, MiniBar, Sparkline } from '@/components/charts/Primitives'
import type { Pt } from '@/components/charts/Primitives'

/* =============================================================================
   Spec §4.1 — Dashboard.
   Purpose: answer "where do I stand and what is running" in under five seconds.

   The eight per-asset-class columns are gone. One dense table replaces them,
   with a hero strip carrying the headline numbers, and the totals row
   reconciles to the hero equity figure to the cent.
   ============================================================================= */

type DetailTab = 'classes' | 'positions' | 'fills' | 'orders'
type AllocView = 'equity' | 'cash' | 'risk'

interface AutomationSummary {
  id: string
  kind: 'single_instrument' | 'pipeline'
  account_mode: string
  armed: boolean
  active?: boolean
  created_at: string
  name?: string
  spec?: { asset_class?: string; instrument_id?: string; universe?: string[] }
}

function useAutomations() {
  return useQuery({
    queryKey: ['automations'],
    queryFn: async () => {
      const { data } = await api.get<{ automations: AutomationSummary[] }>('/api/automations')
      return data.automations ?? []
    },
    refetchInterval: 20_000,
  })
}

function useEquityCurve(range: EquityRange, mode: string) {
  return useQuery({
    queryKey: ['equity-curve', range, mode],
    queryFn: () => fetchEquityCurve(range, mode),
    refetchInterval: 60_000,
    staleTime: 30_000,
  })
}

function useFills() {
  return useQuery({ queryKey: ['fills'], queryFn: () => fetchFills({ limit: 100 }), refetchInterval: 20_000 })
}

function useOrders() {
  return useQuery({ queryKey: ['working-orders'], queryFn: () => fetchWorkingOrders({}), refetchInterval: 10_000 })
}

function useRisk(mode: string) {
  return useQuery({ queryKey: ['risk-summary', mode], queryFn: () => fetchRisk(mode), refetchInterval: 20_000 })
}

export function DashboardPage() {
  const navigate = useNavigate()
  const { mode } = useModeStore()
  const s = usePortfolioSummary()
  const [range, setRange] = useState<EquityRange>('1M')
  const [tab, setTab] = useState<DetailTab>('classes')
  const [allocView, setAllocView] = useState<AllocView>('equity')
  const [curveTable, setCurveTable] = useState(false)

  const curve = useEquityCurve(range, mode)
  const fills = useFills()
  const orders = useOrders()
  const risk = useRisk(mode)
  const automations = useAutomations()
  const positions = useMemo(() => flattenPositions(s.rows), [s.rows])

  const fundedRows = s.rows.filter((r) => r.equity !== 0 || r.totalPnl !== 0 || r.open > 0)
  const visibleRows = fundedRows.length ? fundedRows : s.rows

  const maxAlloc = Math.max(0, ...visibleRows.map((r) => r.allocationPct))

  const allocSegments = useMemo(
    () =>
      visibleRows
        .map((r) => ({
          key: r.id,
          label: r.label,
          pct:
            allocView === 'equity'
              ? r.allocationPct
              : allocView === 'cash'
                ? r.cash
                : Math.abs(r.unrealized),
          value: allocView === 'equity' ? r.equity : allocView === 'cash' ? r.cash : Math.abs(r.unrealized),
          color: assetClassChartColor(r.id),
        }))
        .filter((x) => x.pct > 0)
        .sort((a, b) => b.pct - a.pct),
    [visibleRows, allocView],
  )
  const allocTotal = allocSegments.reduce((a, b) => a + b.pct, 0) || 1

  const riskResult = risk.data
  const riskData: RiskSummary | null = riskResult && !isUnavailable(riskResult) ? riskResult : null
  const marginPct = riskData?.marginUsedPct ?? s.marginUsedPct
  const marginLimit = riskData?.marginLimitPct ?? 60
  const withinLimits = riskData ? riskData.withinLimits : marginPct < marginLimit

  const curveData = curve.data
  const curvePoints: Pt[] =
    curveData && !isUnavailable(curveData)
      ? curveData.points.map((p) => ({
          t: p.t,
          v: Number(p.equity),
          mark: p.deposit ? { label: `Deposit · ${money(p.deposit)}` } : undefined,
        }))
      : []

  const classColumns: Column<ClassRow>[] = [
    {
      key: 'class',
      header: 'Asset class',
      align: 'left',
      numeric: false,
      sortValue: (r) => r.label,
      cell: (r) => (
        <span className="sym">
          <Swatch color={assetClassChartColor(r.id)} />
          <span className="tick" style={{ fontWeight: 'var(--w-medium)', fontSize: 'var(--t-12)' }}>
            {r.label}
          </span>
          {r.foreignCurrency && (
            <Tooltip content={`Reports in ${r.currency}; excluded from USD totals and allocation.`}>
              <span className="badge warn">{r.currency}</span>
            </Tooltip>
          )}
        </span>
      ),
    },
    { key: 'equity', header: 'Equity', sortValue: (r) => r.equity, cell: (r) => <Money value={r.equity} ccy={r.currency} /> },
    { key: 'cash', header: 'Cash', sortValue: (r) => r.cash, cell: (r) => <span className="num mut">{money(r.cash, r.currency)}</span> },
    { key: 'unreal', header: 'Unrealized', sortValue: (r) => r.unrealized, cell: (r) => <Pnl value={r.unrealized} bare /> },
    { key: 'total', header: 'Total P&L', sortValue: (r) => r.totalPnl, cell: (r) => <Pnl value={r.totalPnl} bare /> },
    { key: 'win', header: 'Win rate', sortValue: (r) => r.winRate, cell: (r) => (r.trades > 0 ? fmtWinRate(r.winRate) : <span className="mut">{DASH}</span>) },
    { key: 'trades', header: 'Trades', sortValue: (r) => r.trades, cell: (r) => (r.trades > 0 ? fmtCount(r.trades) : <span className="mut">{DASH}</span>) },
    { key: 'open', header: 'Open', sortValue: (r) => r.open, cell: (r) => fmtCount(r.open) },
    {
      key: 'alloc',
      header: 'Allocation',
      sortValue: (r) => r.allocationPct,
      width: 140,
      cell: (r) => (
        <span className="row" style={{ justifyContent: 'flex-end', gap: 'var(--s-2)' }}>
          <MiniBar pct={r.allocationPct} max={maxAlloc} color={assetClassChartColor(r.id)} />
          <span className="num" style={{ width: 52, textAlign: 'right' }}>
            {r.foreignCurrency ? DASH : pct(r.allocationPct)}
          </span>
        </span>
      ),
    },
  ]

  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Portfolio</h1>
        <Chip>All accounts</Chip>
        {!s.reconciles && (
          <Tooltip content="The server's account total does not match the sum of the asset-class rows. This is a data bug worth surfacing, not hiding.">
            <Badge tone="warn">
              <AlertTriangle size={9} aria-hidden />
              Totals off by {money(Math.abs(s.reconcileDelta))}
            </Badge>
          </Tooltip>
        )}
        <div className="spacer" />
        <Segmented
          ariaLabel="Equity curve range"
          value={range}
          onChange={setRange}
          options={EQUITY_RANGES.map((r) => ({ value: r, label: r }))}
        />
        <IconButton label="Refresh portfolio" onClick={s.refresh}>
          <RotateCw size={14} aria-hidden />
        </IconButton>
        <Button size="sm" icon={<Download size={13} aria-hidden />} onClick={() => exportCsv(visibleRows)}>
          Export
        </Button>
        <Button size="sm" variant="primary" icon={<Plus size={13} aria-hidden />} onClick={() => navigate('/terminal')}>
          New order
        </Button>
      </div>

      <div className="dash-body">
        {/* ── HERO STRIP ─────────────────────────────────────────────────── */}
        <Panel className="dash-hero">
          <div className="hero">
            <div>
              <Label>Account equity</Label>
              {s.loading ? (
                <span className="skel" style={{ width: 190, height: 34, marginTop: 4 }} />
              ) : (
                <span className="big num">{money(s.equity, s.currency)}</span>
              )}
              <div className="row" style={{ marginTop: 6, gap: 'var(--s-3)' }}>
                <Badge tone={s.dayPnl >= 0 ? 'pos' : 'neg'}>
                  {s.dayPnl >= 0 ? '▲' : '▼'} {signedPct(s.equity ? (s.dayPnl / s.equity) * 100 : 0)} all time
                </Badge>
                {curvePoints.length > 1 && (
                  <Sparkline points={curvePoints.map((p) => p.v)} ariaLabel="Equity trend" />
                )}
              </div>
            </div>
            <StatTile
              label={mode === 'LIVE' ? 'Day P&L' : 'Open P&L'}
              value={<Pnl value={s.dayPnl} ccy={s.currency} />}
              meta={s.equity ? signedPct((s.dayPnl / s.equity) * 100) : undefined}
            />
            <StatTile
              label="Unrealized"
              value={<Pnl value={s.unrealized} ccy={s.currency} />}
              meta={`${fmtCount(s.openPositions)} open position${s.openPositions === 1 ? '' : 's'}`}
            />
            <StatTile
              label="Realized"
              value={<Pnl value={s.realized} ccy={s.currency} />}
              meta={`${money(s.feesPaid, s.currency)} fees paid`}
            />
            <StatTile
              label="Win rate"
              value={<span className="num">{fmtWinRate(s.winRate)}</span>}
              meta={
                s.rows.some((r) => r.trades > 0)
                  ? `${fmtCount(s.rows.reduce((a, b) => a + b.trades, 0))} trades`
                  : 'No closed trades yet'
              }
            />
            <StatTile
              label="Buying power"
              value={<Money value={s.buyingPower} ccy={s.currency} />}
              meta={`margin used ${pct(s.marginUsedPct)}`}
            />
          </div>
        </Panel>

        {/* ── EQUITY CURVE ───────────────────────────────────────────────── */}
        <Panel className="dash-curve">
          <PanelHeader
            title="Equity curve"
            badges={<Badge tone="neutral">{range}</Badge>}
            actions={
              <div className="row" style={{ gap: 'var(--s-4)' }}>
                {curveData && !isUnavailable(curveData) && (
                  <div className="row" style={{ gap: 'var(--s-4)', fontSize: 'var(--t-12)' }}>
                    <span>
                      <Label>Max drawdown</Label>{' '}
                      <span className="num neg">{curveData.maxDrawdownPct !== undefined ? signedPct(-Math.abs(curveData.maxDrawdownPct)) : DASH}</span>
                    </span>
                    <span>
                      <Label>Sharpe</Label> <span className="num">{ratio(curveData.sharpe)}</span>
                    </span>
                    <span>
                      <Label>Best day</Label> <span className="num pos">{curveData.bestDay ? signedMoney(curveData.bestDay) : DASH}</span>
                    </span>
                  </div>
                )}
                <IconButton
                  label={curveTable ? 'Show chart' : 'Show data table'}
                  active={curveTable}
                  onClick={() => setCurveTable((v) => !v)}
                >
                  {curveTable ? <LineChart size={14} aria-hidden /> : <Table2 size={14} aria-hidden />}
                </IconButton>
              </div>
            }
          />
          <PanelBody flush={curveTable} className={curveTable ? undefined : 'clip'}>
            {curve.isLoading ? (
              <PanelLoading lines={5} />
            ) : curvePoints.length > 1 ? (
              curveTable ? (
                <ChartTableView points={curvePoints} currency={s.currency} caption="Equity curve data" />
              ) : (
                <AreaChart points={curvePoints} currency={s.currency} height={252} label="Account equity" />
              )
            ) : (
              <EmptyState
                icon={<LineChart size={20} aria-hidden />}
                message="No equity history yet"
                detail={
                  isUnavailable(curveData)
                    ? curveData.reason
                    : 'Snapshots appear here once the account has recorded balance history.'
                }
                action={
                  <Button size="sm" onClick={() => navigate('/backtesting')}>
                    Run a backtest instead
                  </Button>
                }
              />
            )}
          </PanelBody>
        </Panel>

        {/* ── ALLOCATION ─────────────────────────────────────────────────── */}
        <Panel className="dash-alloc">
          <PanelHeader
            title="Allocation by asset class"
            actions={
              <Segmented
                ariaLabel="Allocation basis"
                value={allocView}
                onChange={setAllocView}
                options={[
                  { value: 'equity', label: 'Equity' },
                  { value: 'cash', label: 'Cash' },
                  { value: 'risk', label: 'Risk' },
                ]}
              />
            }
          />
          <PanelBody>
            {allocSegments.length === 0 ? (
              <EmptyState message="Nothing allocated yet" detail="Fund an asset class or open a position to see the split." />
            ) : (
              <>
                <AllocationBar
                  segments={allocSegments.map((x) => ({ ...x, pct: (x.pct / allocTotal) * 100 }))}
                />
                <div style={{ height: 'var(--s-4)' }} />
                <Legend
                  items={allocSegments.map((x) => ({
                    key: x.key,
                    label: x.label,
                    color: x.color,
                    value: pct((x.pct / allocTotal) * 100),
                    meta: money(x.value, s.currency, 0),
                  }))}
                />
                {s.excludedCurrencies.length > 0 && (
                  <div className="callout neutral" style={{ marginTop: 'var(--s-4)' }}>
                    <AlertTriangle size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
                    <span>
                      {s.excludedCurrencies.join(', ')} balances are excluded from the USD total until an FX rate
                      is available.
                    </span>
                  </div>
                )}
              </>
            )}
          </PanelBody>
        </Panel>

        {/* ── PORTFOLIO DETAIL ───────────────────────────────────────────── */}
        <Panel className="dash-detail">
          <Tabs
            ariaLabel="Portfolio detail"
            value={tab}
            onChange={setTab}
            tabs={[
              { value: 'classes', label: 'By asset class' },
              { value: 'positions', label: 'Open positions', count: positions.length },
              { value: 'fills', label: 'Recent fills', count: isUnavailable(fills.data) ? undefined : fills.data?.length },
              { value: 'orders', label: 'Orders', count: isUnavailable(orders.data) ? undefined : orders.data?.length },
            ]}
          />
          <PanelBody flush>
            <TableScroll>
              {tab === 'classes' && (
                <Table
                  caption="Portfolio by asset class"
                  columns={classColumns}
                  rows={visibleRows}
                  rowKey={(r) => r.id}
                  loading={s.loading}
                  onRowClick={(r) => navigate(`/trading?class=${r.id}`)}
                  footer={
                    <tr>
                      <td className="l">Total</td>
                      <td className="n">{money(s.equity, s.currency)}</td>
                      <td className="n">{money(s.cash, s.currency)}</td>
                      <td className={`n ${s.unrealized >= 0 ? 'pos' : 'neg'}`}>{signedMoney(s.unrealized, s.currency)}</td>
                      <td className={`n ${s.dayPnl >= 0 ? 'pos' : 'neg'}`}>{signedMoney(s.dayPnl, s.currency)}</td>
                      <td className="n">{fmtWinRate(s.winRate)}</td>
                      <td className="n">{fmtCount(s.rows.reduce((a, b) => a + b.trades, 0))}</td>
                      <td className="n">{fmtCount(s.openPositions)}</td>
                      <td className="n">{pct(100)}</td>
                    </tr>
                  }
                />
              )}

              {tab === 'positions' && (
                <Table
                  caption="Open positions"
                  rows={positions}
                  rowKey={(p) => p.key}
                  loading={s.loading}
                  onRowClick={(p) => navigate(`/terminal/${encodeURIComponent(p.instrumentId)}`)}
                  empty={
                    <EmptyState
                      message="No open positions"
                      detail="Positions opened manually or by an automation appear here with their live mark and unrealized P&L."
                      action={
                        <Button size="sm" variant="primary" onClick={() => navigate('/terminal')}>
                          Open the terminal
                        </Button>
                      }
                    />
                  }
                  columns={[
                    {
                      key: 'inst',
                      header: 'Instrument',
                      align: 'left',
                      numeric: false,
                      sortValue: (p) => p.instrumentId,
                      cell: (p) => (
                        <IdentityCell
                          swatch={<Swatch color={assetClassChartColor(p.assetClass)} />}
                          ticker={p.instrumentId}
                          name={p.assetClassLabel}
                        />
                      ),
                    },
                    {
                      key: 'side',
                      header: 'Side',
                      numeric: false,
                      align: 'right',
                      sortValue: (p) => p.side,
                      cell: (p) => <Badge tone={p.side === 'long' ? 'pos' : 'neg'}>{p.side}</Badge>,
                    },
                    { key: 'size', header: 'Size', sortValue: (p) => Math.abs(p.qty), cell: (p) => <span className="num">{Math.abs(p.qty)}</span> },
                    { key: 'entry', header: 'Entry', sortValue: (p) => p.entry, cell: (p) => <span className="num">{p.entry}</span> },
                    { key: 'mark', header: 'Mark', sortValue: (p) => p.mark ?? 0, cell: (p) => <span className="num">{p.mark ?? DASH}</span> },
                    { key: 'unreal', header: 'Unrealized', sortValue: (p) => p.unrealized, cell: (p) => <Pnl value={p.unrealized} bare /> },
                    { key: 'ret', header: 'Return', sortValue: (p) => p.returnPct ?? 0, cell: (p) => (p.returnPct === null ? <span className="mut">{DASH}</span> : <Pct value={p.returnPct} />) },
                    { key: 'notional', header: 'Notional', sortValue: (p) => p.notional, cell: (p) => <span className="num mut">{money(p.notional, p.currency)}</span> },
                  ]}
                />
              )}

              {tab === 'fills' && (
                <Table
                  caption="Recent fills"
                  rows={isUnavailable(fills.data) ? [] : (fills.data ?? [])}
                  rowKey={(f) => f.id}
                  loading={fills.isLoading}
                  onRowClick={(f) => navigate('/terminal/' + encodeURIComponent(f.instrumentId))}
                  initialSort={{ key: 'time', dir: 'desc' }}
                  empty={
                    <EmptyState
                      icon={<Inbox size={20} aria-hidden />}
                      message="No fills yet"
                      detail={
                        isUnavailable(fills.data)
                          ? fills.data.reason
                          : 'Executions land here the moment an order fills.'
                      }
                      action={
                        <Button size="sm" onClick={() => navigate('/transactions')}>
                          Open account activity
                        </Button>
                      }
                    />
                  }
                  columns={[
                    {
                      key: 'time',
                      header: 'Time',
                      align: 'left',
                      numeric: false,
                      sortValue: (f) => f.ts,
                      cell: (f) => <span className="mono mut">{relativeTime(f.ts)}</span>,
                    },
                    {
                      key: 'inst',
                      header: 'Instrument',
                      align: 'left',
                      numeric: false,
                      sortValue: (f) => f.instrumentId,
                      cell: (f) => (
                        <IdentityCell
                          swatch={<Swatch color={assetClassChartColor(f.assetClass)} />}
                          ticker={f.instrumentId}
                          name={assetClassInfo(f.assetClass).label}
                        />
                      ),
                    },
                    { key: 'side', header: 'Side', numeric: false, cell: (f) => <Badge tone={f.side === 'buy' ? 'pos' : 'neg'}>{f.side}</Badge> },
                    { key: 'qty', header: 'Quantity', sortValue: (f) => Number(f.qty), cell: (f) => <span className="num">{f.qty}</span> },
                    { key: 'price', header: 'Price', cell: (f) => <span className="num">{f.price ?? DASH}</span> },
                    { key: 'value', header: 'Value', cell: (f) => (f.value ? money(f.value) : DASH) },
                    { key: 'status', header: 'Status', numeric: false, cell: (f) => <Badge tone="neutral">{f.status.replace('_', ' ')}</Badge> },
                  ]}
                />
              )}

              {tab === 'orders' && (
                <Table
                  caption="Working orders"
                  rows={isUnavailable(orders.data) ? [] : (orders.data ?? [])}
                  rowKey={(o) => o.id}
                  loading={orders.isLoading}
                  onRowClick={(o) => navigate('/terminal/' + encodeURIComponent(o.instrumentId))}
                  empty={
                    <EmptyState
                      icon={<Inbox size={20} aria-hidden />}
                      message="No working orders"
                      detail={
                        isUnavailable(orders.data)
                          ? orders.data.reason
                          : 'Limit, stop and bracket orders resting at a venue appear here.'
                      }
                      action={
                        <Button size="sm" variant="primary" onClick={() => navigate('/terminal')}>
                          Place an order
                        </Button>
                      }
                    />
                  }
                  columns={[
                    {
                      key: 'inst',
                      header: 'Instrument',
                      align: 'left',
                      numeric: false,
                      sortValue: (o) => o.instrumentId,
                      cell: (o) => (
                        <IdentityCell
                          swatch={<Swatch color={assetClassChartColor(o.assetClass)} />}
                          ticker={o.instrumentId}
                          name={assetClassInfo(o.assetClass).label}
                        />
                      ),
                    },
                    { key: 'side', header: 'Side', numeric: false, cell: (o) => <Badge tone={o.side === 'buy' ? 'pos' : 'neg'}>{o.side}</Badge> },
                    { key: 'type', header: 'Type', numeric: false, cell: (o) => <span className="sec">{o.type.replace('_', ' ')}</span> },
                    { key: 'qty', header: 'Quantity', sortValue: (o) => Number(o.qty), cell: (o) => <span className="num">{o.qty}</span> },
                    { key: 'filled', header: 'Filled', cell: (o) => <span className="num">{o.filledQty}</span> },
                    { key: 'price', header: 'Price', cell: (o) => <span className="num">{o.price ?? DASH}</span> },
                    { key: 'status', header: 'Status', numeric: false, cell: (o) => <Badge tone="accent">{o.status.replace('_', ' ')}</Badge> },
                    {
                      key: 'placed',
                      header: 'Placed',
                      numeric: false,
                      sortValue: (o) => o.ts,
                      cell: (o) => <span className="mut">{relativeTime(o.ts)}</span>,
                    },
                  ]}
                />
              )}
            </TableScroll>
          </PanelBody>
        </Panel>

        {/* ── ACTIVE AUTOMATIONS ─────────────────────────────────────────── */}
        <Panel className="dash-autos">
          <PanelHeader
            title="Active automations"
            actions={
              <Button size="xs" variant="ghost" onClick={() => navigate('/automations')}>
                Manage
              </Button>
            }
          />
          <PanelBody flush className="clip" style={{ overflowY: 'auto' }}>
            {automations.isLoading ? (
              <div style={{ padding: 'var(--s-4)' }}>
                <PanelLoading lines={3} />
              </div>
            ) : automations.data && automations.data.length > 0 ? (
              automations.data.slice(0, 6).map((a) => {
                const cls = a.spec?.asset_class
                const markets = a.spec?.instrument_id ?? (a.spec?.universe?.length ? `${a.spec.universe.length} symbols` : DASH)
                return (
                  <button
                    key={a.id}
                    type="button"
                    onClick={() => navigate(`/automations?open=${a.id}`)}
                    style={{
                      display: 'grid',
                      gridTemplateColumns: '1fr auto',
                      gap: 'var(--s-3)',
                      alignItems: 'center',
                      padding: '9px var(--s-4)',
                      borderBottom: '1px solid var(--line-hairline)',
                      width: '100%',
                      textAlign: 'left',
                      background: 'transparent',
                    }}
                  >
                    <div style={{ minWidth: 0 }}>
                      <div className="row" style={{ gap: 'var(--s-2)' }}>
                        <span style={{ fontSize: 'var(--t-13)', fontWeight: 'var(--w-semibold)' }} className="truncate-1">
                          {a.name ?? a.kind.replace(/_/g, ' ')}
                        </span>
                        <Badge tone={a.armed ? 'accent' : 'neutral'}>{a.armed ? 'Running' : 'Stopped'}</Badge>
                      </div>
                      <div className="mut" style={{ fontSize: 'var(--t-11)', marginTop: 2 }}>
                        {cls ? assetClassInfo(cls).label : 'Multi-class'} {'·'} {markets} {'·'} {relativeTime(a.created_at)}
                      </div>
                    </div>
                    <Badge tone={a.account_mode === 'live' ? 'pos' : 'warn'}>{a.account_mode}</Badge>
                  </button>
                )
              })
            ) : (
              <EmptyState
                icon={<Zap size={20} aria-hidden />}
                message="Nothing is running"
                detail="An automation watches a market and acts on a strategy without you at the desk."
                action={
                  <Button size="sm" variant="primary" onClick={() => navigate('/automations')}>
                    Create an automation
                  </Button>
                }
              />
            )}
          </PanelBody>
        </Panel>

        {/* ── RISK & EXPOSURE ────────────────────────────────────────────── */}
        <Panel className="dash-risk">
          <PanelHeader
            title="Risk &amp; exposure"
            actions={
              <Badge tone={withinLimits ? 'pos' : 'warn'}>
                <ShieldCheck size={9} aria-hidden />
                {withinLimits ? 'Within limits' : `${riskData?.breaches.length ?? 1} breach`}
              </Badge>
            }
          />
          <PanelBody>
            <KeyValue
              k="Gross exposure"
              v={money(riskData ? riskData.grossExposure : positions.reduce((a, p) => a + Math.abs(p.notional), 0), s.currency)}
            />
            <KeyValue
              k="Net exposure"
              v={
                <>
                  {money(riskData ? riskData.netExposure : positions.reduce((a, p) => a + p.notional, 0), s.currency)}{' '}
                  <span className="mut">
                    {pct(riskData ? riskData.netExposurePctOfEquity : s.equity ? (positions.reduce((a, p) => a + p.notional, 0) / s.equity) * 100 : 0)} of equity
                  </span>
                </>
              }
            />
            <KeyValue k="Cash" v={money(s.cash, s.currency)} />
            <KeyValue k="Fees paid" v={money(s.feesPaid, s.currency)} />
            <KeyValue
              k="Largest position"
              v={
                riskData?.concentrationTopPct != null ? (
                  <>
                    {pct(riskData.concentrationTopPct, 0)} <span className="mut">of gross</span>
                  </>
                ) : (
                  <span className="mut">{DASH}</span>
                )
              }
            />
            <KeyValue
              k="1-day VaR (95%)"
              v={
                riskData?.var95OneDay ? (
                  money(riskData.var95OneDay, s.currency)
                ) : (
                  <Tooltip content="A one-day VaR needs a return history the platform does not retain yet.">
                    <span className="mut">Not computed</span>
                  </Tooltip>
                )
              }
            />
            <div className="hr" />
            <div className="between" style={{ marginBottom: 6 }}>
              <Label>Margin utilisation</Label>
              <span className="num" style={{ fontSize: 'var(--t-12)' }}>
                {pct(marginPct)} of {pct(marginLimit, 0)} limit
              </span>
            </div>
            <Meter
              value={marginPct}
              max={marginLimit}
              tone={marginPct < marginLimit * 0.66 ? 'pos' : marginPct < marginLimit ? 'warn' : 'neg'}
              ariaLabel="Margin utilisation against the account limit"
            />
            {riskData?.breaches.map((b) => (
              <div key={b.label} className="callout neg" style={{ marginTop: 'var(--s-3)' }}>
                <AlertTriangle size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
                <span>
                  <strong>{b.label}.</strong> {b.detail}
                </span>
              </div>
            ))}
            {s.error && <ErrorState message={s.error} onRetry={s.refresh} />}
          </PanelBody>
        </Panel>
      </div>
    </>
  )
}

function exportCsv(rows: ClassRow[]) {
  const head = ['Asset class', 'Equity', 'Cash', 'Unrealized', 'Total P&L', 'Win rate', 'Trades', 'Open', 'Allocation %', 'Currency']
  const body = rows.map((r) => [
    r.label,
    r.equity,
    r.cash,
    r.unrealized,
    r.totalPnl,
    r.winRate,
    r.trades,
    r.open,
    r.allocationPct.toFixed(2),
    r.currency,
  ])
  const csv = [head, ...body].map((line) => line.join(',')).join('\n')
  const url = URL.createObjectURL(new Blob([csv], { type: 'text/csv' }))
  const a = document.createElement('a')
  a.href = url
  a.download = `portfolio-${new Date().toISOString().slice(0, 10)}.csv`
  a.click()
  URL.revokeObjectURL(url)
}
