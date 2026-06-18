/**
 * MLOps — Automation (/mlops/automation).
 *
 * Surfaces the full declarative pipeline engine: create training/inference
 * pipelines from templates (optionally fanned-out across asset × timeframe and
 * scheduled on a bar cadence), visualise their DAG, run them, and track run +
 * node-run history.
 *
 * Talks to the real /api/pipelines/** endpoints (PipelineRecord shape:
 * { id, name, kind, definition: { dag: [...] }, created_at }).
 */

import { useState, useCallback } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { api as client } from '@/lib/api'
import {
  ReactFlow, Background, Controls, MiniMap, addEdge,
  useNodesState, useEdgesState,
  type Node, type Edge, type Connection, MarkerType,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import {
  Workflow, Play, Loader2, Plus, ChevronRight, Trash2,
  CheckCircle, AlertCircle, Clock, X, GitFork, CalendarClock,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { MLOpsSubNav } from '@/components/mlops/MLOpsSubNav'
import { cn } from '@/lib/utils'

// ── Types (match crates/domain/src/pipeline_def.rs + pipeline_manager.rs) ────

interface PipelineNodeDef {
  id: string
  op: string
  needs: string[]
  params?: Record<string, unknown>
}

interface PipelineMatrix {
  asset: string[]
  timeframe: string[]
  window: string[]
}

interface BarSchedule {
  reference_instrument: string
  timeframe: string
  every_n_bars: number
}

interface PipelineDefinition {
  schema_version?: string
  kind: string
  name: string
  dag: PipelineNodeDef[]
  matrix?: PipelineMatrix | null
  template?: boolean
  schedule?: BarSchedule | null
}

interface PipelineRecord {
  id: string
  name: string
  kind: string
  created_by: string
  definition: PipelineDefinition
  created_at: string
}

interface RunRecord {
  id: string
  pipeline_id: string
  parent_run_id?: string | null
  cell_label: string
  status: string
  cached: boolean
  started_at: string
  finished_at?: string | null
  error?: string | null
}

interface NodeRunRecord {
  id: string
  run_id: string
  node_id: string
  op: string
  status: string
  started_at?: string | null
  finished_at?: string | null
  error?: string | null
}

// ── Templates ────────────────────────────────────────────────────────────────

type TemplateKey = 'train' | 'inference'

const TEMPLATES: Record<TemplateKey, { label: string; description: string; kind: string; dag: PipelineNodeDef[] }> = {
  train: {
    label: 'Train → Evaluate → Register',
    description: 'Full training pipeline: materialize data, build features + target, train, calibrate, evaluate, register.',
    kind: 'training',
    dag: [
      { id: 'materialize', op: 'materialize', needs: [] },
      { id: 'features', op: 'features', needs: ['materialize'] },
      { id: 'target', op: 'target', needs: ['features'] },
      { id: 'train', op: 'train', needs: ['target'] },
      { id: 'calibrate', op: 'calibrate', needs: ['train'] },
      { id: 'evaluate', op: 'evaluate', needs: ['calibrate'] },
      { id: 'register', op: 'register', needs: ['evaluate'] },
    ],
  },
  inference: {
    label: 'Predict → Publish',
    description: 'Inference pipeline: load the deployed bundle, predict, calibrate, and publish the forecast.',
    kind: 'inference',
    dag: [
      { id: 'load_bundle', op: 'load_bundle', needs: [] },
      { id: 'predict', op: 'predict', needs: ['load_bundle'] },
      { id: 'calibrate', op: 'calibrate', needs: ['predict'] },
      { id: 'publish', op: 'publish', needs: ['calibrate'] },
    ],
  },
}

// ── API hooks ────────────────────────────────────────────────────────────────

function usePipelines() {
  return useQuery({
    queryKey: ['pipelines'],
    queryFn: () => client.get<PipelineRecord[]>('/api/pipelines').then((r) => r.data),
  })
}

function usePipelineRuns(pipelineId: string | null) {
  return useQuery({
    queryKey: ['pipeline-runs', pipelineId],
    queryFn: () => client.get<RunRecord[]>(`/api/pipelines/${pipelineId}/runs`).then((r) => r.data),
    enabled: !!pipelineId,
    refetchInterval: 4000,
  })
}

function useNodeRuns(runId: string | null) {
  return useQuery({
    queryKey: ['node-runs', runId],
    queryFn: () => client.get<NodeRunRecord[]>(`/api/pipelines/runs/${runId}/nodes`).then((r) => r.data),
    enabled: !!runId,
    refetchInterval: 2000,
  })
}

function useRunPipeline() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (pipelineId: string) =>
      client.post<{ run_id: string; cell_run_ids: string[]; cell_count: number }>(
        `/api/pipelines/${pipelineId}/run`,
        { force: false },
      ).then((r) => r.data),
    onSuccess: (_, pid) => qc.invalidateQueries({ queryKey: ['pipeline-runs', pid] }),
  })
}

