import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'
import { Ban, Inbox, Zap } from 'lucide-react'
import { api, paperApi, type PaperInstrumentActivity } from '@/lib/api'
import { cancelOrder } from '@/api/portfolio'
import { useToast } from '@/hooks/useToast'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useModeStore } from '@/store/mode'
import { assetClassChartColor } from '@/lib/assetClass'
import { DASH, money, price as fmtPrice, qty as fmtQty, clockTime, relativeTime } from '@/lib/format'
import { Panel, PanelBody } from '@/components/primitives/Panel'
import { Badge, Swatch } from '@/components/primitives/Badge'
import { Label, Pnl, Pct } from '@/components/primitives/Num'
import { Button } from '@/components/primitives/Button'
import { Tabs } from '@/components/primitives/Segmented'
import { Table, IdentityCell, TableScroll } from '@/components/primitives/Table'
import { EmptyState } from '@/components/primitives/States'
import { Tape } from './OrderBook'
import type { PlatformAssetClass } from '@/lib/assetClass'

/* =============================================================================
   Spec §4.2 — the dock beneath the chart.
   Tabs: Positions · Open orders · Fills · Automations · Time & sales.
   Net exposure and margin used ride in the tab bar.

   Positions columns (spec MUST):
   Instrument · Side · Size · Entry · Mark · Unrealized · Return · Liq. price ·
   Stop · Source. `Source` is a badge — neutral for Manual, accent for an
   automation, and it links to that automation.
   ============================================================================= */

type DockTab = 'positions' | 'orders' | 'fills' | 'automations' | 'tape'

interface PaperOrderView {
  order_id?: string
  id?: string
  instrument_id?: string
  side: string
  order_type?: string
  qty: string
  filled_qty?: string
  limit_price?: string | null
  status: string
  created_at?: string
  ts?: string
}

function useActivity(instrument: string | undefined) {
  return useQuery({
    queryKey: ['paper-activity', instrument],
    enabled: !!instrument,
    queryFn: async () => {
      const { data } = await paperApi.instrumentActivity(instrument!)
      return data as PaperInstrumentActivity
    },
    refetchInterval: 4000,
  })
}

function useInstrumentAutomations(instrument: string | undefined) {
  return useQuery({
    queryKey: ['automations'],
    queryFn: async () => {
      const { data } = await api.get<{ automations: Record<string, unknown>[] }>('/api/automations')
      return data.automations ?? []
    },
    enabled: !!instrument,
    refetchInterval: 20_000,
  })
}

