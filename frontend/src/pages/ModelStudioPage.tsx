import { useState, useMemo } from 'react'
import { useNavigate } from 'react-router-dom'
import { motion, AnimatePresence, useReducedMotion } from 'framer-motion'
import {
  Brain,
  BarChart2,
  Zap,
  Shield,
  Code2,
  Bot,
  Plus,
  LayoutGrid,
  List,
  Search,
  Loader2,
  ExternalLink,
  ChevronRight,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { useModels } from '@/hooks/useModels'
import { ModelStatusPill } from '@/components/models/ModelStatusPill'
import type { AiModel, ModelKind, ModelStatus } from '@/api/models'
import { format } from 'date-fns'

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

const ALL_KINDS: ModelKind[] = [
  'forecaster',
  'signal_ranker',
  'trade_decision',
  'risk_sizing',
  'embedding',
  'external_llm_adapter',
]

const ALL_STATUSES: ModelStatus[] = [
  'draft',
  'training',
  'evaluating',
  'candidate',
  'active',
  'archived',
  'failed',
]

const SPRING = { type: 'spring' as const, stiffness: 380, damping: 30 }

function ModelCardSkeleton() {
  return (
    <div className="rounded-xl border border-border bg-surface p-5 animate-pulse">
      <div className="flex items-start gap-3 mb-4">
        <div className="h-9 w-9 rounded-lg bg-surface-2" />
        <div className="flex-1 space-y-2">
          <div className="h-4 w-3/4 rounded bg-surface-2" />
          <div className="h-3 w-1/2 rounded bg-surface-2" />
        </div>
        <div className="h-5 w-16 rounded-full bg-surface-2" />
      </div>
      <div className="space-y-2">
        <div className="h-3 w-full rounded bg-surface-2" />
        <div className="h-3 w-2/3 rounded bg-surface-2" />
      </div>
      <div className="mt-4 flex gap-2">
        <div className="h-8 w-16 rounded bg-surface-2" />
        <div className="h-8 w-24 rounded bg-surface-2" />
      </div>
    </div>
  )
}

function ModelCard({ model }: { model: AiModel }) {
  const navigate = useNavigate()
  const shouldReduce = useReducedMotion()
  const Icon = KIND_ICONS[model.model_kind]

  return (
    <motion.div
      layoutId={model.model_id}
      layout
      initial={{ opacity: 0, y: 12 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -8 }}
      transition={shouldReduce ? { duration: 0.001 } : SPRING}
      className="group rounded-xl border border-border bg-surface p-5 hover:border-border-2 hover:shadow-sm transition-shadow cursor-pointer"
      onClick={() => navigate(`/models/${model.model_id}`)}
    >
      {/* Header */}
      <div className="flex items-start gap-3 mb-3">
        <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-surface-2 text-accent shrink-0">
          <Icon className="h-4.5 w-4.5" />
        </div>
        <div className="flex-1 min-w-0">
          <h3 className="font-semibold text-text truncate text-sm leading-5">
            {model.display_name}
          </h3>
          <p className="text-xs text-text-muted font-mono">{model.slug}</p>
        </div>
        <ModelStatusPill status={model.status} />
      </div>

      {/* Description */}
      {model.description && (
        <p className="text-xs text-text-muted line-clamp-2 mb-3">{model.description}</p>
      )}

      {/* Badges */}
      <div className="flex flex-wrap gap-1.5 mb-4">
        <span className="inline-flex items-center rounded-md bg-surface-2 px-2 py-0.5 text-xs text-text-muted border border-border">
          {KIND_LABELS[model.model_kind]}
        </span>
        {model.asset_class && (
          <span className="inline-flex items-center rounded-md bg-surface-2 px-2 py-0.5 text-xs text-text-muted border border-border">
            {model.asset_class}
          </span>
        )}
        {model.definition.framework && (
          <span className="inline-flex items-center rounded-md bg-surface-2 px-2 py-0.5 text-xs font-mono text-text-dim border border-border">
            {model.definition.framework}
          </span>
        )}
      </div>

      <div className="text-xs text-text-dim mb-4">
        Updated {format(new Date(model.updated_at), 'MMM d, yyyy')}
      </div>

      {/* Actions */}
      <div className="flex gap-2">
        <Button
          size="sm"
          variant="outline"
          className="flex-1 text-xs h-7"
          onClick={(e) => {
            e.stopPropagation()
            navigate(`/models/${model.model_id}`)
          }}
        >
          Open
          <ChevronRight className="h-3 w-3 ml-1" />
        </Button>
        <Button
          size="sm"
          variant="outline"
          className="text-xs h-7"
          onClick={(e) => {
            e.stopPropagation()
            navigate(`/models/${model.model_id}?tab=test`)
          }}
        >
          <ExternalLink className="h-3 w-3 mr-1" />
          Test
        </Button>
      </div>
    </motion.div>
  )
}

function ModelTableRow({ model }: { model: AiModel }) {
  const navigate = useNavigate()
  const Icon = KIND_ICONS[model.model_kind]

  return (
    <tr
      className="border-b border-border hover:bg-surface-2 cursor-pointer transition-colors"
      onClick={() => navigate(`/models/${model.model_id}`)}
    >
      <td className="px-4 py-3">
        <div className="flex items-center gap-2">
          <Icon className="h-4 w-4 text-accent shrink-0" />
          <div>
            <div className="text-sm font-medium text-text">{model.display_name}</div>
            <div className="text-xs text-text-dim font-mono">{model.slug}</div>
          </div>
        </div>
      </td>
      <td className="px-4 py-3 text-sm text-text-muted">{KIND_LABELS[model.model_kind]}</td>
      <td className="px-4 py-3">
        <ModelStatusPill status={model.status} />
      </td>
      <td className="px-4 py-3 text-sm text-text-muted">{model.asset_class}</td>
      <td className="px-4 py-3 text-xs text-text-dim font-mono">
        {format(new Date(model.updated_at), 'MMM d, yyyy')}
      </td>
    </tr>
  )
}

export function ModelStudioPage() {
  const navigate = useNavigate()
  const [viewMode, setViewMode] = useState<'grid' | 'table'>('grid')
  const [kindFilter, setKindFilter] = useState<ModelKind | ''>('')
  const [statusFilter, setStatusFilter] = useState<ModelStatus | ''>('')
  const [search, setSearch] = useState('')

  const { data, isLoading } = useModels({
    kind: kindFilter || undefined,
    status: statusFilter || undefined,
    q: search || undefined,
  })

  const models = useMemo(() => data?.models ?? [], [data])

  return (
    <div className="mx-auto w-full max-w-7xl px-6 py-6">
      {/* Header */}
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="flex items-center gap-2 text-2xl font-semibold text-text">
            <Brain className="h-6 w-6 text-accent" />
            AI Model Studio
          </h1>
          <p className="mt-1 text-sm text-text-muted">
            Manage, train, and deploy machine learning models for your trading strategies.
          </p>
        </div>
        <Button onClick={() => navigate('/models/create')}>
          <Plus className="h-4 w-4" />
          Create Model
        </Button>
      </div>

      {/* Filter bar */}
      <div className="mb-5 flex flex-wrap items-center gap-3">
        {/* Search */}
        <div className="relative flex-1 min-w-[200px] max-w-xs">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
          <input
            type="text"
            placeholder="Search models…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="w-full pl-8 pr-3 h-8 rounded-lg border border-border bg-surface text-sm text-text placeholder:text-text-dim focus:outline-none focus:border-accent transition-colors"
          />
        </div>

        {/* Kind filter */}
        <select
          value={kindFilter}
          onChange={(e) => setKindFilter(e.target.value as ModelKind | '')}
          className="h-8 rounded-lg border border-border bg-surface px-3 text-sm text-text focus:outline-none focus:border-accent"
        >
          <option value="">All kinds</option>
          {ALL_KINDS.map((k) => (
            <option key={k} value={k}>
              {KIND_LABELS[k]}
            </option>
          ))}
        </select>

        {/* Status filter */}
        <select
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.target.value as ModelStatus | '')}
          className="h-8 rounded-lg border border-border bg-surface px-3 text-sm text-text focus:outline-none focus:border-accent"
        >
          <option value="">All statuses</option>
          {ALL_STATUSES.map((s) => (
            <option key={s} value={s}>
              {s.charAt(0).toUpperCase() + s.slice(1)}
            </option>
          ))}
        </select>

        {/* View toggle */}
        <div className="ml-auto flex rounded-lg border border-border overflow-hidden">
          <button
            onClick={() => setViewMode('grid')}
            className={cn(
              'flex items-center justify-center w-8 h-8 transition-colors',
              viewMode === 'grid'
                ? 'bg-surface-2 text-text'
                : 'bg-surface text-text-muted hover:text-text',
            )}
          >
            <LayoutGrid className="h-3.5 w-3.5" />
          </button>
          <button
            onClick={() => setViewMode('table')}
            className={cn(
              'flex items-center justify-center w-8 h-8 transition-colors border-l border-border',
              viewMode === 'table'
                ? 'bg-surface-2 text-text'
                : 'bg-surface text-text-muted hover:text-text',
            )}
          >
            <List className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      {/* Body */}
      {isLoading ? (
        viewMode === 'grid' ? (
          <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
            {Array.from({ length: 6 }).map((_, i) => (
              <ModelCardSkeleton key={i} />
            ))}
          </div>
        ) : (
          <div className="flex items-center justify-center py-20 text-text-dim">
            <Loader2 className="h-6 w-6 animate-spin" />
          </div>
        )
      ) : models.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-4 rounded-xl border border-dashed border-border py-24 text-center">
          <Brain className="h-12 w-12 text-text-dim" />
          <div>
            <p className="text-base font-medium text-text">No models yet</p>
            <p className="mt-1 text-sm text-text-muted">
              Create your first model to get started with AI-powered trading.
            </p>
          </div>
          <Button onClick={() => navigate('/models/create')}>
            <Plus className="h-4 w-4" />
            Create Model
          </Button>
        </div>
      ) : viewMode === 'grid' ? (
        <AnimatePresence mode="popLayout">
          <motion.div
            layout
            className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4"
          >
            {models.map((model) => (
              <ModelCard key={model.model_id} model={model} />
            ))}
          </motion.div>
        </AnimatePresence>
      ) : (
        <div className="rounded-xl border border-border bg-surface overflow-hidden">
          <table className="w-full">
            <thead>
              <tr className="border-b border-border bg-surface-2">
                <th className="px-4 py-3 text-left text-xs font-medium text-text-muted">Model</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-text-muted">Kind</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-text-muted">Status</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-text-muted">Asset Class</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-text-muted">Updated</th>
              </tr>
            </thead>
            <tbody>
              {models.map((model) => (
                <ModelTableRow key={model.model_id} model={model} />
              ))}
            </tbody>
          </table>
        </div>
      )}

      {!isLoading && models.length > 0 && (
        <p className="mt-4 text-xs text-text-dim text-right">
          {data?.total ?? models.length} model{(data?.total ?? models.length) !== 1 ? 's' : ''}
        </p>
      )}
    </div>
  )
}
