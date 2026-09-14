import { useEffect, useMemo, useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { AlertTriangle, FlaskConical, Play, RotateCcw, ShieldCheck, StopCircle, Trash2 } from 'lucide-react'
import { backtestsApi, strategyPickerApi, type BacktestSnapshot, type CreateBacktestRequest, type InstrumentCoverage } from '@/api/backtests'
import { api } from '@/lib/api'
import { experimentsApi, type ExperimentView } from '@/api/experiments'
import { useSuiteProgress, type SuiteProgress } from '@/hooks/useSuiteProgress'
import { ASSET_CLASSES, assetClassInfo } from '@/lib/assetClass'
import { isActive, phaseLabel, statusPresentation } from '@/components/backtest/status'
import { Panel, PanelBody, PanelFooter, PanelHeader } from '@/components/primitives/Panel'
import { Button, IconButton } from '@/components/primitives/Button'
import { Badge, KeyValue } from '@/components/primitives/Badge'
import { Label, StatTile } from '@/components/primitives/Num'
import { Field, Input, NumberField } from '@/components/primitives/Field'
import { Segmented, Tabs } from '@/components/primitives/Segmented'
import { Select, Tooltip } from '@/components/primitives/Overlay'
import { Table, TableScroll } from '@/components/primitives/Table'
import { ConfirmDialog } from '@/components/primitives/Modal'
import { EmptyState, PanelLoading } from '@/components/primitives/States'
import { AreaChart, Meter } from '@/components/charts/Primitives'
import { FleetBoard } from '@/components/proving-ground/FleetBoard'
import { RunStudyPanel } from '@/components/proving-ground/RunStudyPanel'
import { ExperimentDetail } from '@/components/workbench/ExperimentDetail'
import { useToast } from '@/hooks/useToast'
import { DASH, money, pct, ratio, relativeTime, shortDate, signedPct } from '@/lib/format'
import { TIMEFRAMES } from '@/hooks/useBars'
import { cn } from '@/lib/utils'

/* =============================================================================
   Spec §4.4 — Back Testing.

   "A run configuration panel (left 340), a results area with the equity curve
    on top and a tabbed dock (Trades · Monthly returns · Drawdown · Statistics).
    Monthly returns use the DIVERGING ramp, never the categorical one."

   Two modes, because the product genuinely has two jobs:
     Quick      — run a strategy against history and look at the result.
     Rigorous   — the honest-evaluation suite: every run costs a trial and the
                  strategy must beat a randomised baseline to count.
   ============================================================================= */

type Mode = 'quick' | 'rigorous'
type ResultTab = 'trades' | 'monthly' | 'drawdown' | 'stats' | 'coverage'

const DURATIONS = [
  { value: '7', label: '7 days' },
  { value: '30', label: '30 days' },
  { value: '90', label: '90 days' },
  { value: '180', label: '180 days' },
  { value: '365', label: '1 year' },
  { value: '730', label: '2 years' },
]

/** Asset classes with an automated collector — the rest cannot auto-backfill. */
const COLLECTABLE = new Set(['crypto_spot_cex', 'perpetual_swap', 'equity', 'etf'])

export function BackTestingPage() {
  const [mode, setMode] = useState<Mode>('quick')

  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Back testing</h1>
        <Segmented
          ariaLabel="Back testing mode"
          value={mode}
          onChange={setMode}
          options={[
            { value: 'quick', label: 'Quick runs', title: 'Run a strategy against history. No search counter.' },
            { value: 'rigorous', label: 'Rigorous', title: 'Every run counts against your search total and must beat a randomised baseline.' },
          ]}
        />
        <div className="spacer" />
        <Label>
          {mode === 'quick'
            ? 'Free exploration · results are indicative, not evidence'
            : 'Every run increments the trial counter and raises the bar for significance'}
        </Label>
      </div>

      {mode === 'quick' ? <QuickMode /> : <RigorousMode />}
    </>
  )
}

/* =============================================================================
   QUICK MODE
   ============================================================================= */