function useCreatePipeline() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (definition: PipelineDefinition) =>
      client.post<PipelineRecord>('/api/pipelines', { definition }).then((r) => r.data),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['pipelines'] }),
  })
}

function useDeletePipeline() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (id: string) => client.delete(`/api/pipelines/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['pipelines'] }),
  })
}

// ── Status chip ──────────────────────────────────────────────────────────────

function StatusChip({ status }: { status: string }) {
  const map: Record<string, { icon: React.ReactNode; cls: string }> = {
    queued: { icon: <Clock className="h-3 w-3" />, cls: 'bg-zinc-500/15 text-zinc-400 border-zinc-500/30' },
    pending: { icon: <Clock className="h-3 w-3" />, cls: 'bg-zinc-500/15 text-zinc-400 border-zinc-500/30' },
    running: { icon: <Loader2 className="h-3 w-3 animate-spin" />, cls: 'bg-blue-500/15 text-blue-400 border-blue-500/30' },
    succeeded: { icon: <CheckCircle className="h-3 w-3" />, cls: 'bg-green-500/15 text-green-400 border-green-500/30' },
    cached: { icon: <CheckCircle className="h-3 w-3" />, cls: 'bg-emerald-500/15 text-emerald-400 border-emerald-500/30' },
    failed: { icon: <AlertCircle className="h-3 w-3" />, cls: 'bg-red-500/15 text-red-400 border-red-500/30' },
    cancelled: { icon: <X className="h-3 w-3" />, cls: 'bg-zinc-500/15 text-zinc-400 border-zinc-500/30' },
    skipped: { icon: <ChevronRight className="h-3 w-3" />, cls: 'bg-zinc-500/15 text-zinc-400 border-zinc-500/30' },
  }
  const { icon, cls } = map[status] ?? map.queued
  return (
    <span className={cn('inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs font-medium', cls)}>
      {icon}
      {status}
    </span>
  )
}

// ── DAG Flow view ────────────────────────────────────────────────────────────

function toFlow(defNodes: PipelineNodeDef[], nodeRuns?: NodeRunRecord[]): { nodes: Node[]; edges: Edge[] } {
  const statusMap: Record<string, string> = {}
  nodeRuns?.forEach((nr) => { statusMap[nr.node_id] = nr.status })

  const colors: Record<string, string> = {
    succeeded: '#22c55e', cached: '#10b981', running: '#3b82f6',
    failed: '#ef4444', pending: '#6b7280', skipped: '#9ca3af',
  }

  const nodes: Node[] = defNodes.map((n, i) => ({
    id: n.id,
    type: 'default',
    position: { x: (i % 4) * 180 + 40, y: Math.floor(i / 4) * 120 + 40 },
    data: { label: n.op },
    style: {
      background: (colors[statusMap[n.id] ?? 'pending']) + '22',
      border: `1.5px solid ${colors[statusMap[n.id] ?? 'pending']}55`,
      borderRadius: 8, fontSize: 11, color: 'var(--tb-text)', padding: '6px 10px',
    },
  }))

  const edges: Edge[] = []
  defNodes.forEach((n) => {
    n.needs.forEach((dep) => {
      edges.push({
        id: `${dep}->${n.id}`, source: dep, target: n.id,
        markerEnd: { type: MarkerType.ArrowClosed, width: 10, height: 10, color: '#6b7280' },
        style: { stroke: '#6b7280', strokeWidth: 1.5 },
      })
    })
  })

  return { nodes, edges }
}

function DagView({ defNodes, nodeRuns }: { defNodes: PipelineNodeDef[]; nodeRuns?: NodeRunRecord[] }) {
  const { nodes: initNodes, edges: initEdges } = toFlow(defNodes, nodeRuns)
  const [nodes, , onNodesChange] = useNodesState(initNodes)
  const [edges, setEdges, onEdgesChange] = useEdgesState(initEdges)
  const onConnect = useCallback((params: Connection) => setEdges((eds) => addEdge(params, eds)), [setEdges])

  return (
    <div style={{ height: 340 }} className="rounded-xl border border-border overflow-hidden bg-surface">
      <ReactFlow
        nodes={nodes} edges={edges}
        onNodesChange={onNodesChange} onEdgesChange={onEdgesChange} onConnect={onConnect}
        fitView proOptions={{ hideAttribution: true }}
      >
        <Background color="var(--tb-border)" gap={20} />
        <Controls style={{ background: 'var(--tb-surface-2)', border: '1px solid var(--tb-border)', borderRadius: 8 }} />
        <MiniMap style={{ background: 'var(--tb-surface-2)', border: '1px solid var(--tb-border)' }} maskColor="transparent" />
      </ReactFlow>
    </div>
  )
}

// ── Create form ──────────────────────────────────────────────────────────────

const inputCls = 'w-full rounded-lg border border-border bg-surface-2 px-3 py-2 text-sm text-text focus:border-accent focus:outline-none'

function CreatePipelineForm({ onClose }: { onClose: () => void }) {
  const [name, setName] = useState('')
  const [tpl, setTpl] = useState<TemplateKey>('train')
  const [assets, setAssets] = useState('')
  const [timeframes, setTimeframes] = useState('')
  const [scheduleOn, setScheduleOn] = useState(false)
  const [schedInstrument, setSchedInstrument] = useState('BTC-USD')
  const [schedTimeframe, setSchedTimeframe] = useState('1h')
  const [everyNBars, setEveryNBars] = useState(24)

  const createMut = useCreatePipeline()

  function submit(e: React.FormEvent) {
    e.preventDefault()
    if (!name.trim()) return

    const csv = (s: string) => s.split(',').map((x) => x.trim()).filter(Boolean)
    const asset = csv(assets)
    const timeframe = csv(timeframes)
    const matrix = asset.length || timeframe.length ? { asset, timeframe, window: [] } : null

    const definition: PipelineDefinition = {
      schema_version: '1.1',
      kind: TEMPLATES[tpl].kind,
      name: name.trim(),
      dag: TEMPLATES[tpl].dag,
      template: false,
      matrix,
      schedule: scheduleOn
        ? { reference_instrument: schedInstrument, timeframe: schedTimeframe, every_n_bars: everyNBars }
        : null,
    }

    createMut.mutate(definition, { onSuccess: onClose })
  }

  return (
    <form onSubmit={submit} className="rounded-xl border border-accent/30 bg-surface p-5 space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-text">New Pipeline</h3>
        <button type="button" onClick={onClose} className="text-text-muted hover:text-text">
          <X className="h-4 w-4" />
        </button>
      </div>

      <div className="space-y-1">
        <label className="text-xs text-text-muted">Name</label>
        <input className={inputCls} value={name} onChange={(e) => setName(e.target.value)} placeholder="Daily BTC retrain" required />
      </div>

      <div className="space-y-1">
        <label className="text-xs text-text-muted">Template</label>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          {(Object.keys(TEMPLATES) as TemplateKey[]).map((k) => (
            <button
              key={k} type="button" onClick={() => setTpl(k)}
              className={cn('rounded-lg border p-3 text-left transition-colors',
                tpl === k ? 'border-accent bg-accent/10' : 'border-border bg-surface-2 hover:border-border-hover')}
            >
              <div className="text-xs font-medium text-text mb-1">{TEMPLATES[k].label}</div>
              <div className="text-xs text-text-muted">{TEMPLATES[k].description}</div>
            </button>
          ))}
        </div>
      </div>

      {/* Fan-out */}
      <div className="space-y-2 rounded-lg border border-border bg-surface-2/40 p-3">
        <div className="flex items-center gap-2 text-xs font-medium text-text">
          <GitFork className="h-3.5 w-3.5 text-accent" /> Fan-out (optional)
        </div>
        <div className="grid grid-cols-2 gap-2">
          <div className="space-y-1">
            <label className="text-xs text-text-muted">Assets (comma-separated)</label>
            <input className={inputCls} value={assets} onChange={(e) => setAssets(e.target.value)} placeholder="BTC-USD, ETH-USD" />
          </div>
          <div className="space-y-1">
            <label className="text-xs text-text-muted">Timeframes (comma-separated)</label>
            <input className={inputCls} value={timeframes} onChange={(e) => setTimeframes(e.target.value)} placeholder="1h, 1d" />
          </div>
        </div>
        <p className="text-xs text-text-dim">Each asset × timeframe combination runs as its own cell.</p>
      </div>

      {/* Schedule */}
      <div className="space-y-2 rounded-lg border border-border bg-surface-2/40 p-3">
        <label className="flex items-center gap-2 text-xs font-medium text-text cursor-pointer">
          <input type="checkbox" checked={scheduleOn} onChange={(e) => setScheduleOn(e.target.checked)} />
          <CalendarClock className="h-3.5 w-3.5 text-accent" /> Bar-cadence schedule (optional)
        </label>
        {scheduleOn && (
          <div className="grid grid-cols-3 gap-2">
            <div className="space-y-1">
              <label className="text-xs text-text-muted">Reference instrument</label>
              <input className={inputCls} value={schedInstrument} onChange={(e) => setSchedInstrument(e.target.value)} />
            </div>
            <div className="space-y-1">
              <label className="text-xs text-text-muted">Timeframe</label>
              <input className={inputCls} value={schedTimeframe} onChange={(e) => setSchedTimeframe(e.target.value)} />
            </div>
            <div className="space-y-1">
              <label className="text-xs text-text-muted">Every N bars</label>
              <input type="number" className={inputCls} value={everyNBars} onChange={(e) => setEveryNBars(+e.target.value)} />
            </div>
          </div>
        )}
      </div>

      <div className="flex items-center justify-end gap-2 pt-1">
        <Button type="button" variant="outline" size="sm" onClick={onClose}>Cancel</Button>
        <Button type="submit" size="sm" disabled={!name.trim() || createMut.isPending}>
          {createMut.isPending && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
          Create Pipeline
        </Button>
      </div>

      {createMut.isError && (
        <p className="text-xs text-red-400">
          {(createMut.error as { response?: { data?: { error?: string } } })?.response?.data?.error ??
            'Failed to create pipeline'}
        </p>
      )}
    </form>
  )
}

// ── Main page ────────────────────────────────────────────────────────────────

export function PipelinesPage() {
  const { data: pipelines = [], isLoading } = usePipelines()
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [activeRunId, setActiveRunId] = useState<string | null>(null)
  const [showCreate, setShowCreate] = useState(false)
  const runMut = useRunPipeline()
  const deleteMut = useDeletePipeline()

  const selected = pipelines.find((p) => p.id === selectedId) ?? null

  const { data: runs = [] } = usePipelineRuns(selectedId)
  const latestRun = runs[0] ?? null
  const trackedRunId = activeRunId ?? latestRun?.id ?? null
  const { data: nodeRuns } = useNodeRuns(trackedRunId)

  function triggerRun() {
    if (!selectedId) return
    runMut.mutate(selectedId, { onSuccess: (data) => setActiveRunId(data.run_id) })
  }

  return (
    <div className="mx-auto w-full max-w-6xl px-6 py-6">
      <div className="mb-4 flex items-center gap-2">
        <Workflow className="h-6 w-6 text-accent" />
        <div>
          <h1 className="text-2xl font-semibold text-text">MLOps</h1>
          <p className="text-sm text-text-muted">Declarative training & inference pipelines — fan-out, scheduling, run history.</p>
        </div>
      </div>

      <MLOpsSubNav />

      {showCreate && (
        <div className="mb-5">
          <CreatePipelineForm onClose={() => setShowCreate(false)} />
        </div>
      )}

      <div className="grid grid-cols-12 gap-4">
        {/* Sidebar */}
        <div className="col-span-12 md:col-span-4 space-y-2">
          <div className="flex items-center justify-between mb-2">
            <span className="text-xs text-text-muted font-medium uppercase tracking-wide">Pipelines</span>
            <button
              onClick={() => setShowCreate(true)} disabled={showCreate}
              className="inline-flex items-center gap-1 text-xs text-text-muted hover:text-accent transition-colors disabled:opacity-40"
            >
              <Plus className="h-3.5 w-3.5" /> New
            </button>
          </div>

          {isLoading && (
            <div className="flex items-center gap-2 text-xs text-text-muted py-4">
              <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading…
            </div>
          )}

          {!isLoading && pipelines.length === 0 && (
            <div className="text-center py-8 rounded-xl border border-dashed border-border text-text-muted">
              <Workflow className="h-6 w-6 mx-auto mb-2 opacity-40" />
              <p className="text-xs">No pipelines yet</p>
              <button onClick={() => setShowCreate(true)} className="text-xs text-accent hover:underline mt-1">
                Create one
              </button>
            </div>
          )}

          {pipelines.map((p) => (
            <button
              key={p.id}
              onClick={() => { setSelectedId(p.id); setActiveRunId(null) }}
              className={cn('w-full text-left rounded-xl border p-3.5 transition-colors',
                selectedId === p.id ? 'border-accent bg-accent/10' : 'border-border bg-surface hover:border-border-hover')}
            >
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium text-text truncate">{p.name}</span>
                <ChevronRight className="h-3.5 w-3.5 text-text-muted shrink-0" />
              </div>
              <div className="flex items-center gap-2 mt-1">
                <span className="text-xs rounded bg-surface-2 px-1.5 py-0.5 text-text-muted">{p.kind}</span>
                <span className="text-xs text-text-muted">{p.definition.dag.length} nodes</span>
                {p.definition.matrix && (p.definition.matrix.asset.length > 0 || p.definition.matrix.timeframe.length > 0) && (
                  <span className="inline-flex items-center gap-0.5 text-xs text-accent"><GitFork className="h-3 w-3" /> fan-out</span>
                )}
                {p.definition.schedule && (
                  <span className="inline-flex items-center gap-0.5 text-xs text-amber-400"><CalendarClock className="h-3 w-3" /></span>
                )}
              </div>
            </button>
          ))}
        </div>

        {/* Main area */}
        <div className="col-span-12 md:col-span-8 space-y-4">
          {!selected ? (
            <div className="flex items-center justify-center h-64 rounded-xl border border-dashed border-border text-text-muted">
              <div className="text-center">
                <Workflow className="h-8 w-8 mx-auto mb-2 opacity-30" />
                <p className="text-sm">Select a pipeline to view its DAG</p>
              </div>
            </div>
          ) : (
            <>
              <div className="flex items-center justify-between">
                <div>
                  <h2 className="text-base font-semibold text-text">{selected.name}</h2>
                  <p className="text-xs text-text-muted mt-0.5">
                    {selected.kind}
                    {selected.definition.schedule &&
                      ` · every ${selected.definition.schedule.every_n_bars} ${selected.definition.schedule.timeframe} bars of ${selected.definition.schedule.reference_instrument}`}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <Button
                    size="sm" variant="outline"
                    onClick={() => { deleteMut.mutate(selected.id); setSelectedId(null) }}
                    disabled={deleteMut.isPending}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                  <Button size="sm" onClick={triggerRun} disabled={runMut.isPending}>
                    {runMut.isPending ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
                    Run
                  </Button>
                </div>
              </div>

              <DagView defNodes={selected.definition.dag} nodeRuns={nodeRuns} />

              {runs.length > 0 && (
                <div className="rounded-xl border border-border bg-surface p-4">
                  <h3 className="text-xs font-medium text-text-muted uppercase tracking-wide mb-3">Run History</h3>
                  <div className="space-y-2 max-h-52 overflow-y-auto">
                    {runs.slice(0, 12).map((run) => (
                      <button
                        key={run.id}
                        onClick={() => setActiveRunId(run.id)}
                        className={cn('w-full flex items-center gap-3 rounded-lg border px-3 py-2 text-left transition-colors',
                          trackedRunId === run.id ? 'border-accent bg-accent/5' : 'border-border hover:border-border-hover')}
                      >
                        <StatusChip status={run.status} />
                        <span className="text-xs text-text-muted truncate flex-1">{run.cell_label}</span>
                        {run.cached && <span className="text-xs text-emerald-400">cached</span>}
                        {run.started_at && (
                          <span className="text-xs text-text-muted shrink-0">{new Date(run.started_at).toLocaleTimeString()}</span>
                        )}
                      </button>
                    ))}
                  </div>
                </div>
              )}

              {nodeRuns && nodeRuns.length > 0 && (
                <div className="rounded-xl border border-border bg-surface p-4">
                  <h3 className="text-xs font-medium text-text-muted uppercase tracking-wide mb-3">
                    Node Progress (run {trackedRunId?.slice(0, 8)}…)
                  </h3>
                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                    {nodeRuns.map((nr) => (
                      <div key={nr.id} className="flex items-center gap-2 rounded-lg border border-border bg-surface-2 px-3 py-2">
                        <StatusChip status={nr.status} />
                        <div className="min-w-0">
                          <div className="text-xs font-medium text-text truncate">{nr.op}</div>
                          <div className="text-xs text-text-muted font-mono truncate">{nr.node_id}</div>
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  )
}
