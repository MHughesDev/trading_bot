import { useState, useEffect, useRef, useMemo } from 'react'
import { Play, Square, Loader2, Database } from 'lucide-react'
import * as Progress from '@radix-ui/react-progress'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import {
  useModel,
  useModelRuns,
  useStartTrain,
  useCancelRun,
  useMarketInstruments,
} from '@/hooks/useMlOps'
import type { ModelRunSnapshot } from '@/api/mlops'
import { formatDistanceToNow } from 'date-fns'
import { HyperparamEditor } from './HyperparamEditor'

interface Props {
  modelId: string
}

const LOOKBACK_OPTIONS = [
  { label: '7 days', value: 7 },
  { label: '30 days', value: 30 },
  { label: '90 days', value: 90 },
  { label: '180 days', value: 180 },
]

const LABEL_HORIZONS = ['15m', '1h', '4h', '1d']

function fmtDate(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
}

function elapsed(run: ModelRunSnapshot): string {
  const start = new Date(run.created_at)
  if (run.finished_at) {
    const end = new Date(run.finished_at)
    const secs = Math.round((end.getTime() - start.getTime()) / 1000)
    if (secs < 60) return `${secs}s`
    const mins = Math.floor(secs / 60)
    const rem = secs % 60
    return `${mins}m ${rem}s`
  }
  return formatDistanceToNow(start, { addSuffix: false })
}

function CircularProgress({ value }: { value: number }) {
  const r = 54
  const circumference = 2 * Math.PI * r
  const offset = circumference - (value / 100) * circumference

  return (
    <div className="relative flex items-center justify-center">
      <svg width={128} height={128} className="-rotate-90">
        <circle
          cx={64}
          cy={64}
          r={r}
          fill="none"
          stroke="var(--tb-border)"
          strokeWidth={8}
        />
        <circle
          cx={64}
          cy={64}
          r={r}
          fill="none"
          stroke="var(--tb-accent)"
          strokeWidth={8}
          strokeDasharray={circumference}
          strokeDashoffset={offset}
          strokeLinecap="round"
          style={{ transition: 'stroke-dashoffset 0.4s ease' }}
        />
      </svg>
      <div className="absolute flex flex-col items-center">
        <span className="text-2xl font-bold font-mono text-text">{Math.round(value)}%</span>
      </div>
    </div>
  )
}