function QuickMode() {
  const qc = useQueryClient()
  const toast = useToast()
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [tab, setTab] = useState<ResultTab>('stats')
  const [deleting, setDeleting] = useState<BacktestSnapshot | null>(null)

  const runs = useQuery({
    queryKey: ['backtests'],
    queryFn: () => backtestsApi.list({ limit: 50 }).then((r) => r.data),
    refetchInterval: (q) =>
      q.state.data?.backtests.some((b) => isActive(b.status)) ? 1500 : 8000,
  })

  const list = runs.data?.backtests ?? []
  const selected = useMemo(
    () => list.find((r) => r.id === selectedId) ?? list[0] ?? null,
    [list, selectedId],
  )

  const remove = useMutation({
    mutationFn: (id: string) => backtestsApi.remove(id),
    onSuccess: () => {
      toast({ title: 'Run deleted' })
      void qc.invalidateQueries({ queryKey: ['backtests'] })
    },
  })
  const stop = useMutation({
    mutationFn: (id: string) => backtestsApi.stop(id),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ['backtests'] }),
  })
  const rerun = useMutation({
    mutationFn: (id: string) => backtestsApi.rerun(id),
    onSuccess: (r) => {
      setSelectedId((r.data as { id: string }).id)
      void qc.invalidateQueries({ queryKey: ['backtests'] })
    },
  })

  return (
    <div className="page-body grid-rail fill" style={{ minHeight: 0 }}>
      <RunConfigPanel onCreated={(id) => setSelectedId(id)} />

      <div style={{ display: 'grid', gridTemplateRows: 'auto minmax(0,1fr)', gap: 'var(--s-3)', minHeight: 0, height: '100%' }}>
        <Panel>
          <PanelHeader
            title="Runs"
            badges={<Badge tone="neutral">{runs.data?.total ?? 0}</Badge>}
            actions={
              list.some((r) => isActive(r.status)) ? (
                <Badge tone="accent">{list.filter((r) => isActive(r.status)).length} running</Badge>
              ) : undefined
            }
          />
          <PanelBody flush style={{ maxHeight: 232, overflow: 'auto' }}>
            {runs.isLoading ? (
              <div style={{ padding: 'var(--s-4)' }}><PanelLoading lines={3} /></div>
            ) : list.length === 0 ? (
              <EmptyState
                icon={<FlaskConical size={20} aria-hidden />}
                message="No backtests yet"
                detail="Configure a run on the left and start it. Results land here."
              />
            ) : (
              <TableScroll>
                <Table
                  dense
                  caption="Backtest runs"
                  rows={list}
                  rowKey={(r) => r.id}
                  selectedKey={selected?.id ?? null}
                  onRowClick={(r) => setSelectedId(r.id)}
                  columns={[
                    {
                      key: 'name',
                      header: 'Run',
                      align: 'left',
                      numeric: false,
                      sortValue: (r) => r.name,
                      cell: (r) => (
                        <span className="sym-stack">
                          <span className="truncate-1" style={{ fontWeight: 'var(--w-medium)' }}>{r.name}</span>
                          <span className="mut truncate-1" style={{ fontSize: 'var(--t-11)' }}>
                            {r.instrument_id} {'·'} {r.timeframe} {'·'} {assetClassInfo(r.asset_class).short}
                          </span>
                        </span>
                      ),
                    },
                    {
                      key: 'status',
                      header: 'Status',
                      numeric: false,
                      sortValue: (r) => r.status,
                      cell: (r) => {
                        const p = statusPresentation(r.status)
                        const tone = r.status === 'completed' ? 'pos' : r.status === 'failed' ? 'neg' : p.busy ? 'accent' : 'neutral'
                        return (
                          <span className="row" style={{ justifyContent: 'flex-end', gap: 6 }}>
                            {p.busy && <span className="spinner" style={{ width: 10, height: 10 }} aria-hidden />}
                            <Badge tone={tone as 'pos'}>{p.label}</Badge>
                          </span>
                        )
                      },
                    },
                    {
                      key: 'progress',
                      header: 'Progress',
                      width: 120,
                      cell: (r) =>
                        isActive(r.status) ? (
                          <Meter value={r.progress} max={100} ariaLabel={`${r.progress}%`} />
                        ) : (
                          <span className="mut">{DASH}</span>
                        ),
                    },
                    {
                      key: 'window',
                      header: 'Window',
                      numeric: false,
                      cell: (r) => (
                        <span className="mut">{shortDate(r.start)} → {shortDate(r.end)}</span>
                      ),
                    },
                    { key: 'created', header: 'Started', numeric: false, sortValue: (r) => r.created_at, cell: (r) => <span className="mut">{relativeTime(r.created_at)}</span> },
                    {
                      key: 'act',
                      header: '',
                      numeric: false,
                      cell: (r) => (
                        <span className="row" style={{ justifyContent: 'flex-end', gap: 2 }}>
                          {isActive(r.status) && (
                            <IconButton label="Stop run" bare onClick={(e) => { e.stopPropagation(); stop.mutate(r.id) }}>
                              <StopCircle size={12} aria-hidden />
                            </IconButton>
                          )}
                          <IconButton label="Re-run" bare onClick={(e) => { e.stopPropagation(); rerun.mutate(r.id) }}>
                            <RotateCcw size={12} aria-hidden />
                          </IconButton>
                          <IconButton
                            label="Delete run"
                            bare
                            disabled={isActive(r.status)}
                            onClick={(e) => { e.stopPropagation(); setDeleting(r) }}
                          >
                            <Trash2 size={12} aria-hidden />
                          </IconButton>
                        </span>
                      ),
                    },
                  ]}
                />
              </TableScroll>
            )}
          </PanelBody>
        </Panel>

        <ResultArea run={selected} tab={tab} onTab={setTab} />
      </div>

      <ConfirmDialog
        open={!!deleting}
        onCancel={() => setDeleting(null)}
        onConfirm={() => {
          if (deleting) remove.mutate(deleting.id)
          setDeleting(null)
        }}
        title="Delete this run?"
        confirmLabel="Delete run"
        consequence={
          <>
            <strong>{deleting?.name}</strong> and its results are removed permanently. The strategy itself
            is untouched.
          </>
        }
      />
    </div>
  )
}

