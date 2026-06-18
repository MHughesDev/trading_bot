/**
 * MLOps — Training Data workbench (/mlops/data).
 *
 * Read-only composition of existing Set-I data endpoints so users can inspect
 * the data a training run would consume *before* committing to a run:
 *   - Data quality   GET  /api/models/data/quality   (gaps, dupes, outliers)
 *   - Walk-forward   POST /api/models/data/windows    (fold geometry preview)
 *   - Feature library GET /api/models/feature-sets     (registered feature sets)
 */

import { useState } from 'react'
import { useQuery, useMutation } from '@tanstack/react-query'
import { api as client, assetApi } from '@/lib/api'
import {
  Database, Loader2, GaugeCircle, LayoutList, ScanLine,
  CheckCircle2, AlertTriangle,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { MLOpsSubNav } from '@/components/mlops/MLOpsSubNav'
import { cn } from '@/lib/utils'

const TIMEFRAMES = ['1m', '5m', '15m', '1h', '4h', '1d']

// ── Types ────────────────────────────────────────────────────────────────────

interface DataQualityResponse {
  instrument: string
  timeframe: string
  start: string
  end: string
  bar_count: number
  coverage_pct: number
  gaps: unknown[]
  dupes: unknown[]
  outliers: unknown[]
}

interface Fold {
  fold: number
  train: { start: number; end: number; len: number }
  cal: { start: number; end: number; len: number }
  test: { start: number; end: number; len: number }
}

interface WindowsResponse {
  row_count: number
  horizon_bars: number
  folds: Fold[]
}

interface FeatureSet {
  id?: string
  ref?: string
  name?: string
  display_name?: string
  description?: string
  features?: string[]
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function isoDaysAgo(days: number) {
  return new Date(Date.now() - days * 86_400_000).toISOString().slice(0, 16)
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-xs text-text-muted">{label}</span>
      {children}
    </label>
  )
}

const inputCls =
  'rounded-lg border border-border bg-surface-2 px-3 py-2 text-sm text-text focus:border-accent focus:outline-none'

// ── Data Quality card ────────────────────────────────────────────────────────

function DataQualityCard({ instruments }: { instruments: string[] }) {
  const [instrument, setInstrument] = useState(instruments[0] ?? 'BTC-USD')
  const [timeframe, setTimeframe] = useState('1m')
  const [start, setStart] = useState(isoDaysAgo(7))
  const [end, setEnd] = useState(isoDaysAgo(0))

  const mut = useMutation({
    mutationFn: () =>
      client
        .get<DataQualityResponse>('/api/models/data/quality', {
          params: {
            instrument,
            timeframe,
            start: new Date(start).toISOString(),
            end: new Date(end).toISOString(),
          },
        })
        .then((r) => r.data),
  })

  const r = mut.data

  return (
    <section className="rounded-xl border border-border bg-surface p-5">
      <div className="flex items-center gap-2 mb-4">
        <GaugeCircle className="h-4.5 w-4.5 text-accent" />
        <h2 className="text-sm font-semibold text-text">Data Quality</h2>
        <span className="text-xs text-text-muted">
          Inspect gaps, duplicates, and outliers in the source bars
        </span>
      </div>

      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 mb-4">
        <Field label="Instrument">
          {instruments.length > 0 ? (
            <select className={inputCls} value={instrument} onChange={(e) => setInstrument(e.target.value)}>
              {instruments.map((s) => (
                <option key={s} value={s}>{s}</option>
              ))}
            </select>
          ) : (
            <input className={inputCls} value={instrument} onChange={(e) => setInstrument(e.target.value)} />
          )}
        </Field>
        <Field label="Timeframe">
          <select className={inputCls} value={timeframe} onChange={(e) => setTimeframe(e.target.value)}>
            {TIMEFRAMES.map((t) => (
              <option key={t} value={t}>{t}</option>
            ))}
          </select>
        </Field>
        <Field label="Start">
          <input type="datetime-local" className={inputCls} value={start} onChange={(e) => setStart(e.target.value)} />
        </Field>
        <Field label="End">
          <input type="datetime-local" className={inputCls} value={end} onChange={(e) => setEnd(e.target.value)} />
        </Field>
      </div>

      <Button size="sm" onClick={() => mut.mutate()} disabled={mut.isPending}>
        {mut.isPending ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ScanLine className="h-3.5 w-3.5" />}
        Check quality
      </Button>

      {mut.isError && (
        <p className="mt-3 text-xs text-red-400">
          {(mut.error as { response?: { data?: { error?: string } } })?.response?.data?.error ??
            'Quality check failed'}
        </p>
      )}

      {r && (
        <div className="mt-4 grid grid-cols-2 sm:grid-cols-4 gap-3">
          <Stat label="Bars" value={r.bar_count.toLocaleString()} />
          <Stat
            label="Coverage"
            value={`${r.coverage_pct.toFixed(1)}%`}
            tone={r.coverage_pct >= 99 ? 'good' : r.coverage_pct >= 95 ? 'warn' : 'bad'}
          />
          <Stat label="Gaps" value={String(r.gaps.length)} tone={r.gaps.length === 0 ? 'good' : 'warn'} />
          <Stat label="Dupes" value={String(r.dupes.length)} tone={r.dupes.length === 0 ? 'good' : 'warn'} />
          <Stat label="Outliers" value={String(r.outliers.length)} tone={r.outliers.length === 0 ? 'good' : 'warn'} />
        </div>
      )}
    </section>
  )
}

function Stat({ label, value, tone }: { label: string; value: string; tone?: 'good' | 'warn' | 'bad' }) {
  const toneCls =
    tone === 'good' ? 'text-emerald-400' : tone === 'warn' ? 'text-amber-400' : tone === 'bad' ? 'text-red-400' : 'text-text'
  return (
    <div className="rounded-lg bg-surface-2 p-3">
      <div className="text-xs text-text-muted mb-1">{label}</div>
      <div className={cn('font-mono text-sm font-medium', toneCls)}>{value}</div>
    </div>
  )
}

// ── Walk-forward windows card ────────────────────────────────────────────────

function WalkForwardCard() {
  const [folds, setFolds] = useState(4)
  const [trainBars, setTrainBars] = useState(5000)
  const [calBars, setCalBars] = useState(500)
  const [testBars, setTestBars] = useState(1000)
  const [embargoBars, setEmbargoBars] = useState(60)
  const [rowCount, setRowCount] = useState(30000)
  const [timeframe, setTimeframe] = useState('1m')
  const [horizon, setHorizon] = useState('1h')

  const mut = useMutation({
    mutationFn: () =>
      client
        .post<WindowsResponse>('/api/models/data/windows', {
          spec: {
            mode: 'expanding',
            folds,
            train_bars: trainBars,
            cal_bars: calBars,
            test_bars: testBars,
            purge_bars: 0,
            embargo_bars: embargoBars,
          },
          row_count: rowCount,
          horizon_token: horizon,
          timeframe,
        })
        .then((r) => r.data),
  })

  const r = mut.data

  return (
    <section className="rounded-xl border border-border bg-surface p-5">
      <div className="flex items-center gap-2 mb-4">
        <LayoutList className="h-4.5 w-4.5 text-accent" />
        <h2 className="text-sm font-semibold text-text">Walk-Forward Windows</h2>
        <span className="text-xs text-text-muted">
          Preview train / calibration / test fold geometry before training
        </span>
      </div>

      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 mb-4">
        <Field label="Folds"><input type="number" className={inputCls} value={folds} onChange={(e) => setFolds(+e.target.value)} /></Field>
        <Field label="Train bars"><input type="number" className={inputCls} value={trainBars} onChange={(e) => setTrainBars(+e.target.value)} /></Field>
        <Field label="Cal bars"><input type="number" className={inputCls} value={calBars} onChange={(e) => setCalBars(+e.target.value)} /></Field>
        <Field label="Test bars"><input type="number" className={inputCls} value={testBars} onChange={(e) => setTestBars(+e.target.value)} /></Field>
        <Field label="Embargo bars"><input type="number" className={inputCls} value={embargoBars} onChange={(e) => setEmbargoBars(+e.target.value)} /></Field>
        <Field label="Row count"><input type="number" className={inputCls} value={rowCount} onChange={(e) => setRowCount(+e.target.value)} /></Field>
        <Field label="Timeframe">
          <select className={inputCls} value={timeframe} onChange={(e) => setTimeframe(e.target.value)}>
            {TIMEFRAMES.map((t) => <option key={t} value={t}>{t}</option>)}
          </select>
        </Field>
        <Field label="Label horizon"><input className={inputCls} value={horizon} onChange={(e) => setHorizon(e.target.value)} /></Field>
      </div>

      <Button size="sm" onClick={() => mut.mutate()} disabled={mut.isPending}>
        {mut.isPending ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <LayoutList className="h-3.5 w-3.5" />}
        Preview folds
      </Button>

      {mut.isError && (
        <p className="mt-3 text-xs text-red-400">
          {(mut.error as { response?: { data?: { error?: string } } })?.response?.data?.error ??
            'Fold preview failed'}
        </p>
      )}

      {r && (
        <div className="mt-4 space-y-2">
          <p className="text-xs text-text-muted">
            {r.folds.length} folds · horizon {r.horizon_bars} bars · {r.row_count.toLocaleString()} rows
          </p>
          {r.folds.map((f) => {
            const total = r.row_count || 1
            const seg = (a: number, b: number) => ({ left: `${(a / total) * 100}%`, width: `${((b - a) / total) * 100}%` })
            return (
              <div key={f.fold} className="flex items-center gap-3">
                <span className="text-xs text-text-muted w-12 shrink-0">#{f.fold}</span>
                <div className="relative h-4 flex-1 rounded bg-surface-2 overflow-hidden">
                  <div className="absolute h-full bg-blue-500/50" style={seg(f.train.start, f.train.end)} title={`train ${f.train.len}`} />
                  <div className="absolute h-full bg-amber-500/60" style={seg(f.cal.start, f.cal.end)} title={`cal ${f.cal.len}`} />
                  <div className="absolute h-full bg-emerald-500/60" style={seg(f.test.start, f.test.end)} title={`test ${f.test.len}`} />
                </div>
              </div>
            )
          })}
          <div className="flex items-center gap-4 text-xs text-text-muted pt-1">
            <span className="flex items-center gap-1"><span className="h-2 w-2 rounded-sm bg-blue-500/50" /> train</span>
            <span className="flex items-center gap-1"><span className="h-2 w-2 rounded-sm bg-amber-500/60" /> cal</span>
            <span className="flex items-center gap-1"><span className="h-2 w-2 rounded-sm bg-emerald-500/60" /> test</span>
          </div>
        </div>
      )}
    </section>
  )
}

// ── Feature library card ─────────────────────────────────────────────────────

function FeatureLibraryCard() {
  const { data, isLoading } = useQuery({
    queryKey: ['feature-sets'],
    queryFn: () =>
      client
        .get<{ feature_sets: FeatureSet[]; registry_version: string | number }>('/api/models/feature-sets')
        .then((r) => r.data),
  })

  const sets = data?.feature_sets ?? []

  return (
    <section className="rounded-xl border border-border bg-surface p-5">
      <div className="flex items-center gap-2 mb-4">
        <Database className="h-4.5 w-4.5 text-accent" />
        <h2 className="text-sm font-semibold text-text">Feature Library</h2>
        {data?.registry_version != null && (
          <span className="text-xs text-text-muted font-mono">registry v{String(data.registry_version)}</span>
        )}
      </div>

      {isLoading ? (
        <div className="flex items-center gap-2 text-xs text-text-muted py-4">
          <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading feature sets…
        </div>
      ) : sets.length === 0 ? (
        <p className="text-xs text-text-muted py-4">No feature sets registered.</p>
      ) : (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          {sets.map((fs, i) => {
            const name = fs.display_name ?? fs.name ?? fs.ref ?? fs.id ?? `Feature set ${i + 1}`
            const ref = fs.ref ?? fs.id
            return (
              <div key={ref ?? i} className="rounded-lg border border-border bg-surface-2 p-3">
                <div className="flex items-center gap-2">
                  <CheckCircle2 className="h-3.5 w-3.5 text-emerald-400 shrink-0" />
                  <span className="text-sm font-medium text-text truncate">{name}</span>
                </div>
                {ref && <div className="text-xs text-text-muted font-mono mt-0.5 truncate">{ref}</div>}
                {fs.description && <p className="text-xs text-text-muted mt-1 line-clamp-2">{fs.description}</p>}
                {fs.features && (
                  <p className="text-xs text-text-dim mt-1">{fs.features.length} features</p>
                )}
              </div>
            )
          })}
        </div>
      )}
    </section>
  )
}

// ── Page ─────────────────────────────────────────────────────────────────────

export function DataWorkbenchPage() {
  const { data: assetsData } = useQuery({
    queryKey: ['initialized-assets'],
    queryFn: () => assetApi.initialized().then((r) => r.data),
  })
  const instruments = (assetsData?.assets ?? []).map((a) => a.symbol)

  return (
    <div className="mx-auto w-full max-w-5xl px-6 py-6">
      <div className="mb-4 flex items-center gap-2">
        <Database className="h-6 w-6 text-accent" />
        <div>
          <h1 className="text-2xl font-semibold text-text">MLOps</h1>
          <p className="text-sm text-text-muted">Inspect and prepare the data your models train on.</p>
        </div>
      </div>

      <MLOpsSubNav />

      <div className="flex items-start gap-2 rounded-lg border border-border bg-surface-2/50 p-3 mb-5 text-xs text-text-muted">
        <AlertTriangle className="h-3.5 w-3.5 text-text-dim shrink-0 mt-0.5" />
        Everything here is read-only — previews never mutate datasets or kick off training.
      </div>

      <div className="space-y-5">
        <DataQualityCard instruments={instruments} />
        <WalkForwardCard />
        <FeatureLibraryCard />
      </div>
    </div>
  )
}
