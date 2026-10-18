import { useState, useRef } from 'react'
import { useParams, useNavigate, useSearchParams } from 'react-router-dom'
import { motion, useReducedMotion } from 'framer-motion'
import { format } from 'date-fns'
import {
  Brain,
  BarChart2,
  Zap,
  Shield,
  Code2,
  Bot,
  Archive,
  ChevronLeft,
  Loader2,
  Pencil,
  Check,
  X,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import {
  useModel,
  useModelAliases,
  useModelUsedBy,
  useModelEvals,
  useEvalDetails,
  useModelVersions,
  useModelRuns,
  useModelDeployments,
  useModelTraces,
  useModelEvents,
  useArchiveModel,
  usePatchModel,
} from '@/hooks/useMlOps'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { Badge } from '@/components/primitives/Badge'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { KIND_LABELS, buildActivity, pinProduction, toEvalRow, traceStats } from '@/components/mlops/studio.helpers'
import { ModelStatusPill } from '@/components/mlops/ModelStatusPill'
import { ModelVersionsTab } from '@/components/mlops/ModelVersionsTab'
import { ModelTrainTab } from '@/components/mlops/ModelTrainTab'
import { ModelTestTab } from '@/components/mlops/ModelTestTab'
import { ModelEvalsTab } from '@/components/mlops/ModelEvalsTab'
import { ModelDeploymentsTab } from '@/components/mlops/ModelDeploymentsTab'
import { ForecastChartsTab } from '@/components/mlops/ForecastChartsTab'
import type { ModelKind } from '@/api/mlops'

const KIND_ICONS: Record<ModelKind, React.ElementType> = {
  forecaster: Brain,
  signal_ranker: BarChart2,
  trade_decision: Zap,
  risk_sizing: Shield,
  embedding: Code2,
  external_llm_adapter: Bot,
}

type Tab = 'overview' | 'versions' | 'train' | 'test' | 'evaluations' | 'deployments' | 'forecast'

const TABS: { id: Tab; label: string }[] = [
  { id: 'overview', label: 'Overview' },
  { id: 'versions', label: 'Versions' },
  { id: 'train', label: 'Train' },
  { id: 'test', label: 'Test Lab' },
  { id: 'evaluations', label: 'Evaluations' },
  { id: 'deployments', label: 'Deployments' },
  { id: 'forecast', label: 'Forecast Quality' },
]

const SPRING = { type: 'spring' as const, stiffness: 380, damping: 30 }

function ScorecardRing({
  label,
  value,
}: {
  label: string
  value: number
}) {
  const r = 28
  const circumference = 2 * Math.PI * r
  const clamped = Math.max(0, Math.min(100, value))
  const offset = circumference - (clamped / 100) * circumference

  return (
    <div className="flex flex-col items-center gap-2">
      <div className="relative">
        <svg width={72} height={72} className="-rotate-90">
          <circle cx={36} cy={36} r={r} fill="none" stroke="var(--line-hairline)" strokeWidth={5} />
          <circle
            cx={36}
            cy={36}
            r={r}
            fill="none"
            stroke="var(--bg-accent)"
            strokeWidth={5}
            strokeDasharray={circumference}
            strokeDashoffset={offset}
            strokeLinecap="round"
            style={{ transition: 'stroke-dashoffset 0.6s ease' }}
          />
        </svg>
        <div className="absolute inset-0 flex items-center justify-center">
          <span className="text-xs font-mono font-bold text-text">{Math.round(value)}</span>
        </div>
      </div>
      <span className="text-xs text-fg-secondary text-center">{label}</span>
    </div>
  )
}

function InlineEdit({
  value,
  onSave,
}: {
  value: string
  onSave: (newValue: string) => void
}) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(value)
  const inputRef = useRef<HTMLInputElement>(null)

  function startEdit() {
    setDraft(value)
    setEditing(true)
    setTimeout(() => inputRef.current?.focus(), 0)
  }

  function save() {
    if (draft.trim() && draft.trim() !== value) {
      onSave(draft.trim())
    }
    setEditing(false)
  }

  function cancel() {
    setDraft(value)
    setEditing(false)
  }

  if (editing) {
    return (
      <div className="flex items-center gap-2">
        <input
          ref={inputRef}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') save()
            if (e.key === 'Escape') cancel()
          }}
          className="text-2xl font-semibold text-text bg-transparent border-b border-accent focus:outline-none"
        />
        <button onClick={save} className="text-pos hover:opacity-80">
          <Check className="h-4 w-4" />
        </button>
        <button onClick={cancel} className="text-fg-secondary hover:opacity-80">
          <X className="h-4 w-4" />
        </button>
      </div>
    )
  }

  return (
    <button
      onClick={startEdit}
      className="group flex items-center gap-2 text-left"
    >
      <h1 className="text-2xl font-semibold text-text">{value}</h1>
      <Pencil className="h-4 w-4 text-fg-tertiary opacity-0 group-hover:opacity-100 transition-opacity shrink-0" />
    </button>
  )
}

