import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { Download, Inbox, RotateCw } from 'lucide-react'
import { portfolioApi } from '@/lib/api'
import { inferAssetClass, assetClassChartColor, assetClassInfo } from '@/lib/assetClass'
import { DASH, clockTime, count as fmtCount, qty as fmtQty, shortDate } from '@/lib/format'
import { Panel, PanelBody, PanelFooter, PanelHeader } from '@/components/primitives/Panel'
import { Button, IconButton } from '@/components/primitives/Button'
import { Badge, Swatch } from '@/components/primitives/Badge'
import { Label } from '@/components/primitives/Num'
import { Field, Input, SearchInput } from '@/components/primitives/Field'
import { Segmented } from '@/components/primitives/Segmented'
import { Table, IdentityCell, TableScroll } from '@/components/primitives/Table'
import { EmptyState, ErrorState, NoResults } from '@/components/primitives/States'

/* =============================================================================
   Account activity — every transaction the account has recorded.

   The backend already accepts start / end / symbol filters; the previous screen
   fetched a flat 500 and offered none of them. This exposes the whole contract:
   a date range, a symbol filter, a side filter and an export.
   ============================================================================= */

interface Transaction {
  ts: string
  symbol: string
  side: string
  quantity: string
  source: string
  correlation_id: string | null
  execution_mode: string | null
}

type SideFilter = 'all' | 'buy' | 'sell'

function isoDaysAgo(days: number): string {
  return new Date(Date.now() - days * 86400_000).toISOString().slice(0, 10)
}

