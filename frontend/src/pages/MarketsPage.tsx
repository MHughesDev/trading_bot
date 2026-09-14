import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { AlertTriangle, Database, Download, ExternalLink, Play, Plus, RotateCw, Search, Square } from 'lucide-react'
import { assetApi, strategiesApi, universeApi } from '@/lib/api'
import { useToast } from '@/hooks/useToast'
import { ASSET_CLASSES, assetClassChartColor, assetClassInfo, inferAssetClass, type PlatformAssetClass } from '@/lib/assetClass'
import { count as fmtCount } from '@/lib/format'
import { Panel, PanelBody, PanelFooter, PanelHeader } from '@/components/primitives/Panel'
import { Button, IconButton } from '@/components/primitives/Button'
import { Badge, Swatch } from '@/components/primitives/Badge'
import { Label } from '@/components/primitives/Num'
import { Field, Input, SearchInput } from '@/components/primitives/Field'
import { Segmented } from '@/components/primitives/Segmented'
import { Select, Tooltip } from '@/components/primitives/Overlay'
import { Table, IdentityCell, TableScroll } from '@/components/primitives/Table'
import { Modal, ConfirmDialog } from '@/components/primitives/Modal'
import { EmptyState, NoResults, PanelLoading } from '@/components/primitives/States'

/* =============================================================================
   Markets — the instrument lifecycle surface.

   Every market the platform knows about, what state it is in, and the controls
   that move it: initialise (seed history and start streaming), start, stop, and
   which strategy and execution mode it runs under.

   This used to be buried in a per-asset page that also duplicated the terminal.
   Charting and trading one market is the terminal's job; managing the set of
   markets is this page's job.
   ============================================================================= */

const LOOKBACKS = [
  { value: '30', label: '30 days' },
  { value: '90', label: '90 days' },
  { value: '180', label: '180 days' },
  { value: '365', label: '1 year' },
  { value: '730', label: '2 years' },
]

interface InitializedAsset {
  symbol: string
  asset_class: string
}

interface Lifecycle {
  /** `uninitialized | initialized_not_active | active` */
  lifecycle?: string
  asset_class?: string
  execution_mode?: string
  strategy_id?: string | null
  venue_id?: string
}