function OverviewTab({ modelId }: { modelId: string }) {
  const { data: model } = useModel(modelId)
  const { data: aliases } = useModelAliases(modelId)
  const usedBy = useModelUsedBy(modelId)
  const evals = useModelEvals(modelId)
  const details = useEvalDetails(modelId, evals.data)
  const versions = useModelVersions(modelId)
  const runs = useModelRuns(modelId)
  const deployments = useModelDeployments(modelId)
  const traces = useModelTraces(modelId)
  const events = useModelEvents(modelId)

  if (!model) return null

  const prod = typeof aliases?.production === 'number' ? aliases.production : null
  // The scorecard shown is the PRODUCTION version's latest evaluation when there
  // is one, else the newest evaluation, and it says which version it describes.
  const evalRows = pinProduction(
    details.flatMap((d) => (d.data ? [toEvalRow(d.data)] : [])),
    prod,
  ).filter((r) => r.scorecard !== null)
  const shown = evalRows[0] ?? null
  const scorecard = shown?.scorecard ?? null
  const aliasEntries = aliases ? Object.entries(aliases) : []
  const stats = traceStats(traces.data)
  const activity = buildActivity({
    runs: runs.data ?? [],
    versions: versions.data ?? [],
    deployments: deployments.data ?? [],
    events: events.data ?? null,
  })

  return (
    <div style={{ display: 'grid', gap: 'var(--s-4)', gridTemplateColumns: 'repeat(auto-fit, minmax(min(100%, 380px), 1fr))' }}>
      <Panel>
        <PanelHeader title="Identity" />
        <PanelBody>
          <dl style={{ display: 'grid', gridTemplateColumns: 'max-content 1fr', gap: '6px var(--s-4)', margin: 0 }}>
            {[
              { label: 'Kind', value: KIND_LABELS[model.model_kind] },
              { label: 'Asset class', value: model.asset_class },
              { label: 'Framework', value: model.definition.framework ?? '—' },
              { label: 'Runtime', value: model.definition.runtime ?? '—' },
              { label: 'Auto-retrain', value: model.definition.auto_retrain ? 'Yes' : 'No' },
              { label: 'Slug', value: model.slug },
              { label: 'Production version', value: prod !== null ? `v${prod}` : 'none promoted' },
            ].map(({ label, value }) => (
              <div key={label} style={{ display: 'contents' }}>
                <dt className="mut">{label}</dt>
                <dd style={{ margin: 0 }}>{value}</dd>
              </div>
            ))}
          </dl>
          {aliasEntries.length > 0 && (
            <div style={{ display: 'flex', gap: 'var(--s-2)', flexWrap: 'wrap', marginTop: 'var(--s-3)' }}>
              {aliasEntries.map(([alias, version]) => (
                <Badge key={alias} tone={alias === 'production' ? 'pos' : 'outline'}>
                  {alias} → v{version}
                </Badge>
              ))}
            </div>
          )}
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader
          title="Health"
          badges={shown ? <Badge tone="neutral">evaluation of v{shown.version}</Badge> : undefined}
        />
        <PanelBody>
          <div style={{ display: 'flex', gap: 'var(--s-4)', flexWrap: 'wrap', marginBottom: 'var(--s-3)', fontSize: 'var(--t-12)' }} data-testid="trace-stats">
            {traces.isError ? (
              <span className="mut">Inference traces could not be read.</span>
            ) : stats === null || stats.n === 0 ? (
              <span className="mut">No inference traces yet, so latency and error rate are not measured.</span>
            ) : (
              <>
                <span>p95 latency <span className="num">{stats.p95LatencyMs === null ? '—' : `${Math.round(stats.p95LatencyMs)} ms`}</span></span>
                <span>error rate <span className="num">{stats.errorRate === null ? '—' : `${(stats.errorRate * 100).toFixed(1)}%`}</span></span>
                <span className="mut">over the last {stats.n} traces; cost per 1k is not recorded</span>
              </>
            )}
          </div>
          {evals.isError ? (
            <ErrorState message="Evaluations could not be loaded." onRetry={() => void evals.refetch()} />
          ) : scorecard ? (
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 'var(--s-3)' }}>
              <ScorecardRing label="Quality" value={scorecard.quality} />
              <ScorecardRing label="Speed" value={scorecard.speed} />
              <ScorecardRing label="Cost" value={scorecard.cost} />
              <ScorecardRing label="Safety" value={scorecard.safety} />
              <ScorecardRing label="Reliability" value={scorecard.reliability} />
              <ScorecardRing label="Overall" value={scorecard.overall} />
            </div>
          ) : (
            <EmptyState
              message="No scorecard yet"
              detail="Run an evaluation from the Versions tab. Latency, error rate and cost per 1k are not rolled up per model yet, so they are not shown."
            />
          )}
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader title="Used by strategies" />
        <PanelBody>
          {usedBy.isError ? (
            <ErrorState message="Used-by could not be read." onRetry={() => void usedBy.refetch()} />
          ) : usedBy.isLoading ? (
            <PanelLoading lines={2} />
          ) : (usedBy.data ?? []).length === 0 ? (
            <span className="mut">No strategy references this model.</span>
          ) : (
            <div style={{ display: 'flex', gap: 'var(--s-2)', flexWrap: 'wrap' }}>
              {(usedBy.data ?? []).map((s) => (
                <span key={s.id} className="chip">{s.name}</span>
              ))}
            </div>
          )}
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader title="Recent activity" />
        <PanelBody>
          {activity.length === 0 ? (
            <span className="mut">Nothing recorded yet.</span>
          ) : (
            <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }}>
              {activity.slice(0, 8).map((a) => (
                <li key={a.key} style={{ display: 'flex', gap: 'var(--s-3)' }}>
                  <span className="mut num" style={{ flex: 'none', fontSize: 'var(--t-11)' }}>
                    {format(new Date(a.at), 'MMM d, HH:mm')}
                  </span>
                  <span>{a.text}</span>
                </li>
              ))}
            </ul>
          )}
          <p className="mut" style={{ fontSize: 'var(--t-11)', marginTop: 'var(--s-3)' }}>
            {events.isError
              ? 'The registry’s event log could not be read, so renames, promotions and rollbacks are missing from this feed.'
              : 'Training runs, versions and deployments, joined with the registry’s event log (renames, promotions, rollbacks).'}
          </p>
        </PanelBody>
      </Panel>

      {model.description && (
        <Panel>
          <PanelHeader title="Description" />
          <PanelBody>{model.description}</PanelBody>
        </Panel>
      )}
    </div>
  )
}