export function TransactionsPage() {
  const navigate = useNavigate()
  const [symbol, setSymbol] = useState('')
  const [side, setSide] = useState<SideFilter>('all')
  const [start, setStart] = useState(isoDaysAgo(30))
  const [end, setEnd] = useState(new Date().toISOString().slice(0, 10))
  const [q, setQ] = useState('')

  const tx = useQuery({
    queryKey: ['account-transactions', symbol, start, end],
    queryFn: async () => {
      const { data } = await portfolioApi.transactions({
        limit: 1000,
        symbol: symbol || undefined,
        start: start ? new Date(start).toISOString() : undefined,
        end: end ? new Date(`${end}T23:59:59`).toISOString() : undefined,
      })
      return ((data as { transactions?: Transaction[] })?.transactions ?? (data as unknown as Transaction[]) ?? []) as Transaction[]
    },
    refetchInterval: 30_000,
    retry: false,
  })

  const rows = useMemo(() => {
    let list = tx.data ?? []
    if (side !== 'all') list = list.filter((t) => t.side?.toLowerCase() === side)
    const needle = q.trim().toLowerCase()
    if (needle) {
      list = list.filter((t) => `${t.symbol} ${t.source} ${t.correlation_id ?? ''}`.toLowerCase().includes(needle))
    }
    return list
  }, [tx.data, side, q])

  function exportCsv() {
    const head = ['Time', 'Symbol', 'Side', 'Quantity', 'Source', 'Mode', 'Correlation id']
    const body = rows.map((t) => [t.ts, t.symbol, t.side, t.quantity, t.source, t.execution_mode ?? '', t.correlation_id ?? ''])
    const csv = [head, ...body].map((l) => l.join(',')).join('\n')
    const url = URL.createObjectURL(new Blob([csv], { type: 'text/csv' }))
    const a = document.createElement('a')
    a.href = url
    a.download = `activity-${start}-to-${end}.csv`
    a.click()
    URL.revokeObjectURL(url)
  }

  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Account activity</h1>
        <Badge tone="neutral">{fmtCount(rows.length)}</Badge>
        <div className="spacer" />
        <IconButton label="Refresh" onClick={() => void tx.refetch()}>
          <RotateCw size={14} aria-hidden />
        </IconButton>
        <Button size="sm" icon={<Download size={13} aria-hidden />} disabled={rows.length === 0} onClick={exportCsv}>
          Export
        </Button>
      </div>

      <div className="page-body">
        <Panel>
          <PanelHeader title="Filters" />
          <PanelBody>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit,minmax(170px,1fr))', gap: 'var(--s-3)', alignItems: 'end' }}>
              <Input label="From" type="date" value={start} max={end} onChange={(e) => setStart(e.target.value)} />
              <Input label="To" type="date" value={end} min={start} onChange={(e) => setEnd(e.target.value)} />
              <Input
                label="Symbol"
                value={symbol}
                placeholder="All markets"
                onChange={(e) => setSymbol(e.target.value.toUpperCase())}
              />
              <Field label="Side">
                <Segmented
                  ariaLabel="Side"
                  wide
                  value={side}
                  onChange={setSide}
                  options={[
                    { value: 'all', label: 'All' },
                    { value: 'buy', label: 'Buy' },
                    { value: 'sell', label: 'Sell' },
                  ]}
                />
              </Field>
              <Field label="Search">
                <SearchInput value={q} onValueChange={setQ} placeholder="Symbol, source or id" />
              </Field>
            </div>
          </PanelBody>
        </Panel>

        <Panel style={{ minHeight: 360 }}>
          <PanelHeader
            title="Transactions"
            actions={
              <Label>
                {shortDate(start)} {'→'} {shortDate(end)}
              </Label>
            }
          />
          <PanelBody flush>
            {tx.isError ? (
              <ErrorState
                message="The account ledger is not reachable."
                detail="Account activity is served by a portfolio service that is not running. Per-instrument fills are visible in the terminal dock."
                onRetry={() => void tx.refetch()}
              />
            ) : (
              <TableScroll>
                <Table
                  caption="Account transactions"
                  rows={rows}
                  rowKey={(t, i) => `${t.ts}-${t.symbol}-${i}`}
                  loading={tx.isLoading}
                  onRowClick={(t) => navigate(`/terminal/${encodeURIComponent(t.symbol)}`)}
                  initialSort={{ key: 'time', dir: 'desc' }}
                  empty={
                    q ? (
                      <NoResults query={q} onClear={() => setQ('')} />
                    ) : (
                      <EmptyState
                        icon={<Inbox size={20} aria-hidden />}
                        message="No transactions in this range"
                        detail="Widen the dates, or place an order to see activity here."
                        action={
                          <Button size="sm" variant="primary" onClick={() => navigate('/terminal')}>
                            Open the terminal
                          </Button>
                        }
                      />
                    )
                  }
                  columns={[
                    {
                      key: 'time',
                      header: 'Time',
                      align: 'left',
                      numeric: false,
                      sortValue: (t) => t.ts,
                      cell: (t) => (
                        <span className="sym-stack">
                          <span className="mono" style={{ fontSize: 'var(--t-12)' }}>{clockTime(t.ts)}</span>
                          <span className="mut" style={{ fontSize: 'var(--t-11)' }}>{shortDate(t.ts)}</span>
                        </span>
                      ),
                    },
                    {
                      key: 'symbol',
                      header: 'Instrument',
                      align: 'left',
                      numeric: false,
                      sortValue: (t) => t.symbol,
                      cell: (t) => (
                        <IdentityCell
                          swatch={<Swatch color={assetClassChartColor(inferAssetClass(t.symbol))} />}
                          ticker={t.symbol}
                          name={assetClassInfo(inferAssetClass(t.symbol)).label}
                        />
                      ),
                    },
                    {
                      key: 'side',
                      header: 'Side',
                      numeric: false,
                      sortValue: (t) => t.side,
                      cell: (t) => <Badge tone={t.side?.toLowerCase() === 'buy' ? 'pos' : 'neg'}>{t.side}</Badge>,
                    },
                    {
                      key: 'qty',
                      header: 'Quantity',
                      sortValue: (t) => Number(t.quantity) || 0,
                      cell: (t) => fmtQty(t.symbol, t.quantity),
                    },
                    { key: 'source', header: 'Source', numeric: false, cell: (t) => <span className="mut">{t.source || DASH}</span> },
                    {
                      key: 'mode',
                      header: 'Account',
                      numeric: false,
                      cell: (t) => <Badge tone={t.execution_mode === 'live' ? 'pos' : 'warn'}>{t.execution_mode ?? 'paper'}</Badge>,
                    },
                    {
                      key: 'corr',
                      header: 'Correlation',
                      numeric: false,
                      cell: (t) => <span className="mono mut truncate-1">{t.correlation_id ?? DASH}</span>,
                    },
                  ]}
                />
              </TableScroll>
            )}
          </PanelBody>
          <PanelFooter>
            <Label>Click a row to open that market in the terminal.</Label>
          </PanelFooter>
        </Panel>
      </div>
    </>
  )
}
