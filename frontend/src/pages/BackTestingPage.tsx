import { useMemo, useState } from 'react'
import { useQuery, useMutation } from '@tanstack/react-query'
import { Plus, FlaskConical, Loader2, ShieldCheck } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { backtestsApi, type BacktestSnapshot } from '@/api/backtests'
import { experimentsApi, type ExperimentView } from '@/api/experiments'
import { isActive } from '@/components/backtest/status'
import { BacktestTile } from '@/components/backtest/BacktestTile'
import { CreateBacktestDialog } from '@/components/backtest/CreateBacktestDialog'
import { BacktestDetailsDialog } from '@/components/backtest/BacktestDetailsDialog'
import { FleetBoard } from '@/components/proving-ground/FleetBoard'
import { RunStudyPanel } from '@/components/proving-ground/RunStudyPanel'
import { ExperimentDetail } from '@/components/workbench/ExperimentDetail'
import { Badge } from '@/components/ui/badge'
import { AlertTriangle } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { toast } from '@/hooks/useToast'
import { useSuiteProgress, type SuiteProgress } from '@/hooks/useSuiteProgress'

type Mode = 'simple' | 'rigorous'

const STATE_VARIANT: Record<string, 'default' | 'active' | 'inactive' | 'warning'> = {
  candidate: 'default',
  validated: 'active',
  live: 'active',
  decaying: 'warning',
  retired: 'inactive',
}

export function BackTestingPage() {
  const [mode, setMode] = useState<Mode>('simple')

  return (
    <div className="mx-auto w-full max-w-7xl px-6 py-6">
      <header className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="flex items-center gap-2 text-2xl font-semibold text-text">
            <FlaskConical className="h-6 w-6 text-blue-400" />
            Back Testing
          </h1>
          <p className="mt-1 text-sm text-text-muted">
            Simulate strategies against historical data.
          </p>
        </div>
        <ModeSwitcher mode={mode} onChange={setMode} />
      </header>

      {mode === 'simple' ? <SimpleMode /> : <RigorousMode />}
    </div>
  )
}

function ModeSwitcher({ mode, onChange }: { mode: Mode; onChange: (m: Mode) => void }) {
  return (
    <div className="flex items-center gap-1 rounded-full border border-border bg-surface-2 p-1">
      <ModeButton active={mode === 'simple'} onClick={() => onChange('simple')}>
        Simple
      </ModeButton>
      <ModeButton active={mode === 'rigorous'} onClick={() => onChange('rigorous')}>
        <ShieldCheck className="h-3.5 w-3.5" />
        Rigorous
      </ModeButton>
    </div>
  )
}

function ModeButton({
  active,
  onClick,
  children,
}: {
  active: boolean
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`flex items-center gap-1.5 rounded-full px-4 py-1.5 text-sm font-medium transition-colors ${
        active
          ? 'bg-surface text-text shadow-sm border border-border'
          : 'text-text-muted hover:text-text'
      }`}
    >
      {children}
    </button>
  )
}

// ── Simple mode ──────────────────────────────────────────────────────────────
// Just run a backtest against a strategy. No trial counter, no significance
// testing. Good for quick sanity checks during development.

