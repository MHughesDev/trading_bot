// Command Center -- /mlops
//
// Discovery for the Model Studio: every model as a card (default) or a sortable
// table, with its status, headline metric, accuracy trend and the strategies
// that use it. Every figure here is read from a registry route; a figure the
// platform has not measured shows a dash, never zero (Set H H-5.2).
import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { motion, AnimatePresence, useReducedMotion } from 'framer-motion'
import { Brain, LayoutGrid, List, Plus, ArrowDownUp } from 'lucide-react'
import { Button } from '@/components/primitives/Button'
import { Panel } from '@/components/primitives/Panel'
import { Table, type Column } from '@/components/primitives/Table'
import { SearchInput } from '@/components/primitives/Field'
import { EmptyState, ErrorState, Skeleton } from '@/components/primitives/States'
import { useModels } from '@/hooks/useMlOps'
import { useModelSummaries } from '@/hooks/useModelSummaries'
import { ModelStatusPill } from '@/components/mlops/ModelStatusPill'
import { PlatformHealthPanel } from '@/components/mlops/PlatformHealthPanel'
import { FalsificationPanel } from '@/components/mlops/FalsificationPanel'
import { Sparkline, UsedByChips } from '@/components/mlops/StudioBits'
import { KIND_ICONS } from '@/components/mlops/kindIcons'
import {
  ALL_KINDS,
  ALL_STATUSES,
  KIND_LABELS,
  assetClassOptions,
  formatMetric,
  metricLabel,
  readViewMode,
  sortModels,
  writeViewMode,
  type ModelSummary,
  type SortDir,
  type SortKey,
  type ViewMode,
} from '@/components/mlops/studio.helpers'
import type { AiModel, ModelKind, ModelStatus } from '@/api/mlops'
import { format } from 'date-fns'

const SPRING = { type: 'spring' as const, stiffness: 380, damping: 30 }

const SORT_LABELS: Record<SortKey, string> = {
  updated: 'Updated',
  name: 'Name',
  kind: 'Kind',
  status: 'Status',
  metric: 'Primary metric',
  version: 'Version',
}

function storage(): Storage | null {
  try {
    return typeof window === 'undefined' ? null : window.localStorage
  } catch {
    return null
  }
}

function MetricCell({ summary }: { summary: ModelSummary | undefined }) {
  const p = summary?.primary
  if (!p) return <span className="mut">—</span>
  return (
    <span className="num" title={p.key}>
      <span className="mut" style={{ fontSize: 'var(--t-11)' }}>
        {metricLabel(p.key)}{' '}
      </span>
      {formatMetric(p.key, p.value)}
    </span>
  )
}

function versionLabel(s: ModelSummary | undefined): string {
  if (!s) return '—'
  if (s.productionVersion !== null) return `v${s.productionVersion}`
  if (s.latestVersion !== null) return `v${s.latestVersion} (not promoted)`
  return 'no versions'
}

