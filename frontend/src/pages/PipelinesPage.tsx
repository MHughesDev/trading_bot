/**
 * I-6.8 — Pipeline Builder + run queue page.
 *
 * Left panel: list of pipeline definitions + run-queue status.
 * Right panel: ReactFlow DAG visualisation of the selected pipeline's nodes.
 * Bottom sheet: node-run progress for the latest run.
 */

import { useState, useCallback } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { api as client } from '@/lib/api'
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  addEdge,
  useNodesState,
  useEdgesState,
  type Node,
  type Edge,
  type Connection,
  MarkerType,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import {
  Workflow, Play, Loader2, Plus, ChevronRight,
  CheckCircle, AlertCircle, Clock, X,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

// ── Types ──────────────────────────────────────────────────────────────────

interface PipelineNode {
  id: string
  op: string
  needs: string[]
  params?: Record<string, unknown>
}

interface PipelineDefinition {
  pipeline_id: string
  slug: string
  display_name: string
  description?: string
  nodes: PipelineNode[]
  created_at: string
}

interface PipelineRun {
  run_id: string
  pipeline_id: string
  status: 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled'
  started_at?: string
  finished_at?: string
  error?: string
}

interface NodeRun {
  node_id: string
  op: string
  status: 'pending' | 'running' | 'succeeded' | 'failed' | 'skipped'
  started_at?: string
  finished_at?: string
  error?: string
}

// ── API hooks ──────────────────────────────────────────────────────────────

function usePipelines() {
  return useQuery({
    queryKey: ['pipelines'],
    queryFn: () =>
      client.get<{ pipelines: PipelineDefinition[] }>('/api/pipelines').then((r) => r.data),
  })
}

function usePipelineRuns(pipelineId: string | null) {
  return useQuery({
    queryKey: ['pipeline-runs', pipelineId],
    queryFn: () =>
      client
        .get<{ runs: PipelineRun[] }>(`/api/pipelines/${pipelineId}/runs`)
        .then((r) => r.data),
    enabled: !!pipelineId,
    refetchInterval: 5000,
  })
}

function useNodeRuns(runId: string | null) {
  return useQuery({
    queryKey: ['node-runs', runId],
    queryFn: () =>
      client
        .get<{ node_runs: NodeRun[] }>(`/api/pipelines/runs/${runId}/nodes`)
        .then((r) => r.data),
    enabled: !!runId,
    refetchInterval: 2000,
  })
}

function useRunPipeline() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (pipelineId: string) =>
      client.post<{ run_id: string }>(`/api/pipelines/${pipelineId}/run`).then((r) => r.data),
    onSuccess: (_, pid) => qc.invalidateQueries({ queryKey: ['pipeline-runs', pid] }),
  })
}

// ── Status chip ────────────────────────────────────────────────────────────

function StatusChip({ status }: { status: string }) {
  const map: Record<string, { icon: React.ReactNode; cls: string }> = {
    queued: {
      icon: <Clock className="h-3 w-3" />,
      cls: 'bg-zinc-500/15 text-zinc-400 border-zinc-500/30',
    },
    running: {
      icon: <Loader2 className="h-3 w-3 animate-spin" />,
      cls: 'bg-blue-500/15 text-blue-400 border-blue-500/30',
    },
    succeeded: {
      icon: <CheckCircle className="h-3 w-3" />,
      cls: 'bg-green-500/15 text-green-400 border-green-500/30',
    },
    failed: {
      icon: <AlertCircle className="h-3 w-3" />,
      cls: 'bg-red-500/15 text-red-400 border-red-500/30',
    },
    cancelled: {
      icon: <X className="h-3 w-3" />,
      cls: 'bg-zinc-500/15 text-zinc-400 border-zinc-500/30',
    },
    pending: {
      icon: <Clock className="h-3 w-3" />,
      cls: 'bg-zinc-500/15 text-zinc-400 border-zinc-500/30',
    },
    skipped: {
      icon: <ChevronRight className="h-3 w-3" />,
      cls: 'bg-zinc-500/15 text-zinc-400 border-zinc-500/30',
    },
  }
  const { icon, cls } = map[status] ?? map.queued
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs font-medium',
        cls,
      )}
    >
      {icon}
      {status}
    </span>
  )
}