function SimpleMode() {
  const [createOpen, setCreateOpen] = useState(false)
  const [selected, setSelected] = useState<BacktestSnapshot | null>(null)

  const { data, isLoading } = useQuery({
    queryKey: ['backtests'],
    queryFn: () => backtestsApi.list().then((r) => r.data.backtests),
    refetchInterval: (q) =>
      (q.state.data ?? []).some((r) => isActive(r.status)) ? 1500 : 8000,
  })

  const runs = useMemo(() => data ?? [], [data])
  const runningCount = useMemo(
    () => runs.filter((r) => isActive(r.status)).length,
    [runs],
  )

  return (
    <>
      <div className="mb-4 flex items-center justify-between">
        <p className="text-sm text-text-muted">
          Quick runs against historical data — no search counter, no significance testing.
          {runningCount > 0 && (
            <span className="ml-1 text-blue-400">{runningCount} running.</span>
          )}
        </p>
        <Button onClick={() => setCreateOpen(true)}>
          <Plus className="h-4 w-4" />
          New backtest
        </Button>
      </div>

      {isLoading ? (
        <div className="flex items-center justify-center py-20 text-text-dim">
          <Loader2 className="h-6 w-6 animate-spin" />
        </div>
      ) : runs.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-3 rounded-lg border border-dashed border-border py-20 text-center">
          <FlaskConical className="h-10 w-10 text-text-dim" />
          <div className="text-text-muted">No backtests yet.</div>
          <Button variant="outline" onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4" />
            Run your first backtest
          </Button>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
          {runs.map((run) => (
            <BacktestTile key={run.id} run={run} onOpen={setSelected} />
          ))}
        </div>
      )}

      <CreateBacktestDialog open={createOpen} onOpenChange={setCreateOpen} />
      <BacktestDetailsDialog
        runId={selected?.id ?? null}
        fallback={selected}
        open={!!selected}
        onOpenChange={(o) => !o && setSelected(null)}
      />
    </>
  )
}

// ── Rigorous mode ─────────────────────────────────────────────────────────────
// Full significance-testing pipeline. Every run increments the search counter
// and raises the bar the strategy must clear. Includes randomized baseline
// selection, validation checkpoints, and a one-shot final test on reserved data.

function RigorousMode() {
  const [selected, setSelected] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const [progressByExp, setProgressByExp] = useState<Record<string, SuiteProgress>>({})

  const experiments = useQuery({
    queryKey: ['suite', 'experiments'],
    queryFn: () => experimentsApi.list().then((r) => r.data.experiments),
    refetchInterval: 8000,
  })
  useSuiteProgress((p) => {
    setProgressByExp((m) => ({ ...m, [p.experiment_id]: p }))
    experiments.refetch()
  })

  const selectedExp = experiments.data?.find((e) => e.id === selected) ?? null

  return (
    <>
      <div className="mb-4 flex items-center justify-between">
        <p className="text-sm text-text-muted">
          Every run counts against your search total. Strategy must beat a randomized
          baseline and pass all checkpoints to be considered significant.
        </p>
        <Button onClick={() => setCreating(true)}>
          <Plus className="h-4 w-4" />
          New strategy test
        </Button>
      </div>

      <FleetBoard experiments={experiments.data ?? []} progress={progressByExp} />

      <div className="mt-4 grid grid-cols-1 gap-4 lg:grid-cols-[20rem_1fr]">
        <div className="flex flex-col gap-2">
          {experiments.isLoading ? (
            <div className="text-sm text-text-dim">Loading…</div>
          ) : experiments.data && experiments.data.length > 0 ? (
            experiments.data.map((e) => (
              <StrategyTestRow
                key={e.id}
                exp={e}
                active={e.id === selected}
                onClick={() => setSelected(e.id)}
              />
            ))
          ) : (
            <div className="rounded-md border border-dashed border-border-2 p-4 text-sm text-text-dim">
              No strategy tests yet. Create one to begin.
            </div>
          )}
        </div>

        <div className="flex flex-col gap-4">
          {selectedExp ? (
            <>
              <RunStudyPanel expId={selectedExp.id} onRan={() => experiments.refetch()} />
              <ExperimentDetail exp={selectedExp} />
            </>
          ) : (
            <div className="flex h-64 items-center justify-center rounded-lg border border-dashed border-border-2 text-sm text-text-dim">
              Select a strategy test to view its results.
            </div>
          )}
        </div>
      </div>

      {creating && (
        <CreateStrategyTestDialog
          onClose={() => setCreating(false)}
          onCreated={(id) => {
            setCreating(false)
            setSelected(id)
            experiments.refetch()
          }}
        />
      )}
    </>
  )
}