export function ModelTrainTab({ modelId }: Props) {
  const [versionNote, setVersionNote] = useState('')
  const [hyperparams, setHyperparams] = useState<Record<string, unknown>>({})
  const [hyperparamsValid, setHyperparamsValid] = useState(true)
  const [logLines, setLogLines] = useState<string[]>([])
  const logEndRef = useRef<HTMLDivElement>(null)

  // Data selection
  const [instrument, setInstrument] = useState<string>('')
  const [timeframe, setTimeframe] = useState<string>('')
  const [lookbackDays, setLookbackDays] = useState<number>(30)
  const [labelHorizon, setLabelHorizon] = useState<string>('1h')

  const { data: model } = useModel(modelId)
  const framework = model?.definition.framework ?? 'xgboost'
  const runtime = model?.definition.runtime ?? 'python'

  const { data: runs, isLoading: runsLoading } = useModelRuns(modelId)
  const { data: marketInstruments } = useMarketInstruments()
  const startMut = useStartTrain(modelId)
  const cancelMut = useCancelRun(modelId)

  // Distinct instruments available, and the timeframes each has.
  const instrumentOptions = useMemo(() => {
    const map = new Map<string, typeof marketInstruments>()
    for (const mi of marketInstruments ?? []) {
      const list = map.get(mi.instrument_id) ?? []
      list.push(mi)
      map.set(mi.instrument_id, list)
    }
    return map
  }, [marketInstruments])

  // Default the selection once data loads.
  useEffect(() => {
    if (!marketInstruments || marketInstruments.length === 0) return
    if (!instrument) {
      const first = marketInstruments[0]
      setInstrument(first.instrument_id)
      setTimeframe(first.timeframe)
    }
  }, [marketInstruments, instrument])

  const timeframesForInstrument = instrumentOptions.get(instrument) ?? []
  const selectedCoverage = timeframesForInstrument.find((m) => m.timeframe === timeframe)

  const activeRun = runs?.find((r) => r.status === 'running')
  const latestRun = runs?.[0]

  // Accumulate log lines from phase changes
  useEffect(() => {
    if (!activeRun) return
    const line = `[${new Date().toLocaleTimeString()}] Phase: ${activeRun.phase} (${Math.round(activeRun.progress)}%)`
    setLogLines((prev) => {
      const last = prev[prev.length - 1]
      // Avoid duplicate lines
      if (last && last.includes(activeRun.phase) && last.includes(Math.round(activeRun.progress).toString())) {
        return prev
      }
      return [...prev.slice(-200), line]
    })
  }, [activeRun?.phase, activeRun?.progress])

  useEffect(() => {
    logEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [logLines])

  async function handleStart() {
    if (!hyperparamsValid) return
    setLogLines([])
    await startMut.mutateAsync({
      version_note: versionNote.trim() || undefined,
      hyperparams: Object.keys(hyperparams).length > 0 ? hyperparams : undefined,
      data: instrument
        ? {
            instruments: [instrument],
            timeframe: timeframe || '1m',
            lookback_days: lookbackDays,
            label_horizon: labelHorizon,
          }
        : undefined,
    })
  }

  async function handleCancel() {
    if (activeRun) {
      await cancelMut.mutateAsync(activeRun.run_id)
    }
  }

  const displayRun = activeRun ?? latestRun

  return (
    <div className="grid grid-cols-1 lg:grid-cols-[280px_1fr_220px] gap-4">
      {/* Left — Config */}
      <div className="rounded-xl border border-border bg-surface p-4 space-y-4">
        <h3 className="text-sm font-medium text-text">Training config</h3>

        {/* Data selection */}
        <div className="space-y-3 rounded-lg border border-border bg-surface-2 p-3">
          <div className="flex items-center gap-1.5">
            <Database className="h-3.5 w-3.5 text-accent" />
            <span className="text-xs font-medium text-text">Training data</span>
          </div>

          {!marketInstruments || marketInstruments.length === 0 ? (
            <p className="text-xs text-text-dim">
              No stored market data found. Initialize an asset first so bars
              accumulate in ClickHouse.
            </p>
          ) : (
            <>
              <div>
                <label className="block text-[11px] font-medium text-text-muted mb-1">
                  Instrument
                </label>
                <select
                  value={instrument}
                  onChange={(e) => {
                    const next = e.target.value
                    setInstrument(next)
                    const tfs = instrumentOptions.get(next) ?? []
                    if (tfs.length && !tfs.some((m) => m.timeframe === timeframe)) {
                      setTimeframe(tfs[0].timeframe)
                    }
                  }}
                  disabled={!!activeRun}
                  className="w-full h-8 rounded-lg border border-border bg-surface px-2 text-xs text-text focus:outline-none focus:border-accent disabled:opacity-50"
                >
                  {[...instrumentOptions.keys()].map((id) => (
                    <option key={id} value={id}>
                      {id}
                    </option>
                  ))}
                </select>
              </div>

              <div className="grid grid-cols-2 gap-2">
                <div>
                  <label className="block text-[11px] font-medium text-text-muted mb-1">
                    Timeframe
                  </label>
                  <select
                    value={timeframe}
                    onChange={(e) => setTimeframe(e.target.value)}
                    disabled={!!activeRun}
                    className="w-full h-8 rounded-lg border border-border bg-surface px-2 text-xs text-text focus:outline-none focus:border-accent disabled:opacity-50"
                  >
                    {timeframesForInstrument.map((m) => (
                      <option key={m.timeframe} value={m.timeframe}>
                        {m.timeframe}
                      </option>
                    ))}
                  </select>
                </div>
                <div>
                  <label className="block text-[11px] font-medium text-text-muted mb-1">
                    Lookback
                  </label>
                  <select
                    value={lookbackDays}
                    onChange={(e) => setLookbackDays(Number(e.target.value))}
                    disabled={!!activeRun}
                    className="w-full h-8 rounded-lg border border-border bg-surface px-2 text-xs text-text focus:outline-none focus:border-accent disabled:opacity-50"
                  >
                    {LOOKBACK_OPTIONS.map((o) => (
                      <option key={o.value} value={o.value}>
                        {o.label}
                      </option>
                    ))}
                  </select>
                </div>
              </div>

              <div>
                <label className="block text-[11px] font-medium text-text-muted mb-1">
                  Label horizon (forward return)
                </label>
                <select
                  value={labelHorizon}
                  onChange={(e) => setLabelHorizon(e.target.value)}
                  disabled={!!activeRun}
                  className="w-full h-8 rounded-lg border border-border bg-surface px-2 text-xs text-text focus:outline-none focus:border-accent disabled:opacity-50"
                >
                  {LABEL_HORIZONS.map((h) => (
                    <option key={h} value={h}>
                      {h}
                    </option>
                  ))}
                </select>
              </div>

              {selectedCoverage && (
                <p className="text-[11px] text-text-dim leading-relaxed">
                  {selectedCoverage.bars.toLocaleString()} bars stored ·{' '}
                  {fmtDate(selectedCoverage.first_ms)} – {fmtDate(selectedCoverage.last_ms)}
                </p>
              )}
            </>
          )}
        </div>

        <div>
          <label className="block text-xs font-medium text-text-muted mb-1.5">
            Version note
          </label>
          <input
            type="text"
            value={versionNote}
            onChange={(e) => setVersionNote(e.target.value)}
            placeholder="e.g. Added dropout layer"
            disabled={!!activeRun}
            className="w-full h-8 rounded-lg border border-border bg-surface px-2.5 text-xs text-text placeholder:text-text-dim focus:outline-none focus:border-accent disabled:opacity-50"
          />
        </div>

        <div className="flex items-center gap-1.5 text-[11px]">
          <span className="rounded-md bg-surface-2 px-2 py-0.5 font-mono text-text-muted">
            {framework}
          </span>
          <span className="rounded-md bg-surface-2 px-2 py-0.5 font-mono text-text-muted">
            {runtime}
          </span>
        </div>

        <HyperparamEditor
          key={framework}
          framework={framework}
          disabled={!!activeRun}
          onChange={(hp, valid) => {
            setHyperparams(hp)
            setHyperparamsValid(valid)
          }}
        />

        <div className="flex gap-2">
          <Button
            className="flex-1 text-xs h-8"
            onClick={handleStart}
            disabled={!!activeRun || startMut.isPending || !hyperparamsValid}
          >
            {startMut.isPending ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Play className="h-3.5 w-3.5" />
            )}
            Start
          </Button>
          <Button
            variant="outline"
            className="text-xs h-8"
            onClick={handleCancel}
            disabled={!activeRun || cancelMut.isPending}
          >
            <Square className="h-3.5 w-3.5" />
            Stop
          </Button>
        </div>

        {startMut.isError && (
          <p className="text-xs text-pnl-down">Failed to start training.</p>
        )}
      </div>

      {/* Center — Progress */}
      <div className="rounded-xl border border-border bg-surface p-6 flex flex-col items-center justify-center gap-4">
        {runsLoading ? (
          <Loader2 className="h-8 w-8 animate-spin text-text-dim" />
        ) : displayRun ? (
          <>
            <CircularProgress value={displayRun.progress} />

            <div className="text-center">
              <p className="text-sm font-medium text-text capitalize">
                {displayRun.status === 'running' ? displayRun.phase : displayRun.status}
              </p>
              <p className="text-xs text-text-muted mt-1">
                Elapsed: <span className="font-mono">{elapsed(displayRun)}</span>
              </p>
            </div>

            {/* Linear progress bar for smaller detail */}
            <div className="w-full max-w-xs">
              <Progress.Root
                value={displayRun.progress}
                className="relative h-1.5 w-full overflow-hidden rounded-full bg-surface-2"
              >
                <Progress.Indicator
                  className="h-full bg-accent transition-transform duration-500 ease-out"
                  style={{ transform: `translateX(-${100 - displayRun.progress}%)` }}
                />
              </Progress.Root>
            </div>

            {displayRun.error && (
              <p className="text-xs text-pnl-down text-center px-4">{displayRun.error}</p>
            )}

            <div className="flex gap-2 flex-wrap justify-center">
              {displayRun.metrics &&
                Object.entries(displayRun.metrics)
                  .slice(0, 4)
                  .map(([k, v]) => (
                    <div
                      key={k}
                      className="rounded-lg bg-surface-2 px-3 py-1.5 text-center"
                    >
                      <p className="text-xs text-text-muted">{k}</p>
                      <p className="text-sm font-mono font-medium text-text">
                        {typeof v === 'number' ? v.toFixed(3) : String(v)}
                      </p>
                    </div>
                  ))}
            </div>
          </>
        ) : (
          <div className="text-center">
            <CircularProgress value={0} />
            <p className="text-sm text-text-muted mt-4">
              No training runs yet. Configure and click Start.
            </p>
          </div>
        )}
      </div>

      {/* Right — Event log */}
      <div className="rounded-xl border border-border bg-surface p-4 flex flex-col">
        <h3 className="text-xs font-medium text-text-muted mb-3 shrink-0">Events</h3>
        <div className="flex-1 overflow-y-auto font-mono text-xs text-text-muted space-y-1 min-h-[200px] max-h-[400px]">
          {logLines.length === 0 ? (
            <p className="text-text-dim italic">Waiting for events…</p>
          ) : (
            logLines.map((line, i) => (
              <p key={i} className="leading-relaxed">
                {line}
              </p>
            ))
          )}
          <div ref={logEndRef} />
        </div>

        {/* Recent runs list */}
        {runs && runs.length > 0 && (
          <div className="mt-4 pt-3 border-t border-border shrink-0">
            <p className="text-xs text-text-dim mb-2">Recent runs</p>
            <div className="space-y-1.5">
              {runs.slice(0, 4).map((run) => (
                <div key={run.run_id} className="flex items-center gap-2 text-xs">
                  <span
                    className={cn(
                      'h-1.5 w-1.5 rounded-full shrink-0',
                      run.status === 'running'
                        ? 'bg-blue-400'
                        : run.status === 'succeeded'
                          ? 'bg-pnl-up'
                          : run.status === 'failed'
                            ? 'bg-pnl-down'
                            : 'bg-text-dim',
                    )}
                  />
                  <span className="font-mono text-text-dim truncate">
                    {run.run_id.slice(0, 8)}
                  </span>
                  <span className="text-text-muted ml-auto capitalize">{run.status}</span>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