/* --- run configuration (left 340) ------------------------------------------ */
function RunConfigPanel({ onCreated }: { onCreated: (id: string) => void }) {
  const qc = useQueryClient()
  const toast = useToast()
  const [strategy, setStrategy] = useState<string | null>(null)
  const [assetClass, setAssetClass] = useState('crypto_spot_cex')
  const [instrument, setInstrument] = useState('BTC-USDT')
  const [timeframe, setTimeframe] = useState('1h')
  const [duration, setDuration] = useState('90')
  const [balance, setBalance] = useState('100000')
  const [quote, setQuote] = useState('USD')
  const [name, setName] = useState('')
  const [autoCollect, setAutoCollect] = useState(true)

  const strategies = useQuery({
    queryKey: ['strategies', 'picker'],
    queryFn: () => strategyPickerApi.list().then((r) => r.data.strategies),
  })

  const coverage = useQuery({
    queryKey: ['backtest-coverage'],
    queryFn: () => api.get<{ coverage: InstrumentCoverage[] }>('/api/backtests/coverage').then((r) => r.data.coverage),
    staleTime: 60_000,
  })

  const collectable = COLLECTABLE.has(assetClass)
  useEffect(() => {
    if (!collectable) setAutoCollect(false)
  }, [collectable])

  const cov = useMemo(() => {
    const rows = (coverage.data ?? []).filter((c) => c.instrument_id === instrument)
    if (rows.length === 0) return { none: true, forTf: null, others: [] as string[] }
    const forTf = rows.find((r) => r.timeframe === timeframe) ?? null
    return { none: false, forTf, others: rows.map((r) => r.timeframe) }
  }, [coverage.data, instrument, timeframe])

  const create = useMutation({
    mutationFn: () => {
      const end = new Date()
      const start = new Date(Date.now() - Number(duration) * 86400_000)
      const req: CreateBacktestRequest = {
        name: name || undefined,
        strategy_ref: strategy ?? undefined,
        instrument_id: instrument,
        asset_class: assetClass,
        timeframe,
        start: start.toISOString(),
        end: end.toISOString(),
        initial_balance: balance,
        quote_currency: quote,
        auto_collect: autoCollect,
      }
      return backtestsApi.create(req)
    },
    onSuccess: (r) => {
      toast({ title: 'Backtest queued', description: `${instrument} · ${timeframe} · ${duration} days` })
      onCreated(r.data.id)
      void qc.invalidateQueries({ queryKey: ['backtests'] })
    },
    onError: (e: unknown) => {
      const err = e as { response?: { data?: { message?: string; error?: string } } }
      toast({
        title: 'Could not start the run',
        description: err.response?.data?.message ?? err.response?.data?.error ?? 'The backtest service refused this configuration.',
        variant: 'error',
      })
    },
  })

  const valid = !!strategy && instrument.trim().length > 0

  return (
    <Panel style={{ position: 'sticky', top: 0 }}>
      <PanelHeader title="Run configuration" />
      <PanelBody style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
        <Field label="Strategy">
          <Select
            ariaLabel="Strategy"
            value={strategy}
            onChange={setStrategy}
            placeholder="Choose a saved strategy…"
            options={(strategies.data ?? []).map((s) => ({ value: s.id, label: s.strategy_id, text: s.strategy_id }))}
          />
        </Field>

        <Field label="Asset class">
          <Select
            ariaLabel="Asset class"
            value={assetClass}
            onChange={setAssetClass}
            options={ASSET_CLASSES.map((a) => ({ value: a.id, label: a.label, text: a.label }))}
          />
        </Field>

        <Input
          label="Instrument"
          value={instrument}
          onChange={(e) => setInstrument(e.target.value.toUpperCase())}
          spellCheck={false}
        />

        {cov.none && instrument && (
          <div className="callout warn">
            <AlertTriangle size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
            <span>
              No stored bars for <strong>{instrument}</strong>.
              {collectable
                ? ' Auto-collect will backfill them before the run starts.'
                : ` ${assetClassInfo(assetClass).label} has no automated collector, so this run would have nothing to simulate.`}
            </span>
          </div>
        )}
        {!cov.none && !cov.forTf && (
          <div className="callout warn">
            <AlertTriangle size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
            <span>
              {instrument} has bars, but not at {timeframe}. Available: {cov.others.join(', ')}.
            </span>
          </div>
        )}

        <Field label="Timeframe">
          <Segmented
            ariaLabel="Timeframe"
            wide
            value={timeframe}
            onChange={setTimeframe}
            options={TIMEFRAMES.map((t) => ({ value: t.label.toLowerCase(), label: t.label }))}
          />
        </Field>

        <Field label="Window">
          <Select
            ariaLabel="Duration"
            value={duration}
            onChange={setDuration}
            options={DURATIONS.map((d) => ({ value: d.value, label: d.label, text: d.label }))}
          />
        </Field>

        <div style={{ display: 'grid', gridTemplateColumns: '2fr 1fr', gap: 'var(--s-2)' }}>
          <NumberField label="Starting balance" value={balance} onValueChange={setBalance} dp={0} step={1000} min={0} />
          <Input label="Quote" value={quote} onChange={(e) => setQuote(e.target.value.toUpperCase())} />
        </div>

        <Input label="Name (optional)" value={name} onChange={(e) => setName(e.target.value)} placeholder="Defaults to strategy · instrument" />

        <Tooltip
          content={collectable ? undefined : `${assetClassInfo(assetClass).label} has no automated collector.`}
        >
          <label className="row" style={{ gap: 'var(--s-2)', fontSize: 'var(--t-12)', opacity: collectable ? 1 : 0.5 }}>
            <input
              type="checkbox"
              checked={autoCollect}
              disabled={!collectable}
              onChange={(e) => setAutoCollect(e.target.checked)}
            />
            Backfill missing bars before running
          </label>
        </Tooltip>
      </PanelBody>
      <PanelFooter>
        <Button
          variant="primary"
          block
          icon={<Play size={13} aria-hidden />}
          disabled={!valid}
          loading={create.isPending}
          onClick={() => create.mutate()}
        >
          Run backtest
        </Button>
      </PanelFooter>
    </Panel>
  )
}