function StrategyTestRow({
  exp,
  active,
  onClick,
}: {
  exp: ExperimentView
  active: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`flex flex-col gap-1 rounded-md border p-3 text-left transition-colors ${
        active ? 'border-accent bg-accent/10' : 'border-border-2 bg-surface hover:bg-surface-2'
      }`}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="truncate text-sm font-medium text-text">{exp.experiment_id}</span>
        {exp.unsafe && (
          <Badge variant="destructive" className="gap-1">
            <AlertTriangle className="h-3 w-3" />
            unsafe
          </Badge>
        )}
      </div>
      <div className="flex items-center justify-between gap-2 text-xs">
        <Badge variant={STATE_VARIANT[exp.state] ?? 'default'}>{exp.state}</Badge>
        <span className="text-text-dim">
          <span className="font-semibold tabular-nums text-text-muted">
            {exp.trial_counter.toLocaleString()}
          </span>{' '}
          searches used
        </span>
      </div>
    </button>
  )
}

function CreateStrategyTestDialog({
  onClose,
  onCreated,
}: {
  onClose: () => void
  onCreated: (id: string) => void
}) {
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
        experiment_id: form.experiment_id,
        strategy_family: form.strategy_family,
        strategy_type: form.strategy_type,
        universe_ref: form.universe_ref,
        research_start: new Date(form.research_start).toISOString(),
        research_end: new Date(form.research_end).toISOString(),
        holdout_start: new Date(form.holdout_start).toISOString(),
        holdout_end: new Date(form.holdout_end).toISOString(),
      }),
    onSuccess: (r) => {
      toast({ title: 'Strategy test created' })
      onCreated(r.data.id)
    },
    onError: (e: unknown) => {
      const msg =
        (e as { response?: { data?: { message?: string } } })?.response?.data?.message ??
        'Could not create strategy test'
      toast({ title: msg, variant: 'error' })
    },
  })

  const set = (k: keyof typeof form) => (e: React.ChangeEvent<HTMLInputElement>) =>
    setForm((f) => ({ ...f, [k]: e.target.value }))
  const valid = form.experiment_id.trim() && form.strategy_family.trim()

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4" onClick={onClose}>
      <Card className="w-full max-w-lg" onClick={(e) => e.stopPropagation()}>
        <CardHeader>
          <CardTitle className="text-base">New strategy test</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <Field label="Name" value={form.experiment_id} onChange={set('experiment_id')} />
          <Field label="Strategy family" value={form.strategy_family} onChange={set('strategy_family')} />
          <Field label="Strategy type (sets recommended baseline)" value={form.strategy_type} onChange={set('strategy_type')} />
          <Field label="Asset" value={form.universe_ref} onChange={set('universe_ref')} />
          <div className="grid grid-cols-2 gap-3">
            <Field label="Research start" type="date" value={form.research_start} onChange={set('research_start')} />
            <Field label="Research end" type="date" value={form.research_end} onChange={set('research_end')} />
            <Field label="Reserved data start" type="date" value={form.holdout_start} onChange={set('holdout_start')} />
            <Field label="Reserved data end" type="date" value={form.holdout_end} onChange={set('holdout_end')} />
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={onClose}>Cancel</Button>
            <Button disabled={!valid || create.isPending} onClick={() => create.mutate()}>
              {create.isPending && <Loader2 className="h-3 w-3 animate-spin" />}
              Create
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  )
}

function Field({
  label,
  value,
  onChange,
  type = 'text',
}: {
  label: string
  value: string
  onChange: (e: React.ChangeEvent<HTMLInputElement>) => void
  type?: string
}) {
  return (
    <label className="flex flex-col gap-1 text-xs text-text-dim">
      {label}
      <input
        type={type}
        value={value}
        onChange={onChange}
        className="rounded-md border border-border-2 bg-surface px-2 py-1.5 text-sm text-text"
      />
    </label>
  )
}