export function MarketsPage() {
  const navigate = useNavigate()
  const qc = useQueryClient()
  const toast = useToast()
  const [q, setQ] = useState('')
  const [classFilter, setClassFilter] = useState<'all' | PlatformAssetClass>('all')
  const [initOpen, setInitOpen] = useState(false)
  const [stopping, setStopping] = useState<string | null>(null)

  const assets = useQuery({
    queryKey: ['initialized-assets'],
    queryFn: async () => {
      const { data } = await assetApi.initialized()
      return (data as { assets?: InitializedAsset[] }).assets ?? []
    },
    refetchInterval: 20_000,
  })

  const strategies = useQuery({
    queryKey: ['strategies'],
    queryFn: async () => {
      const { data } = await strategiesApi.list()
      return (data as { strategies?: { id: string; strategy_id: string }[] }).strategies ?? []
    },
  })

  const start = useMutation({
    mutationFn: (symbol: string) => assetApi.start(symbol),
    onSuccess: (_, s) => {
      toast({ title: `${s} started`, description: 'Live bars are streaming and strategies will run.' })
      void qc.invalidateQueries({ queryKey: ['initialized-assets'] })
    },
    onError: () => toast({ title: 'Could not start that market', variant: 'error' }),
  })

  const stop = useMutation({
    mutationFn: (symbol: string) => assetApi.stop(symbol),
    onSuccess: (_, s) => {
      toast({ title: `${s} stopped` })
      void qc.invalidateQueries({ queryKey: ['initialized-assets'] })
    },
    onError: () => toast({ title: 'Could not stop that market', variant: 'error' }),
  })

  const rows = useMemo(() => {
    let list = assets.data ?? []
    if (classFilter !== 'all') list = list.filter((a) => a.asset_class === classFilter)
    const needle = q.trim().toLowerCase()
    if (needle) list = list.filter((a) => a.symbol.toLowerCase().includes(needle))
    return list
  }, [assets.data, classFilter, q])

  const byClass = useMemo(() => {
    const m = new Map<string, number>()
    for (const a of assets.data ?? []) m.set(a.asset_class, (m.get(a.asset_class) ?? 0) + 1)
    return m
  }, [assets.data])

  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Markets</h1>
        <Badge tone="neutral">{fmtCount((assets.data ?? []).length)} initialised</Badge>
        <div className="spacer" />
        <IconButton label="Refresh" onClick={() => void assets.refetch()}>
          <RotateCw size={14} aria-hidden />
        </IconButton>
        <Button size="sm" variant="primary" icon={<Plus size={13} aria-hidden />} onClick={() => setInitOpen(true)}>
          Add a market
        </Button>
      </div>

      <div className="page-body">
        <Panel>
          <PanelHeader
            title="Coverage by asset class"
            actions={<Label>{ASSET_CLASSES.length} classes supported</Label>}
          />
          <PanelBody>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit,minmax(120px,1fr))', gap: 'var(--s-3)' }}>
              {ASSET_CLASSES.map((a) => {
                const n = byClass.get(a.id) ?? 0
                return (
                  <button
                    key={a.id}
                    type="button"
                    onClick={() => setClassFilter(classFilter === a.id ? 'all' : a.id)}
                    aria-pressed={classFilter === a.id}
                    style={{
                      textAlign: 'left',
                      padding: 'var(--s-3)',
                      borderRadius: 'var(--r-md)',
                      border: `1px solid ${classFilter === a.id ? 'var(--line-accent)' : 'var(--line-hairline)'}`,
                      background: classFilter === a.id ? 'var(--bg-selected)' : 'var(--bg-sunken)',
                    }}
                  >
                    <span className="row" style={{ gap: 6 }}>
                      <Swatch color={assetClassChartColor(a.id)} />
                      <span className="lbl">{a.short}</span>
                    </span>
                    <div className="num" style={{ fontSize: 'var(--t-20)', fontWeight: 'var(--w-semibold)', marginTop: 4 }}>
                      {n}
                    </div>
                  </button>
                )
              })}
            </div>
          </PanelBody>
        </Panel>

        <Panel style={{ minHeight: 340 }}>
          <PanelHeader
            title="Initialised markets"
            actions={
              <div className="row" style={{ gap: 'var(--s-2)' }}>
                <div style={{ width: 220 }}>
                  <SearchInput value={q} onValueChange={setQ} placeholder="Filter markets" />
                </div>
                {classFilter !== 'all' && (
                  <Button size="xs" variant="ghost" onClick={() => setClassFilter('all')}>
                    Clear class filter
                  </Button>
                )}
              </div>
            }
          />
          <PanelBody flush>
            {assets.isLoading ? (
              <div style={{ padding: 'var(--s-4)' }}>
                <PanelLoading lines={5} />
              </div>
            ) : (
              <TableScroll>
                <Table
                  caption="Initialised markets"
                  rows={rows}
                  rowKey={(a) => a.symbol}
                  onRowClick={(a) => navigate(`/terminal/${encodeURIComponent(a.symbol)}`)}
                  empty={
                    q ? (
                      <NoResults query={q} onClear={() => setQ('')} />
                    ) : (
                      <EmptyState
                        icon={<Database size={20} aria-hidden />}
                        message="No markets initialised"
                        detail="Initialising a market seeds its history and starts streaming live bars. Everything else in the product depends on it."
                        action={
                          <Button size="sm" variant="primary" onClick={() => setInitOpen(true)}>
                            Add your first market
                          </Button>
                        }
                      />
                    )
                  }
                  columns={[
                    {
                      key: 'symbol',
                      header: 'Market',
                      align: 'left',
                      numeric: false,
                      sortValue: (a) => a.symbol,
                      cell: (a) => (
                        <IdentityCell
                          swatch={<Swatch color={assetClassChartColor(a.asset_class)} />}
                          ticker={a.symbol}
                          name={assetClassInfo(a.asset_class).label}
                        />
                      ),
                    },
                    {
                      key: 'state',
                      header: 'State',
                      numeric: false,
                      cell: (a) => <LifecycleCell symbol={a.symbol} />,
                    },
                    {
                      key: 'strategy',
                      header: 'Strategy',
                      numeric: false,
                      cell: (a) => (
                        <StrategyCell symbol={a.symbol} strategies={strategies.data ?? []} />
                      ),
                    },
                    {
                      key: 'mode',
                      header: 'Execution',
                      numeric: false,
                      cell: (a) => <ExecutionModeCell symbol={a.symbol} />,
                    },
                    {
                      key: 'act',
                      header: '',
                      numeric: false,
                      cell: (a) => (
                        <span className="row" style={{ justifyContent: 'flex-end', gap: 2 }}>
                          <Tooltip content="Start streaming and running strategies">
                            <IconButton
                              label={`Start ${a.symbol}`}
                              bare
                              onClick={(e) => {
                                e.stopPropagation()
                                start.mutate(a.symbol)
                              }}
                            >
                              <Play size={12} aria-hidden />
                            </IconButton>
                          </Tooltip>
                          <Tooltip content="Stop streaming">
                            <IconButton
                              label={`Stop ${a.symbol}`}
                              bare
                              onClick={(e) => {
                                e.stopPropagation()
                                setStopping(a.symbol)
                              }}
                            >
                              <Square size={12} aria-hidden />
                            </IconButton>
                          </Tooltip>
                          <Tooltip content="Open in the terminal">
                            <IconButton
                              label={`Open ${a.symbol}`}
                              bare
                              onClick={(e) => {
                                e.stopPropagation()
                                navigate(`/terminal/${encodeURIComponent(a.symbol)}`)
                              }}
                            >
                              <ExternalLink size={12} aria-hidden />
                            </IconButton>
                          </Tooltip>
                        </span>
                      ),
                    },
                  ]}
                />
              </TableScroll>
            )}
          </PanelBody>
          <PanelFooter>
            <Label>Click a row to trade that market. Stopping a market halts its feed; it keeps its history.</Label>
          </PanelFooter>
        </Panel>
      </div>

      <InitialiseDialog
        open={initOpen}
        onClose={() => setInitOpen(false)}
        onDone={() => {
          setInitOpen(false)
          void qc.invalidateQueries({ queryKey: ['initialized-assets'] })
        }}
      />

      <ConfirmDialog
        open={!!stopping}
        onCancel={() => setStopping(null)}
        onConfirm={() => {
          if (stopping) stop.mutate(stopping)
          setStopping(null)
        }}
        title={`Stop ${stopping}?`}
        confirmLabel="Stop the feed"
        consequence={
          <>
            Live bars stop arriving for <strong>{stopping}</strong> and any strategy bound to it stops
            receiving data. Stored history is kept, and you can start it again at any time.
          </>
        }
      />
    </>
  )
}

