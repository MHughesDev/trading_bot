import { useState, useRef } from 'react'
import { useParams, useNavigate, useSearchParams } from 'react-router-dom'
import { motion, useReducedMotion } from 'framer-motion'
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
  useArchiveModel,
  usePatchModel,
} from '@/hooks/useMlOps'
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

const KIND_LABELS: Record<ModelKind, string> = {
  forecaster: 'Forecaster',
  signal_ranker: 'Signal Ranker',
  trade_decision: 'Trade Decision',
  risk_sizing: 'Risk Sizing',
  embedding: 'Embedding',
  external_llm_adapter: 'LLM Adapter',
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
  const offset = circumference - (value / 100) * circumference

  return (
    <div className="flex flex-col items-center gap-2">
      <div className="relative">
        <svg width={72} height={72} className="-rotate-90">
          <circle cx={36} cy={36} r={r} fill="none" stroke="var(--tb-border)" strokeWidth={5} />
          <circle
            cx={36}
            cy={36}
            r={r}
            fill="none"
            stroke="var(--tb-accent)"
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
      <span className="text-xs text-text-muted text-center">{label}</span>
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
        <button onClick={save} className="text-pnl-up hover:opacity-80">
          <Check className="h-4 w-4" />
        </button>
        <button onClick={cancel} className="text-text-muted hover:opacity-80">
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
      <Pencil className="h-4 w-4 text-text-dim opacity-0 group-hover:opacity-100 transition-opacity shrink-0" />
    </button>
  )
}

function OverviewTab({ modelId }: { modelId: string }) {
  const { data: model } = useModel(modelId)
  const { data: aliases } = useModelAliases(modelId)
  const { data: usedBy } = useModelUsedBy(modelId)
  const { data: evals } = useModelEvals(modelId)

  if (!model) return null

  const latestEval = evals?.[evals.length - 1]
  const scorecard = latestEval?.scorecard
  const aliasEntries = aliases ? Object.entries(aliases) : []

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
      {/* Identity */}
      <div className="rounded-xl border border-border bg-surface p-5">
        <h3 className="text-sm font-medium text-text mb-4">Identity</h3>
        <dl className="space-y-2.5">
          {[
            { label: 'Kind', value: KIND_LABELS[model.model_kind] },
            { label: 'Asset class', value: model.asset_class, mono: true },
            {
              label: 'Framework',
              value: model.definition.framework ?? '—',
              mono: true,
            },
            { label: 'Runtime', value: model.definition.runtime ?? '—', mono: true },
            {
              label: 'Auto-retrain',
              value: model.definition.auto_retrain ? 'Yes' : 'No',
            },
            { label: 'Slug', value: model.slug, mono: true },
          ].map(({ label, value, mono }) => (
            <div key={label} className="flex items-start justify-between gap-4">
              <dt className="text-xs text-text-muted shrink-0">{label}</dt>
              <dd
                className={cn(
                  'text-sm text-text text-right truncate',
                  mono && 'font-mono text-xs',
                )}
              >
                {value}
              </dd>
            </div>
          ))}
        </dl>

        {/* Aliases */}
        {aliasEntries.length > 0 && (
          <div className="mt-4 pt-4 border-t border-border">
            <p className="text-xs text-text-muted mb-2">Aliases</p>
            <div className="flex flex-wrap gap-2">
              {aliasEntries.map(([alias, version]) => (
                <span
                  key={alias}
                  className="inline-flex items-center gap-1 rounded-full border border-border bg-surface-2 px-2 py-0.5 text-xs text-text-muted"
                >
                  {alias}
                  <span className="font-mono text-accent">→ v{version}</span>
                </span>
              ))}
            </div>
          </div>
        )}
      </div>

      {/* Health panel */}
      <div className="rounded-xl border border-border bg-surface p-5">
        <h3 className="text-sm font-medium text-text mb-4">Health</h3>
        {scorecard ? (
          <div className="grid grid-cols-3 gap-4">
            <ScorecardRing label="Quality" value={scorecard.quality * 100} />
            <ScorecardRing label="Speed" value={scorecard.speed * 100} />
            <ScorecardRing label="Cost" value={scorecard.cost * 100} />
            <ScorecardRing label="Safety" value={scorecard.safety * 100} />
            <ScorecardRing label="Reliability" value={scorecard.reliability * 100} />
            <ScorecardRing label="Overall" value={scorecard.overall * 100} />
          </div>
        ) : (
          <div className="flex items-center justify-center py-10">
            <p className="text-sm text-text-dim">
              No evaluation data yet. Run an evaluation to see health metrics.
            </p>
          </div>
        )}
      </div>

      {/* Used by */}
      {usedBy && usedBy.length > 0 && (
        <div className="rounded-xl border border-border bg-surface p-5">
          <h3 className="text-sm font-medium text-text mb-3">Used by strategies</h3>
          <div className="flex flex-wrap gap-2">
            {usedBy.map((s) => (
              <span
                key={s.id}
                className="inline-flex items-center rounded-full border border-border bg-surface-2 px-3 py-1 text-xs text-text-muted"
              >
                {s.name}
                <span
                  className={cn(
                    'ml-1.5 h-1.5 w-1.5 rounded-full',
                    s.status === 'active' ? 'bg-pnl-up' : 'bg-text-dim',
                  )}
                />
              </span>
            ))}
          </div>
        </div>
      )}

      {/* Description */}
      {model.description && (
        <div className="rounded-xl border border-border bg-surface p-5">
          <h3 className="text-sm font-medium text-text mb-3">Description</h3>
          <p className="text-sm text-text-muted leading-relaxed">{model.description}</p>
        </div>
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
      <div className="flex items-center justify-center py-24 text-text-dim">
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
        className="flex items-center gap-1.5 text-sm text-text-muted hover:text-text transition-colors mb-5"
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
          <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-surface-2 text-accent shrink-0">
            <Icon className="h-5 w-5" />
          </div>

          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-3 flex-wrap">
              <InlineEdit
                value={model.display_name}
                onSave={(name) => patchMut.mutate({ display_name: name })}
              />
              <ModelStatusPill status={model.status} />
              <span className="inline-flex items-center rounded-md border border-border bg-surface-2 px-2 py-0.5 text-xs text-text-muted">
                {KIND_LABELS[model.model_kind]}
              </span>
            </div>
            <p className="text-sm text-text-muted mt-1 font-mono">{model.slug}</p>
          </div>

          {/* Actions */}
          <div className="flex items-center gap-2 shrink-0">
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
                : 'text-text-muted hover:text-text',
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