export function ModelDetailPage() {
  const { id } = useParams<{ id: string }>()
  const navigate = useNavigate()
  const [searchParams, setSearchParams] = useSearchParams()
  const shouldReduce = useReducedMotion()

  const tabParam = searchParams.get('tab') as Tab | null
  const [activeTab, setActiveTab] = useState<Tab>(
    TABS.find((t) => t.id === tabParam)?.id ?? 'overview',
  )

  const { data: model, isLoading } = useModel(id!)
  const patchMut = usePatchModel(id!)
  const archiveMut = useArchiveModel()

  const Icon = model ? KIND_ICONS[model.model_kind] : Brain

  function handleTabChange(tab: Tab) {
    setActiveTab(tab)
    setSearchParams(tab === 'overview' ? {} : { tab })
  }

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-24 text-fg-tertiary">
        <Loader2 className="h-8 w-8 animate-spin" />
      </div>
    )
  }

  if (!model) {
    return (
      <div className="flex flex-col items-center justify-center py-24 text-center gap-4">
        <p className="text-base font-medium text-text">Model not found</p>
        <Button variant="outline" onClick={() => navigate('/mlops')}>
          <ChevronLeft className="h-4 w-4" />
          Back to models
        </Button>
      </div>
    )
  }

  return (
    <div className="mx-auto w-full max-w-6xl px-6 py-6">
      {/* Back nav */}
      <button
        onClick={() => navigate('/mlops')}
        className="flex items-center gap-1.5 text-sm text-fg-secondary hover:text-text transition-colors mb-5"
      >
        <ChevronLeft className="h-4 w-4" />
        ML Ops
      </button>

      {/* Glass header */}
      <motion.div
        layoutId={id}
        transition={shouldReduce ? { duration: 0.001 } : SPRING}
        className="rounded-xl border border-border bg-surface/80 backdrop-blur-sm p-5 mb-5"
      >
        <div className="flex items-start gap-4">
          <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-sunken text-accent shrink-0">
            <Icon className="h-5 w-5" />
          </div>

          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-3 flex-wrap">
              <InlineEdit
                value={model.display_name}
                onSave={(name) => patchMut.mutate({ display_name: name })}
              />
              <ModelStatusPill status={model.status} />
              <span className="inline-flex items-center rounded-md border border-border bg-sunken px-2 py-0.5 text-xs text-fg-secondary">
                {KIND_LABELS[model.model_kind]}
              </span>
            </div>
            <p className="text-sm text-fg-secondary mt-1 font-mono">{model.slug}</p>
          </div>

          {/* Actions */}
          <div className="flex items-center gap-2 shrink-0">
            <Button
              variant="outline"
              size="sm"
              onClick={() => navigate(`/mlops/graph?model=${model.model_id}`)}
              className="text-xs"
            >
              Lineage
            </Button>
            {model.status !== 'archived' && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => archiveMut.mutate(model.model_id)}
                disabled={archiveMut.isPending}
                className="text-xs"
              >
                <Archive className="h-3.5 w-3.5" />
                Archive
              </Button>
            )}
          </div>
        </div>
      </motion.div>

      {/* Tab bar */}
      <div className="flex border-b border-border mb-5 gap-0.5">
        {TABS.map((tab) => (
          <button
            key={tab.id}
            onClick={() => handleTabChange(tab.id)}
            className={cn(
              'relative px-4 py-2.5 text-sm font-medium transition-colors',
              activeTab === tab.id
                ? 'text-text'
                : 'text-fg-secondary hover:text-text',
            )}
          >
            {tab.label}
            {activeTab === tab.id && (
              <motion.div
                layoutId="tab-underline"
                className="absolute bottom-0 left-0 right-0 h-0.5 bg-accent rounded-full"
                transition={shouldReduce ? { duration: 0.001 } : SPRING}
              />
            )}
          </button>
        ))}
      </div>

      {/* Tab content */}
      <div>
        {activeTab === 'overview' && <OverviewTab modelId={id!} />}
        {activeTab === 'versions' && <ModelVersionsTab modelId={id!} />}
        {activeTab === 'train' && <ModelTrainTab modelId={id!} />}
        {activeTab === 'test' && <ModelTestTab modelId={id!} />}
        {activeTab === 'evaluations' && <ModelEvalsTab modelId={id!} />}
        {activeTab === 'deployments' && <ModelDeploymentsTab modelId={id!} />}
        {activeTab === 'forecast' && <ForecastChartsTab modelId={id!} />}
      </div>
    </div>
  )
}