/* --- results: equity curve on top, tabbed dock below ------------------------ */
function ResultArea({
  run,
  tab,
  onTab,
}: {
  run: BacktestSnapshot | null
  tab: ResultTab
  onTab: (t: ResultTab) => void
}) {
  if (!run) {
    return (
      <Panel>
        <PanelHeader title="Result" />
        <PanelBody>
          <EmptyState
            icon={<FlaskConical size={20} aria-hidden />}
            message="No run selected"
            detail="Pick a run above, or configure one on the left."
          />
        </PanelBody>
      </Panel>
    )
  }

  const p = statusPresentation(run.status)
  const pnls = run.result?.stats_pnls ?? {}
  const ccy = Object.keys(pnls)[0]
  const totalPnl = ccy ? pnls[ccy]['PnL (total)'] : undefined
  const sharpe = run.result?.stats_general?.['Sharpe Ratio (252 days)']
  const maxDd = run.result?.stats_returns?.['Max Drawdown (%)']

  // A per-bar equity series is not returned by the engine yet; the headline
  // statistics are. Render the stats honestly rather than draw a fake curve.
  const hasCurve = false

  return (
    <Panel style={{ minHeight: 0 }}>
      <PanelHeader
        title={run.name}
        badges={
          <>
            <Badge tone={run.status === 'completed' ? 'pos' : run.status === 'failed' ? 'neg' : 'accent'}>{p.label}</Badge>
            <Badge tone="neutral">{run.instrument_id}</Badge>
            <Badge tone="neutral">{run.timeframe}</Badge>
          </>
        }
        actions={
          <Label>
            {shortDate(run.start)} {'→'} {shortDate(run.end)} {'·'} {money(run.initial_balance, run.quote_currency, 0)} start
          </Label>
        }
      />

      {run.status === 'failed' && (
        <div className="banner neg">
          <AlertTriangle size={13} aria-hidden />
          <span>
            Failed during {phaseLabel(run.failed_phase ?? '')}. {run.error}
          </span>
        </div>
      )}

      <div style={{ padding: 'var(--s-4) var(--s-5)', borderBottom: '1px solid var(--line-hairline)' }}>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit,minmax(120px,1fr))', gap: 'var(--s-5)' }}>
          <StatTile
            label="Net P&L"
            value={
              totalPnl === undefined ? (
                <span className="mut">{DASH}</span>
              ) : (
                <span className={cn('num', totalPnl >= 0 ? 'pos' : 'neg')}>{money(totalPnl, ccy)}</span>
              )
            }
            meta={ccy}
          />
          <StatTile label="Orders filled" value={<span className="num">{run.result?.total_orders ?? DASH}</span>} />
          <StatTile label="Positions" value={<span className="num">{run.result?.total_positions ?? DASH}</span>} />
          <StatTile label="Sharpe" value={<span className="num">{sharpe === undefined ? DASH : ratio(sharpe)}</span>} meta="252 days" />
          <StatTile
            label="Max drawdown"
            value={maxDd === undefined ? <span className="mut">{DASH}</span> : <span className="num neg">{signedPct(-Math.abs(maxDd))}</span>}
          />
          <StatTile label="Events" value={<span className="num">{run.result?.total_events ?? DASH}</span>} />
        </div>
      </div>

      {hasCurve ? (
        <div style={{ padding: 'var(--s-4)' }}>
          <AreaChart points={[]} height={220} />
        </div>
      ) : run.status === 'completed' ? (
        <div style={{ padding: 'var(--s-4) var(--s-5)', borderBottom: '1px solid var(--line-hairline)' }}>
          <div className="callout neutral">
            <span>
              The engine returns summary statistics but not a per-bar equity series yet. When it does, the
              curve renders here with the same crosshair, range selector and table view as the portfolio
              equity curve.
            </span>
          </div>
        </div>
      ) : null}

      <Tabs
        ariaLabel="Result detail"
        value={tab}
        onChange={onTab}
        tabs={[
          { value: 'stats', label: 'Statistics' },
          { value: 'trades', label: 'Trades' },
          { value: 'monthly', label: 'Monthly returns' },
          { value: 'drawdown', label: 'Drawdown' },
          { value: 'coverage', label: 'Data coverage' },
        ]}
      />

      <PanelBody style={{ overflow: 'auto' }}>
        {tab === 'stats' && (
          run.result ? (
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit,minmax(260px,1fr))', gap: 'var(--s-5)' }}>
              <StatGroup title="General" values={run.result.stats_general} />
              <StatGroup title="Returns" values={run.result.stats_returns} />
              {Object.entries(run.result.stats_pnls ?? {}).map(([c, v]) => (
                <StatGroup key={c} title={`P&L (${c})`} values={v} />
              ))}
            </div>
          ) : (
            <EmptyState message="No statistics yet" detail="They appear the moment the run completes." />
          )
        )}

        {tab === 'trades' && (
          <EmptyState
            message="Per-trade detail is not returned yet"
            detail="The engine reports totals; a trade-by-trade ledger will render here as a sortable table when the result payload carries one."
          />
        )}

        {tab === 'monthly' && (
          <EmptyState
            message="Monthly returns need a per-bar equity series"
            detail="Once the engine returns one, this renders as a diverging heat map — never the categorical palette."
          />
        )}

        {tab === 'drawdown' && (
          maxDd !== undefined ? (
            <div style={{ maxWidth: 520 }}>
              <KeyValue k="Max drawdown" v={<span className="neg">{signedPct(-Math.abs(maxDd))}</span>} />
              <div style={{ height: 'var(--s-3)' }} />
              <Label>Against a 20% tolerance</Label>
              <Meter value={Math.abs(maxDd)} max={20} tone={Math.abs(maxDd) < 10 ? 'pos' : Math.abs(maxDd) < 20 ? 'warn' : 'neg'} ariaLabel="Drawdown" />
              <div className="mut" style={{ fontSize: 'var(--t-12)', marginTop: 'var(--s-3)', lineHeight: 1.5 }}>
                An underwater curve renders here once the engine returns a per-bar equity series.
              </div>
            </div>
          ) : (
            <EmptyState message="No drawdown recorded" />
          )
        )}

        {tab === 'coverage' && (
          run.coverage ? (
            <div style={{ maxWidth: 560 }}>
              <KeyValue k="Expected bars" v={run.coverage.expected_bars.toLocaleString()} />
              <KeyValue k="Present bars" v={run.coverage.present_bars.toLocaleString()} />
              <KeyValue k="Collected during the run" v={run.coverage.collected_bars.toLocaleString()} />
              <KeyValue k="Gaps" v={String(run.coverage.missing_ranges.length)} />
              {run.coverage.missing_ranges.length > 0 && (
                <div className="callout warn" style={{ marginTop: 'var(--s-3)' }}>
                  <AlertTriangle size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
                  <div>
                    <strong>Missing ranges</strong>
                    <div style={{ marginTop: 4 }}>
                      {run.coverage.missing_ranges.slice(0, 8).map((r, i) => (
                        <div key={i} className="mono" style={{ fontSize: 'var(--t-11)' }}>
                          {shortDate(r.from)} {'→'} {shortDate(r.to)}
                        </div>
                      ))}
                    </div>
                  </div>
                </div>
              )}
            </div>
          ) : (
            <EmptyState message="No coverage report" detail="Coverage is recorded when a run checks its data before simulating." />
          )
        )}
      </PanelBody>
    </Panel>
  )
}