function ModelCardSkeleton() {
  return (
    <Panel aria-hidden>
      <div style={{ padding: 'var(--s-4)', display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
        <Skeleton width="70%" height={16} />
        <Skeleton width="40%" height={11} />
        <Skeleton width="100%" height={28} />
        <Skeleton width="60%" height={12} />
      </div>
    </Panel>
  )
}

function ModelCard({ model, summary }: { model: AiModel; summary: ModelSummary | undefined }) {
  const navigate = useNavigate()
  const reduce = useReducedMotion()
  const Icon = KIND_ICONS[model.model_kind]
  const open = (tab?: string) =>
    navigate(`/mlops/${model.model_id}${tab ? `?tab=${tab}` : ''}`)

  return (
    <motion.div
      layoutId={model.model_id}
      layout
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -6 }}
      transition={reduce ? { duration: 0.001 } : SPRING}
    >
      <Panel data-testid="model-card">
        <div
          role="link"
          tabIndex={0}
          aria-label={`Open ${model.display_name}`}
          onClick={() => open()}
          onKeyDown={(e) => {
            if (e.key === 'Enter') open()
          }}
          style={{ padding: 'var(--s-4)', cursor: 'pointer', display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}
        >
          <div style={{ display: 'flex', gap: 'var(--s-3)', alignItems: 'flex-start' }}>
            <Icon size={18} aria-hidden style={{ flex: 'none', marginTop: 2, color: 'var(--fg-accent)' }} />
            <div style={{ minWidth: 0, flex: 1 }}>
              <div className="h2 truncate-1">{model.display_name}</div>
              <div className="mut truncate-1" style={{ fontSize: 'var(--t-11)' }}>
                {KIND_LABELS[model.model_kind]} · {model.asset_class || 'any asset'} · {versionLabel(summary)}
              </div>
            </div>
            <ModelStatusPill status={model.status} />
          </div>

          <Sparkline points={summary?.series ?? []} label={summary?.primary ? metricLabel(summary.primary.key) : 'metric'} width={240} />

          <div style={{ display: 'flex', gap: 'var(--s-4)', alignItems: 'baseline' }}>
            <MetricCell summary={summary} />
            <span className="mut" style={{ fontSize: 'var(--t-11)' }} title="Latency and cost per 1k come from inference traces, which are not rolled up per model yet.">
              p95 · cost/1k not measured
            </span>
          </div>

          <UsedByChips usedBy={summary?.usedBy ?? null} />
        </div>
        <div style={{ display: 'flex', gap: 'var(--s-2)', padding: '0 var(--s-4) var(--s-4)' }}>
          <Button size="sm" onClick={() => open()}>Open</Button>
          <Button size="sm" onClick={() => open('test')}>Test</Button>
          <Button size="sm" onClick={() => open('train')}>Train</Button>
        </div>
      </Panel>
    </motion.div>
  )
}

export function MlOpsPage() {
  const navigate = useNavigate()
  const [view, setView] = useState<ViewMode>(() => readViewMode(storage()))
  const [kind, setKind] = useState<ModelKind | ''>('')
  const [status, setStatus] = useState<ModelStatus | ''>('')
  const [assetClass, setAssetClass] = useState('')
  const [search, setSearch] = useState('')
  const [sortKey, setSortKey] = useState<SortKey>('updated')
  const [sortDir, setSortDir] = useState<SortDir>('desc')

  const filters = {
    kind: kind || undefined,
    status: status || undefined,
    asset_class: assetClass || undefined,
    q: search.trim() || undefined,
  }
  const { data, isLoading, isError, refetch } = useModels(filters)
  const models = useMemo(() => data?.models ?? [], [data])
  const { summaries } = useModelSummaries(models)
  const sorted = useMemo(
    () => sortModels(models, summaries, sortKey, sortDir),
    [models, summaries, sortKey, sortDir],
  )
  const assetClasses = useMemo(() => assetClassOptions(models, assetClass), [models, assetClass])
  const anyFilter = Boolean(filters.kind || filters.status || filters.asset_class || filters.q)

  function changeView(next: ViewMode) {
    setView(next)
    writeViewMode(storage(), next)
  }
  function clearFilters() {
    setKind('')
    setStatus('')
    setAssetClass('')
    setSearch('')
  }

  const columns: Column<AiModel>[] = [
    {
      key: 'name',
      header: 'Model',
      cell: (m) => {
        const Icon = KIND_ICONS[m.model_kind]
        return (
          <span style={{ display: 'inline-flex', gap: 'var(--s-2)', alignItems: 'center' }}>
            <Icon size={14} aria-hidden style={{ color: 'var(--fg-accent)' }} />
            <span className="truncate-1">{m.display_name}</span>
          </span>
        )
      },
    },
    { key: 'kind', header: 'Kind', numeric: false, align: 'left', cell: (m) => KIND_LABELS[m.model_kind] },
    { key: 'status', header: 'Status', numeric: false, align: 'left', cell: (m) => <ModelStatusPill status={m.status} /> },
    { key: 'metric', header: 'Primary metric', cell: (m) => <MetricCell summary={summaries[m.model_id]} /> },
    { key: 'version', header: 'Version', cell: (m) => versionLabel(summaries[m.model_id]) },
    {
      key: 'usedby',
      header: 'Used by',
      cell: (m) => {
        const u = summaries[m.model_id]?.usedBy
        return u === null || u === undefined ? '…' : u.length
      },
    },
    {
      key: 'updated',
      header: 'Updated',
      cell: (m) => format(new Date(m.updated_at), 'MMM d, yyyy'),
    },
  ]

  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Models</h1>
        <span className="lbl">Train, evaluate and deploy the models your strategies call</span>
        <div className="spacer" />
        <Button size="sm" onClick={() => navigate('/mlops/leaderboard')}>Leaderboard / compare</Button>
        <Button size="sm" onClick={() => navigate('/mlops/graph')}>Lineage</Button>
        <Button size="sm" variant="primary" icon={<Plus size={13} aria-hidden />} onClick={() => navigate('/mlops/create')}>
          New model
        </Button>
      </div>

      <div className="page-body">
        {/* The platform's report on its own judgment, above the models rather than
            behind a tab (SPEC §16.2, ADR-P5-01). */}
        <PlatformHealthPanel className="mb-4" />
        <FalsificationPanel className="mb-4" />

        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 'var(--s-3)', alignItems: 'center', marginBottom: 'var(--s-4)' }}>
          <SearchInput value={search} onValueChange={setSearch} placeholder="Search models…" aria-label="Search models" />
          <select aria-label="Kind" value={kind} onChange={(e) => setKind(e.target.value as ModelKind | '')}>
            <option value="">All kinds</option>
            {ALL_KINDS.map((k) => (
              <option key={k} value={k}>{KIND_LABELS[k]}</option>
            ))}
          </select>
          <select aria-label="Status" value={status} onChange={(e) => setStatus(e.target.value as ModelStatus | '')}>
            <option value="">All statuses</option>
            {ALL_STATUSES.map((s) => (
              <option key={s} value={s}>{s.charAt(0).toUpperCase() + s.slice(1)}</option>
            ))}
          </select>
          <select aria-label="Asset class" value={assetClass} onChange={(e) => setAssetClass(e.target.value)}>
            <option value="">All asset classes</option>
            {assetClasses.map((a) => (
              <option key={a} value={a}>{a}</option>
            ))}
          </select>

          <div style={{ marginLeft: 'auto', display: 'flex', gap: 'var(--s-2)', alignItems: 'center' }}>
            <select aria-label="Sort by" value={sortKey} onChange={(e) => setSortKey(e.target.value as SortKey)}>
              {(Object.keys(SORT_LABELS) as SortKey[]).map((k) => (
                <option key={k} value={k}>{`Sort: ${SORT_LABELS[k]}`}</option>
              ))}
            </select>
            <Button
              size="sm"
              aria-label={sortDir === 'asc' ? 'Ascending' : 'Descending'}
              icon={<ArrowDownUp size={13} aria-hidden />}
              onClick={() => setSortDir((d) => (d === 'asc' ? 'desc' : 'asc'))}
            >
              {sortDir === 'asc' ? 'Asc' : 'Desc'}
            </Button>
            <Button size="sm" active={view === 'cards'} aria-label="Card view" icon={<LayoutGrid size={13} aria-hidden />} onClick={() => changeView('cards')} />
            <Button size="sm" active={view === 'table'} aria-label="Table view" icon={<List size={13} aria-hidden />} onClick={() => changeView('table')} />
          </div>
        </div>

        {isError ? (
          <ErrorState message="The model list could not be loaded." detail="This is a failed request, not an empty registry." onRetry={() => void refetch()} />
        ) : isLoading ? (
          view === 'cards' ? (
            <div style={{ display: 'grid', gap: 'var(--s-4)', gridTemplateColumns: 'repeat(auto-fill, minmax(min(100%, 340px), 1fr))' }}>
              {Array.from({ length: 6 }).map((_, i) => (
                <ModelCardSkeleton key={i} />
              ))}
            </div>
          ) : (
            <Panel>
              <Table columns={columns} rows={[]} rowKey={(m) => m.model_id} loading sortable={false} />
            </Panel>
          )
        ) : models.length === 0 ? (
          anyFilter ? (
            <EmptyState
              message="No models match these filters."
              action={<Button size="sm" onClick={clearFilters}>Clear filters</Button>}
            />
          ) : (
            <EmptyState
              icon={<Brain size={20} aria-hidden />}
              message="No models yet"
              detail="Create your first model to train, evaluate and deploy it."
              action={
                <Button variant="primary" icon={<Plus size={13} aria-hidden />} onClick={() => navigate('/mlops/create')}>
                  Create model
                </Button>
              }
            />
          )
        ) : view === 'cards' ? (
          <AnimatePresence mode="popLayout">
            <motion.div layout style={{ display: 'grid', gap: 'var(--s-4)', gridTemplateColumns: 'repeat(auto-fill, minmax(min(100%, 340px), 1fr))' }}>
              {sorted.map((m) => (
                <ModelCard key={m.model_id} model={m} summary={summaries[m.model_id]} />
              ))}
            </motion.div>
          </AnimatePresence>
        ) : (
          <Panel>
            <Table
              columns={columns}
              rows={sorted}
              rowKey={(m) => m.model_id}
              sortable={false}
              onRowClick={(m) => navigate(`/mlops/${m.model_id}`)}
              caption="Models"
            />
          </Panel>
        )}

        {!isLoading && !isError && models.length > 0 && (
          <p className="mut" style={{ marginTop: 'var(--s-3)', textAlign: 'right', fontSize: 'var(--t-11)' }}>
            {data?.total ?? models.length} model{(data?.total ?? models.length) !== 1 ? 's' : ''}
          </p>
        )}
      </div>
    </>
  )
}