// ── DAG Flow view ──────────────────────────────────────────────────────────

function toPipelineNodes(defNodes: PipelineNode[], nodeRuns?: NodeRun[]): { nodes: Node[]; edges: Edge[] } {
  const statusMap: Record<string, string> = {}
  nodeRuns?.forEach((nr) => { statusMap[nr.node_id] = nr.status })

  const nodeColors: Record<string, string> = {
    succeeded: '#22c55e',
    running: '#3b82f6',
    failed: '#ef4444',
    pending: '#6b7280',
    skipped: '#9ca3af',
  }

  const nodes: Node[] = defNodes.map((n, i) => ({
    id: n.id,
    type: 'default',
    position: { x: (i % 4) * 180 + 40, y: Math.floor(i / 4) * 120 + 40 },
    data: { label: n.op },
    style: {
      background: nodeColors[statusMap[n.id] ?? 'pending'] + '22',
      border: `1.5px solid ${nodeColors[statusMap[n.id] ?? 'pending']}55`,
      borderRadius: 8,
      fontSize: 11,
      color: 'var(--tb-text)',
      padding: '6px 10px',
    },
  }))

  const edges: Edge[] = []
  defNodes.forEach((n) => {
    n.needs.forEach((dep) => {
      edges.push({
        id: `${dep}->${n.id}`,
        source: dep,
        target: n.id,
        markerEnd: { type: MarkerType.ArrowClosed, width: 10, height: 10, color: '#6b7280' },
        style: { stroke: '#6b7280', strokeWidth: 1.5 },
      })
    })
  })

  return { nodes, edges }
}

function DagView({ pipeline, nodeRuns }: { pipeline: PipelineDefinition; nodeRuns?: NodeRun[] }) {
  const { nodes: initNodes, edges: initEdges } = toPipelineNodes(pipeline.nodes, nodeRuns)
  const [nodes, , onNodesChange] = useNodesState(initNodes)
  const [edges, setEdges, onEdgesChange] = useEdgesState(initEdges)

  const onConnect = useCallback(
    (params: Connection) => setEdges((eds) => addEdge(params, eds)),
    [setEdges],
  )

  return (
    <div style={{ height: 360 }} className="rounded-xl border border-border overflow-hidden bg-surface">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        fitView
        proOptions={{ hideAttribution: true }}
      >
        <Background color="var(--tb-border)" gap={20} />
        <Controls
          style={{ background: 'var(--tb-surface-2)', border: '1px solid var(--tb-border)', borderRadius: 8 }}
        />
        <MiniMap
          style={{ background: 'var(--tb-surface-2)', border: '1px solid var(--tb-border)' }}
          maskColor="transparent"
        />
      </ReactFlow>
    </div>
  )
}

// ── Main page ──────────────────────────────────────────────────────────────