export function TerminalDock({
  instrument,
  assetClass,
  markPrice,
  className,
}: {
  instrument: string
  assetClass: PlatformAssetClass
  markPrice: number | null
  className?: string
}) {
  const [tab, setTab] = useState<DockTab>('positions')
  const navigate = useNavigate()
  const toast = useToast()
  const qc = useQueryClient()
  const cancel = useMutation({
    mutationFn: (id: string) => cancelOrder(id),
    onSuccess: (ok, id) => {
      toast({
        title: ok ? 'Order cancelled' : 'Cancel refused',
        description: ok ? undefined : `The venue would not cancel ${id}.`,
        variant: ok ? 'default' : 'error',
      })
      void qc.invalidateQueries({ queryKey: ['paper-activity', instrument] })
    },
  })
  const { mode } = useModeStore()
  const activity = useActivity(instrument)
  const automations = useInstrumentAutomations(instrument)

  const position = activity.data?.position ?? null
  const orders = (activity.data?.orders ?? []) as unknown as PaperOrderView[]
  const working = orders.filter((o) => ['new', 'open', 'partially_filled', 'accepted'].includes(o.status))
  const fills = orders.filter((o) => ['filled', 'partially_filled'].includes(o.status))

  const posQty = position ? Number(position.quantity ?? 0) : 0
  const entry = position ? Number(position.average_entry_price ?? 0) : 0
  const unreal = markPrice != null && posQty !== 0 ? (markPrice - entry) * posQty : 0
  const notional = markPrice != null ? Math.abs(posQty) * markPrice : 0
  const related = automations.data?.filter((a) => {
    const spec = (a.spec ?? {}) as { instrument_id?: string; universe?: string[] }
    return spec.instrument_id === instrument || spec.universe?.includes(instrument)
  }) ?? []

  return (
    <Panel className={className}>
      <Tabs
        ariaLabel="Terminal dock"
        value={tab}
        onChange={setTab}
        tabs={[
          { value: 'positions', label: 'Positions', count: position && posQty !== 0 ? 1 : 0 },
          { value: 'orders', label: 'Open orders', count: working.length },
          { value: 'fills', label: 'Fills', count: fills.length },
          { value: 'automations', label: 'Automations', count: related.length },
          { value: 'tape', label: 'Time & sales' },
        ]}
        right={
          <span className="row" style={{ gap: 'var(--s-4)', paddingRight: 'var(--s-4)', fontSize: 'var(--t-12)' }}>
            <span>
              <Label>Net exposure</Label>{' '}
              <span className="num">{markPrice != null ? money(posQty * markPrice) : DASH}</span>
            </span>
            <span>
              <Label>Notional</Label> <span className="num">{money(notional)}</span>
            </span>
          </span>
        }
      />
      <PanelBody flush style={{ overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
        {tab === 'positions' && (
          <TableScroll>
            <Table
              caption={`Open position in ${instrument}`}
              rows={position && posQty !== 0 ? [position] : []}
              rowKey={() => instrument}
              loading={activity.isLoading}
              empty={
                <EmptyState
                  message={`No position in ${instrument}`}
                  detail="Fills from this ticket or from an automation open a position here."
                />
              }
              columns={[
                {
                  key: 'inst',
                  header: 'Instrument',
                  align: 'left',
                  numeric: false,
                  cell: () => (
                    <IdentityCell
                      swatch={<Swatch color={assetClassChartColor(assetClass)} />}
                      ticker={instrument.split('-')[0]}
                      name={instrument}
                    />
                  ),
                },
                {
                  key: 'side',
                  header: 'Side',
                  numeric: false,
                  cell: () => <Badge tone={posQty >= 0 ? 'pos' : 'neg'}>{posQty >= 0 ? 'Long' : 'Short'}</Badge>,
                },
                { key: 'size', header: 'Size', cell: () => fmtQty(instrument, Math.abs(posQty)) },
                { key: 'entry', header: 'Entry', cell: () => fmtPrice(instrument, entry) },
                { key: 'mark', header: 'Mark', cell: () => (markPrice != null ? fmtPrice(instrument, markPrice) : DASH) },
                { key: 'unreal', header: 'Unrealized', cell: () => <Pnl value={unreal} bare /> },
                {
                  key: 'ret',
                  header: 'Return',
                  cell: () =>
                    entry > 0 && markPrice != null ? (
                      <Pct value={((markPrice - entry) / entry) * 100 * (posQty >= 0 ? 1 : -1)} />
                    ) : (
                      <span className="mut">{DASH}</span>
                    ),
                },
                { key: 'liq', header: 'Liq. price', cell: () => <span className="mut">{DASH}</span> },
                { key: 'stop', header: 'Stop', cell: () => <span className="mut">{DASH}</span> },
                {
                  key: 'src',
                  header: 'Source',
                  numeric: false,
                  cell: () =>
                    related.length ? (
                      <Badge tone="accent" to={`/automations?open=${related[0].id as string}`}>
                        Automation
                      </Badge>
                    ) : (
                      <Badge tone="neutral">Manual</Badge>
                    ),
                },
              ]}
            />
          </TableScroll>
        )}

        {tab === 'orders' && (
          <TableScroll>
            <Table
              caption={`Working orders in ${instrument}`}
              rows={working}
              rowKey={(o) => o.order_id ?? o.id ?? String(o.created_at)}
              loading={activity.isLoading}
              empty={
                <EmptyState
                  icon={<Inbox size={20} aria-hidden />}
                  message="No working orders"
                  detail="Limit, stop and bracket orders resting at the venue appear here until they fill or cancel."
                />
              }
              columns={[
                {
                  key: 'time',
                  header: 'Placed',
                  align: 'left',
                  numeric: false,
                  cell: (o) => <span className="mono mut">{clockTime(o.created_at ?? o.ts)}</span>,
                },
                { key: 'side', header: 'Side', numeric: false, cell: (o) => <Badge tone={o.side === 'buy' ? 'pos' : 'neg'}>{o.side}</Badge> },
                { key: 'type', header: 'Type', numeric: false, cell: (o) => <span className="sec">{(o.order_type ?? 'market').replace('_', ' ')}</span> },
                { key: 'qty', header: 'Qty', cell: (o) => fmtQty(instrument, Number(o.qty)) },
                { key: 'filled', header: 'Filled', cell: (o) => fmtQty(instrument, Number(o.filled_qty ?? 0)) },
                { key: 'price', header: 'Limit', cell: (o) => (o.limit_price ? fmtPrice(instrument, Number(o.limit_price)) : DASH) },
                { key: 'status', header: 'Status', numeric: false, cell: (o) => <Badge tone="neutral">{o.status.replace('_', ' ')}</Badge> },
                {
                  key: 'act',
                  header: '',
                  numeric: false,
                  cell: (o) => (
                    <Button
                      size="xs"
                      variant="ghost"
                      icon={<Ban size={11} aria-hidden />}
                      loading={cancel.isPending}
                      onClick={(e) => {
                        e.stopPropagation()
                        cancel.mutate(o.order_id ?? o.id ?? '')
                      }}
                    >
                      Cancel
                    </Button>
                  ),
                },
              ]}
            />
          </TableScroll>
        )}

        {tab === 'fills' && (
          <TableScroll>
            <Table
              caption={`Fills in ${instrument}`}
              rows={fills}
              rowKey={(o) => `${o.order_id ?? o.id}-${o.status}`}
              loading={activity.isLoading}
              empty={
                <EmptyState
                  icon={<Inbox size={20} aria-hidden />}
                  message="No fills yet"
                  detail={`Executions in ${instrument} for the ${mode.toLowerCase()} account appear here with price, size and fee.`}
                />
              }
              columns={[
                { key: 'time', header: 'Time', align: 'left', numeric: false, cell: (o) => <span className="mono mut">{clockTime(o.created_at ?? o.ts)}</span> },
                { key: 'side', header: 'Side', numeric: false, cell: (o) => <Badge tone={o.side === 'buy' ? 'pos' : 'neg'}>{o.side}</Badge> },
                { key: 'qty', header: 'Qty', cell: (o) => fmtQty(instrument, Number(o.filled_qty ?? o.qty)) },
                { key: 'price', header: 'Price', cell: (o) => (o.limit_price ? fmtPrice(instrument, Number(o.limit_price)) : DASH) },
                { key: 'value', header: 'Value', cell: (o) => (o.limit_price ? money(Number(o.limit_price) * Number(o.filled_qty ?? o.qty)) : DASH) },
                { key: 'fee', header: 'Fee', cell: () => <span className="mut">{DASH}</span> },
                { key: 'src', header: 'Source', numeric: false, cell: () => <Badge tone="neutral">Manual</Badge> },
              ]}
            />
          </TableScroll>
        )}

        {tab === 'automations' && (
          <div style={{ overflow: 'auto', flex: 1 }}>
            {related.length === 0 ? (
              <EmptyState
                icon={<Zap size={20} aria-hidden />}
                message={`No automation is trading ${instrument}`}
                detail="An automation runs a strategy against this market without you at the desk."
                action={
                  <Button size="sm" variant="primary" onClick={() => navigate('/automations')}>
                    Create one
                  </Button>
                }
              />
            ) : (
              related.map((a) => (
                <button
                  key={a.id as string}
                  type="button"
                  onClick={() => navigate(`/automations?open=${a.id as string}`)}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 'var(--s-3)',
                    padding: '10px var(--s-4)',
                    borderBottom: '1px solid var(--line-hairline)',
                    width: '100%',
                    textAlign: 'left',
                    background: 'transparent',
                  }}
                >
                  <Badge tone={a.armed ? 'accent' : 'neutral'}>{a.armed ? 'Running' : 'Stopped'}</Badge>
                  <span style={{ fontSize: 'var(--t-13)', fontWeight: 'var(--w-medium)' }}>
                    {String(a.kind).replace(/_/g, ' ')}
                  </span>
                  <span className="mut" style={{ fontSize: 'var(--t-11)' }}>
                    {relativeTime(a.created_at as string)}
                  </span>
                  <span className="spacer" />
                  <Badge tone={a.account_mode === 'live' ? 'pos' : 'warn'}>{String(a.account_mode)}</Badge>
                </button>
              ))
            )}
          </div>
        )}

        {tab === 'tape' && (
          <Tape instrument={instrument} />
        )}
      </PanelBody>
    </Panel>
  )
}