/* --- per-row cells ---------------------------------------------------------- */
function LifecycleCell({ symbol }: { symbol: string }) {
  const lc = useQuery({
    queryKey: ['lifecycle', symbol],
    queryFn: async () => {
      const { data } = await assetApi.lifecycle(symbol)
      return data as Lifecycle
    },
    refetchInterval: 30_000,
    retry: false,
  })

  const state = lc.data?.lifecycle ?? 'unknown'
  const tone =
    state === 'active' ? 'pos' : state === 'initialized_not_active' ? 'warn' : state === 'uninitialized' ? 'neutral' : 'neutral'
  const label =
    state === 'active'
      ? 'Streaming'
      : state === 'initialized_not_active'
        ? 'Stopped'
        : state === 'uninitialized'
          ? 'No data'
          : state.replace(/_/g, ' ')

  return (
    <span className="row" style={{ justifyContent: 'flex-end', gap: 'var(--s-2)' }}>
      {lc.data?.venue_id && <span className="lbl">{lc.data.venue_id}</span>}
      <Badge tone={tone as 'pos'}>{label}</Badge>
    </span>
  )
}

function StrategyCell({ symbol, strategies }: { symbol: string; strategies: { id: string; strategy_id: string }[] }) {
  const qc = useQueryClient()
  const toast = useToast()
  const current = useQuery({
    queryKey: ['asset-strategy', symbol],
    queryFn: async () => {
      const { data } = await assetApi.strategy(symbol)
      return (data as { strategy_id?: string | null })?.strategy_id ?? null
    },
    retry: false,
  })
  const setStrategy = useMutation({
    mutationFn: (id: string) => assetApi.setStrategy(symbol, id),
    onSuccess: () => {
      toast({ title: `${symbol} strategy updated` })
      void qc.invalidateQueries({ queryKey: ['asset-strategy', symbol] })
    },
    onError: () => toast({ title: 'Could not set that strategy', variant: 'error' }),
  })

  return (
    <span onClick={(e) => e.stopPropagation()} style={{ display: 'inline-block', minWidth: 170 }}>
      <Select
        ariaLabel={`Strategy for ${symbol}`}
        small
        value={current.data ?? null}
        placeholder="None"
        onChange={(v) => setStrategy.mutate(v)}
        options={strategies.map((s) => ({ value: s.id, label: s.strategy_id, text: s.strategy_id }))}
      />
    </span>
  )
}

function ExecutionModeCell({ symbol }: { symbol: string }) {
  const qc = useQueryClient()
  const toast = useToast()
  const current = useQuery({
    queryKey: ['exec-mode', symbol],
    queryFn: async () => {
      const { data } = await assetApi.executionMode(symbol)
      const d = data as { mode?: string; execution_mode?: string }
      return d?.mode ?? d?.execution_mode ?? 'paper'
    },
    retry: false,
  })
  const setMode = useMutation({
    mutationFn: (m: string) => assetApi.setExecutionMode(symbol, m),
    onSuccess: (_, m) => {
      toast({
        title: `${symbol} now executes on ${m}`,
        variant: m === 'live' ? 'warning' : 'default',
      })
      void qc.invalidateQueries({ queryKey: ['exec-mode', symbol] })
    },
    onError: () => toast({ title: 'Could not change the execution mode', variant: 'error' }),
  })

  return (
    <span onClick={(e) => e.stopPropagation()} style={{ display: 'inline-block' }}>
      <Segmented
        ariaLabel={`Execution mode for ${symbol}`}
        value={current.data ?? 'paper'}
        onChange={(v) => setMode.mutate(v)}
        options={[
          { value: 'paper', label: 'Paper' },
          { value: 'live', label: 'Live' },
        ]}
      />
    </span>
  )
}