function StatGroup({ title, values }: { title: string; values?: Record<string, number> }) {
  const entries = Object.entries(values ?? {})
  if (entries.length === 0) return null
  return (
    <div>
      <Label>{title}</Label>
      <div style={{ marginTop: 'var(--s-2)' }}>
        {entries.map(([k, v]) => (
          <KeyValue key={k} k={k} v={typeof v === 'number' ? ratio(v, 4) : String(v)} />
        ))}
      </div>
    </div>
  )
}

/* =============================================================================
   RIGOROUS MODE — the honest-evaluation suite (Set J)
   ============================================================================= */
function RigorousMode() {
  const [selected, setSelected] = useState<string | null>(null)
  const [progressByExp, setProgress] = useState<Record<string, SuiteProgress>>({})
  const [creating, setCreating] = useState(false)

  const experiments = useQuery({
    queryKey: ['suite', 'experiments'],
    queryFn: () => experimentsApi.list().then((r) => r.data.experiments),
    refetchInterval: 8000,
  })

  useSuiteProgress((p) => {
    setProgress((prev) => ({ ...prev, [p.experiment_id]: p }))
    void experiments.refetch()
  })

  const list = experiments.data ?? []
  const selectedExp = list.find((e) => e.id === selected) ?? null

  return (
    <div className="page-body" style={{ minHeight: 0 }}>
      <CalibrationCard />
      <FleetBoard experiments={list} progress={progressByExp} />

      <div style={{ display: 'grid', gridTemplateColumns: '340px minmax(0,1fr)', gap: 'var(--s-3)', alignItems: 'start' }}>
        <Panel>
          <PanelHeader
            title="Experiments"
            actions={
              <Button size="xs" variant="primary" onClick={() => setCreating(true)}>
                New
              </Button>
            }
          />
          <PanelBody flush style={{ maxHeight: 520, overflow: 'auto' }}>
            {experiments.isLoading ? (
              <div style={{ padding: 'var(--s-4)' }}><PanelLoading lines={4} /></div>
            ) : list.length === 0 ? (
              <EmptyState
                icon={<ShieldCheck size={20} aria-hidden />}
                message="No experiments yet"
                detail="An experiment tracks every search you spend against one hypothesis, so a result that looks good can be told apart from one that is good."
                action={<Button size="sm" variant="primary" onClick={() => setCreating(true)}>Create one</Button>}
              />
            ) : (
              list.map((e) => <ExperimentRow key={e.id} exp={e} active={e.id === selected} onClick={() => setSelected(e.id)} />)
            )}
          </PanelBody>
        </Panel>

        <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)', minWidth: 0 }}>
          {selectedExp ? (
            <>
              <RunStudyPanel expId={selectedExp.id} onRan={() => void experiments.refetch()} />
              <ExperimentDetail exp={selectedExp} />
            </>
          ) : (
            <Panel>
              <PanelBody>
                <EmptyState
                  message="Select an experiment"
                  detail="Its nulls, gate funnel, significance, vault and every study it has run appear here."
                />
              </PanelBody>
            </Panel>
          )}
        </div>
      </div>

      {creating && (
        <CreateExperimentDialog
          onClose={() => setCreating(false)}
          onCreated={(id) => {
            setCreating(false)
            setSelected(id)
            void experiments.refetch()
          }}
        />
      )}
    </div>
  )
}

