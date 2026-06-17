// Backtest Workbench (Set J, J-5.5–J-5.8). The experiment console lists every
// Experiment with its trial counter and lifecycle state ALWAYS visible (and the
// `unsafe` flag prominent when set); selecting one opens the full apparatus —
// distribution viewers, null picker, gate-funnel board, significance card, vault
// panel — in ExperimentDetail. A suite-calibration meta-view sits at the top.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { AlertTriangle, Loader2, Plus } from 'lucide-react'
import { experimentsApi, type ExperimentView } from '@/api/experiments'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { toast } from '@/hooks/useToast'
import { useSuiteProgress } from '@/hooks/useSuiteProgress'
import { ExperimentDetail } from '@/components/workbench/ExperimentDetail'

const STATE_VARIANT: Record<string, 'default' | 'active' | 'inactive' | 'warning'> = {
  candidate: 'default',
  validated: 'active',
  live: 'active',
  decaying: 'warning',
  retired: 'inactive',
}

export function WorkbenchPage() {
  const qc = useQueryClient()
  const [selected, setSelected] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)

  const experiments = useQuery({
    queryKey: ['suite', 'experiments'],
    queryFn: () => experimentsApi.list().then((r) => r.data.experiments),
    refetchInterval: 8000,
  })

  // Live progress nudges a refresh of the lists/details.
  useSuiteProgress(() => {
    qc.invalidateQueries({ queryKey: ['suite'] })
  })

  const selectedExp = experiments.data?.find((e) => e.id === selected) ?? null

  return (
    <div className="mx-auto flex w-full max-w-7xl flex-col gap-4 px-6 py-6">
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold text-text">Backtest Workbench</h1>
          <p className="text-xs text-text-dim">
            Honest evaluation: every result carries its trial count, distributions are sealed, and
            significance never appears without its null.
          </p>
        </div>
        <Button onClick={() => setCreating(true)}>
          <Plus className="h-4 w-4" />
          New experiment
        </Button>
      </header>

      <CalibrationCard />

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[20rem_1fr]">
        {/* Experiment console — counter + lifecycle always on screen. */}
        <div className="flex flex-col gap-2">
          {experiments.isLoading ? (
            <div className="text-sm text-text-dim">Loading…</div>
          ) : experiments.data && experiments.data.length > 0 ? (
            experiments.data.map((e) => (
              <ExperimentRow
                key={e.id}
                exp={e}
                active={e.id === selected}
                onClick={() => setSelected(e.id)}
              />
            ))
          ) : (
            <div className="rounded-md border border-dashed border-border-2 p-4 text-sm text-text-dim">
              No experiments yet. Create one to begin.
            </div>
          )}
        </div>

        <div>
          {selectedExp ? (
            <ExperimentDetail exp={selectedExp} />
          ) : (
            <div className="flex h-64 items-center justify-center rounded-lg border border-dashed border-border-2 text-sm text-text-dim">
              Select an experiment to open the workbench.
            </div>
          )}
        </div>
      </div>

      {creating && (
        <CreateExperimentDialog
          onClose={() => setCreating(false)}
          onCreated={(id) => {
            setCreating(false)
            setSelected(id)
            experiments.refetch()
          }}
        />
      )}
    </div>
  )
}

function ExperimentRow({
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
      {/* Counter + lifecycle, always rendered together. */}
      <div className="flex items-center justify-between gap-2 text-xs">
        <Badge variant={STATE_VARIANT[exp.state] ?? 'default'}>{exp.state}</Badge>
        <span className="text-text-dim">
          <span className="font-semibold tabular-nums text-text-muted">
            {exp.trial_counter.toLocaleString()}
          </span>{' '}
          trials
        </span>
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
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle className="text-sm">Suite calibration</CardTitle>
        {c.calibration.optimistic ? (
          <Badge variant="warning" className="gap-1">
            <AlertTriangle className="h-3 w-3" />
            systematically optimistic
          </Badge>
        ) : (
          <Badge variant="active">calibrated</Badge>
        )}
      </CardHeader>
      <CardContent className="grid grid-cols-3 gap-3 text-sm">
        <Metric label="mean realized percentile" value={c.calibration.mean_percentile.toFixed(2)} />
        <Metric label="worst-5% coverage" value={`${(c.calibration.worst5_coverage * 100).toFixed(0)}%`} />
        <Metric label="experiments" value={String(c.experiments_contributing)} />
      </CardContent>
    </Card>
  )
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md bg-surface-2 p-3">
      <div className="text-[10px] uppercase tracking-wide text-text-dim">{label}</div>
      <div className="mt-1 text-lg font-semibold text-text">{value}</div>
    </div>
  )
}

function CreateExperimentDialog({
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
      toast({ title: 'Experiment created' })
      onCreated(r.data.id)
    },
    onError: (e: unknown) => {
      const msg =
        (e as { response?: { data?: { message?: string } } })?.response?.data?.message ??
        'Could not create experiment'
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
          <CardTitle className="text-base">New experiment</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <Field label="Experiment id" value={form.experiment_id} onChange={set('experiment_id')} />
          <Field label="Strategy family" value={form.strategy_family} onChange={set('strategy_family')} />
          <Field label="Strategy type (seeds null recommendation)" value={form.strategy_type} onChange={set('strategy_type')} />
          <Field label="Universe ref" value={form.universe_ref} onChange={set('universe_ref')} />
          <div className="grid grid-cols-2 gap-3">
            <Field label="Research start" type="date" value={form.research_start} onChange={set('research_start')} />
            <Field label="Research end" type="date" value={form.research_end} onChange={set('research_end')} />
            <Field label="Holdout start" type="date" value={form.holdout_start} onChange={set('holdout_start')} />
            <Field label="Holdout end" type="date" value={form.holdout_end} onChange={set('holdout_end')} />
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
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