/* --- initialise ------------------------------------------------------------- */
function InitialiseDialog({
  open,
  onClose,
  onDone,
}: {
  open: boolean
  onClose: () => void
  onDone: () => void
}) {
  const toast = useToast()
  const [symbol, setSymbol] = useState('')
  const [assetClass, setAssetClass] = useState<PlatformAssetClass>('crypto_spot_cex')
  const [lookback, setLookback] = useState('90')
  const [search, setSearch] = useState('')

  useEffect(() => {
    if (open) {
      setSymbol('')
      setSearch('')
    }
  }, [open])

  useEffect(() => {
    if (symbol) setAssetClass(inferAssetClass(symbol))
  }, [symbol])

  const results = useQuery({
    queryKey: ['universe-search', search],
    enabled: search.trim().length >= 2,
    queryFn: async () => {
      const { data } = await universeApi.search(search.trim())
      return ((data as { results?: { symbol: string; name?: string }[] })?.results ?? []).slice(0, 12)
    },
    retry: false,
  })

  const init = useMutation({
    mutationFn: () => assetApi.init(symbol, Number(lookback), assetClass),
    onSuccess: () => {
      toast({
        title: `${symbol} queued for initialisation`,
        description: `Seeding ${lookback} days of history, then streaming live.`,
      })
      onDone()
    },
    onError: (e: unknown) => {
      const err = e as { response?: { data?: { message?: string } } }
      toast({ title: 'Could not initialise that market', description: err.response?.data?.message, variant: 'error' })
    },
  })

  const estBars = Number(lookback) * 24

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Add a market"
      description="Seeds historical bars, then keeps the market streaming live."
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" disabled={!symbol.trim()} loading={init.isPending} onClick={() => init.mutate()}>
            Initialise {symbol || 'market'}
          </Button>
        </>
      }
    >
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
        <Field label="Find an instrument" hint="Search the venue universe, or type a symbol directly.">
          <SearchInput value={search} onValueChange={setSearch} placeholder="BTC, AAPL, EURUSD…" autoFocus />
        </Field>

        {search.trim().length >= 2 && (
          <div className="well" style={{ padding: 0, maxHeight: 200, overflow: 'auto' }}>
            {results.isLoading ? (
              <div style={{ padding: 'var(--s-3)' }}>
                <PanelLoading lines={3} />
              </div>
            ) : (results.data ?? []).length === 0 ? (
              <div className="empty" style={{ minHeight: 90 }}>
                <span className="msg">
                  Nothing matched <em>{search}</em>. You can still type the symbol below.
                </span>
              </div>
            ) : (
              (results.data ?? []).map((r) => (
                <button
                  key={r.symbol}
                  type="button"
                  className="menu-item"
                  onClick={() => {
                    setSymbol(r.symbol)
                    setSearch('')
                  }}
                >
                  <Search size={12} aria-hidden />
                  <span style={{ fontWeight: 'var(--w-medium)' }}>{r.symbol}</span>
                  {r.name && <span className="mut truncate-1">{r.name}</span>}
                </button>
              ))
            )}
          </div>
        )}

        <Input
          label="Symbol"
          value={symbol}
          onChange={(e) => setSymbol(e.target.value.toUpperCase())}
          placeholder="BTC-USD"
          spellCheck={false}
        />

        <Field label="Asset class">
          <Select
            ariaLabel="Asset class"
            value={assetClass}
            onChange={(v) => setAssetClass(v as PlatformAssetClass)}
            options={ASSET_CLASSES.map((a) => ({ value: a.id, label: a.label, text: a.label }))}
          />
        </Field>

        <Field label="History to seed" hint={`About ${fmtCount(estBars)} hourly bars.`}>
          <Segmented
            ariaLabel="Lookback"
            wide
            value={lookback}
            onChange={setLookback}
            options={LOOKBACKS.map((l) => ({ value: l.value, label: l.label }))}
          />
        </Field>

        <div className="callout info">
          <Download size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
          <span>
            Seeding runs in the background. The market appears in the list immediately and fills in as bars
            arrive — you do not need to stay on this page.
          </span>
        </div>

        {!assetClassInfo(assetClass).clob && (
          <div className="callout warn">
            <AlertTriangle size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
            <span>
              {assetClassInfo(assetClass).label} has no central limit order book, so depth and stop orders
              are unavailable for it.
            </span>
          </div>
        )}
      </div>
    </Modal>
  )
}