export function PipelinesPage() {
  const { data, isLoading } = usePipelines()
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [activeRunId, setActiveRunId] = useState<string | null>(null)
  const runMut = useRunPipeline()

  const pipelines = data?.pipelines ?? []
  const selected = pipelines.find((p) => p.pipeline_id === selectedId) ?? null

  const { data: runsData } = usePipelineRuns(selectedId)
  const runs = runsData?.runs ?? []
  const latestRun = runs.at(0) ?? null

  const trackedRunId = activeRunId ?? latestRun?.run_id ?? null
  const { data: nodeRunsData } = useNodeRuns(trackedRunId)
  const nodeRuns = nodeRunsData?.node_runs

  function triggerRun() {
    if (!selectedId) return
    runMut.mutate(selectedId, {
      onSuccess: (data) => setActiveRunId(data.run_id),
    })
  }

  return (
    <div className="mx-auto w-full max-w-6xl px-6 py-6">
      <div className="flex items-center gap-3 mb-6">
        <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-surface-2 text-accent">
          <Workflow className="h-4.5 w-4.5" />
        </div>
        <div>
          <h1 className="text-lg font-semibold text-text">Pipeline Builder</h1>
          <p className="text-xs text-text-muted">
            Declarative DAG pipelines for training and inference workflows
          </p>
        </div>
      </div>

      <div className="grid grid-cols-12 gap-4">
        {/* Sidebar — pipeline list */}
        <div className="col-span-12 md:col-span-4 space-y-2">
          <div className="flex items-center justify-between mb-2">
            <span className="text-xs text-text-muted font-medium uppercase tracking-wide">
              Pipelines
            </span>
            <button
              className="inline-flex items-center gap-1 text-xs text-text-muted hover:text-accent transition-colors"
              title="Create pipeline via API or YAML import"
            >
              <Plus className="h-3.5 w-3.5" />
              Add
            </button>
          </div>

          {isLoading && (
            <div className="flex items-center gap-2 text-xs text-text-muted py-4">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              Loading…
            </div>
          )}

          {!isLoading && pipelines.length === 0 && (
            <div className="text-center py-8 rounded-xl border border-dashed border-border text-text-muted">
              <Workflow className="h-6 w-6 mx-auto mb-2 opacity-40" />
              <p className="text-xs">No pipelines defined</p>
              <p className="text-xs mt-0.5 opacity-60">POST to /api/pipelines to create one</p>
            </div>
          )}

          {pipelines.map((p) => (
            <button
              key={p.pipeline_id}
              onClick={() => {
                setSelectedId(p.pipeline_id)
                setActiveRunId(null)
              }}
              className={cn(
                'w-full text-left rounded-xl border p-3.5 transition-colors',
                selectedId === p.pipeline_id
                  ? 'border-accent bg-accent/10'
                  : 'border-border bg-surface hover:border-border-hover',
              )}
            >
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium text-text truncate">{p.display_name}</span>
                <ChevronRight className="h-3.5 w-3.5 text-text-muted shrink-0" />
              </div>
              <div className="text-xs text-text-muted font-mono mt-0.5">{p.slug}</div>
              <div className="text-xs text-text-muted mt-1">
                {p.nodes.length} nodes
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
              {/* Pipeline header */}
              <div className="flex items-center justify-between">
                <div>
                  <h2 className="text-base font-semibold text-text">{selected.display_name}</h2>
                  {selected.description && (
                    <p className="text-xs text-text-muted mt-0.5">{selected.description}</p>
                  )}
                </div>
                <Button
                  size="sm"
                  onClick={triggerRun}
                  disabled={runMut.isPending}
                >
                  {runMut.isPending ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Play className="h-3.5 w-3.5" />
                  )}
                  Run Pipeline
                </Button>
              </div>

              {/* DAG view */}
              <DagView pipeline={selected} nodeRuns={nodeRuns} />

              {/* Run history */}
              {runs.length > 0 && (
                <div className="rounded-xl border border-border bg-surface p-4">
                  <h3 className="text-xs font-medium text-text-muted uppercase tracking-wide mb-3">
                    Run History
                  </h3>
                  <div className="space-y-2 max-h-52 overflow-y-auto">
                    {runs.slice(0, 10).map((run) => (
                      <button
                        key={run.run_id}
                        onClick={() => setActiveRunId(run.run_id)}
                        className={cn(
                          'w-full flex items-center gap-3 rounded-lg border px-3 py-2 text-left transition-colors',
                          trackedRunId === run.run_id
                            ? 'border-accent bg-accent/5'
                            : 'border-border hover:border-border-hover',
                        )}
                      >
                        <StatusChip status={run.status} />
                        <span className="text-xs font-mono text-text-muted truncate flex-1">
                          {run.run_id.slice(0, 8)}…
                        </span>
                        {run.started_at && (
                          <span className="text-xs text-text-muted shrink-0">
                            {new Date(run.started_at).toLocaleTimeString()}
                          </span>
                        )}
                      </button>
                    ))}
                  </div>
                </div>
              )}

              {/* Node-run breakdown */}
              {nodeRuns && nodeRuns.length > 0 && (
                <div className="rounded-xl border border-border bg-surface p-4">
                  <h3 className="text-xs font-medium text-text-muted uppercase tracking-wide mb-3">
                    Node Progress (run {trackedRunId?.slice(0, 8)}…)
                  </h3>
                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                    {nodeRuns.map((nr) => (
                      <div
                        key={nr.node_id}
                        className="flex items-center gap-2 rounded-lg border border-border bg-surface-2 px-3 py-2"
                      >
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