const STATE_TONE: Record<string, 'pos' | 'neg' | 'warn' | 'accent' | 'neutral'> = {
  candidate: 'accent',
  validated: 'pos',
  live: 'pos',
  decaying: 'warn',
  retired: 'neutral',
}

function ExperimentRow({ exp, active, onClick }: { exp: ExperimentView; active: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        display: 'block',
        width: '100%',
        textAlign: 'left',
        padding: '10px var(--s-4)',
        borderBottom: '1px solid var(--line-hairline)',
        background: active ? 'var(--bg-selected)' : 'transparent',
        boxShadow: active ? 'inset 2px 0 0 var(--line-accent)' : undefined,
      }}
    >
      <div className="row" style={{ gap: 'var(--s-2)' }}>
        <span className="truncate-1" style={{ fontSize: 'var(--t-13)', fontWeight: 'var(--w-semibold)' }}>
          {exp.experiment_id}
        </span>
        {exp.unsafe && (
          <Badge tone="neg">
            <AlertTriangle size={9} aria-hidden />
            unsafe
          </Badge>
        )}
        <span className="spacer" />
        <Badge tone={STATE_TONE[exp.state] ?? 'neutral'}>{exp.state}</Badge>
      </div>
      <div className="mut" style={{ fontSize: 'var(--t-11)', marginTop: 3 }}>
        {exp.strategy_family} {'·'} <span className="num">{exp.trial_counter.toLocaleString()}</span> searches used
      </div>
    </button>
  )
}

function CalibrationCard() {
  const calibration = useQuery({
    queryKey: ['suite', 'calibration'],
    queryFn: () => experimentsApi.calibration().then((r) => r.data),
    refetchInterval: 15000,
  })
  const c = calibration.data
  if (!c || c.calibration.n_points === 0) return null
  return (
    <Panel>
      <PanelHeader
        title="Suite calibration"
        actions={
          c.calibration.optimistic ? (
            <Badge tone="warn">
              <AlertTriangle size={9} aria-hidden />
              systematically optimistic
            </Badge>
          ) : (
            <Badge tone="pos">calibrated</Badge>
          )
        }
      />
      <PanelBody>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3,minmax(0,1fr))', gap: 'var(--s-4)' }}>
          <StatTile label="Mean realised percentile" value={<span className="num">{ratio(c.calibration.mean_percentile)}</span>} />
          <StatTile label="Worst-5% coverage" value={<span className="num">{pct(c.calibration.worst5_coverage * 100, 0)}</span>} />
          <StatTile label="Experiments contributing" value={<span className="num">{c.experiments_contributing}</span>} />
        </div>
      </PanelBody>
    </Panel>
  )
}

function CreateExperimentDialog({ onClose, onCreated }: { onClose: () => void; onCreated: (id: string) => void }) {
  const toast = useToast()
  const [form, setForm] = useState({
    experiment_id: '',
    strategy_family: '',
    strategy_type: 'daily_trend',
    universe_ref: 'BTC-USD',
    research_start: '2020-01-01',
    research_end: '2022-01-01',
    holdout_start: '2023-01-01',
    holdout_end: '2024-01-01',
  })

  const create = useMutation({
    mutationFn: () =>
      experimentsApi.create({
        ...form,
        research_start: new Date(form.research_start).toISOString(),
        research_end: new Date(form.research_end).toISOString(),
        holdout_start: new Date(form.holdout_start).toISOString(),
        holdout_end: new Date(form.holdout_end).toISOString(),
      }),
    onSuccess: (r) => {
      toast({ title: 'Experiment created' })
      onCreated((r.data as { id: string }).id)
    },
    onError: () => toast({ title: 'Could not create the experiment', variant: 'error' }),
  })

  const set = (k: keyof typeof form) => (e: React.ChangeEvent<HTMLInputElement>) =>
    setForm((f) => ({ ...f, [k]: e.target.value }))

  const valid = form.experiment_id.trim() && form.strategy_family.trim()

  return (
    <div className="scrim" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal wide" role="dialog" aria-modal="true" aria-label="New experiment">
        <div className="modal-hd">
          <div className="h2">New experiment</div>
          <span className="spacer" />
          <Label>Reserved data is spent once, and only once</Label>
        </div>
        <div className="modal-bd" style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-4)' }}>
          <Input label="Name" value={form.experiment_id} onChange={set('experiment_id')} placeholder="btc-trend-v1" />
          <Input label="Strategy family" value={form.strategy_family} onChange={set('strategy_family')} placeholder="trend_following" />
          <Input label="Strategy type" value={form.strategy_type} onChange={set('strategy_type')} />
          <Input label="Universe" value={form.universe_ref} onChange={set('universe_ref')} />
          <Input label="Research start" type="date" value={form.research_start} onChange={set('research_start')} />
          <Input label="Research end" type="date" value={form.research_end} onChange={set('research_end')} />
          <Input label="Reserved start" type="date" value={form.holdout_start} onChange={set('holdout_start')} />
          <Input label="Reserved end" type="date" value={form.holdout_end} onChange={set('holdout_end')} />
        </div>
        <div className="modal-ft">
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" disabled={!valid} loading={create.isPending} onClick={() => create.mutate()}>
            Create experiment
          </Button>
        </div>
      </div>
    </div>
  )
}

