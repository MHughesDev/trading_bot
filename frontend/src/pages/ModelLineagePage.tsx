// Lineage Graph — /mlops/graph
//
// Renders the global model lineage graph using @xyflow/react.  Each node
// represents an entity in the pipeline (dataset version → training run →
// model version → deployment → strategy); edges show data/artifact flow.
//
// Per-model lineage can be reached from the Cockpit; this page shows the
// full cross-model view.
import { useCallback, useMemo } from 'react'
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  useNodesState,
  useEdgesState,
  type Node,
  type Edge,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { useQuery } from '@tanstack/react-query'
import { useParams, Link } from 'react-router-dom'
import { modelsApi, type LineageGraph } from '@/api/mlops'

// ── Node styles matching the .tb-node design language ────────────────────────

const NODE_COLORS: Record<string, string> = {
  dataset: 'var(--tb-accent-blue, #3b82f6)',
  training_run: 'var(--tb-accent-amber, #f59e0b)',
  model_version: 'var(--tb-accent-green, #22c55e)',
  deployment: 'var(--tb-accent-purple, #a855f7)',
  strategy: 'var(--tb-accent-cyan, #06b6d4)',
  default: 'var(--muted-foreground, #6b7280)',
}

function nodeColor(type: string) {
  return NODE_COLORS[type] ?? NODE_COLORS.default
}

function buildNodes(raw: LineageGraph['nodes']): Node[] {
  return raw.map((n, i) => ({
    id: n.id,
    type: 'default',
    position: { x: (i % 4) * 220, y: Math.floor(i / 4) * 130 },
    data: {
      label: (
        <div style={{ fontSize: 12, textAlign: 'center' }}>
          <div
            style={{
              background: nodeColor(n.type),
              color: '#fff',
              borderRadius: 4,
              padding: '2px 6px',
              marginBottom: 4,
              fontWeight: 600,
              fontSize: 10,
              textTransform: 'uppercase',
              letterSpacing: '0.05em',
            }}
          >
            {n.type.replace(/_/g, ' ')}
          </div>
          <div>{String(n.data?.label ?? n.id)}</div>
        </div>
      ),
    },
    style: {
      background: 'var(--card, #1a1d23)',
      border: `1px solid ${nodeColor(n.type)}`,
      borderRadius: 8,
      padding: '8px 12px',
      color: 'var(--foreground, #f1f5f9)',
      minWidth: 140,
    },
  }))
}

function buildEdges(raw: LineageGraph['edges']): Edge[] {
  return raw.map((e) => ({
    id: e.id,
    source: e.source,
    target: e.target,
    animated: false,
    style: { stroke: 'var(--border, #334155)', strokeWidth: 1.5 },
  }))
}

// ── Stub graph shown when no model is selected and no global data exists ─────

const STUB_NODES: Node[] = [
  {
    id: 'ds-1',
    position: { x: 0, y: 60 },
    data: { label: 'Dataset v1' },
    style: { background: '#1a1d23', border: '1px solid #3b82f6', borderRadius: 8, padding: '8px 12px', color: '#f1f5f9' },
  },
  {
    id: 'run-1',
    position: { x: 220, y: 60 },
    data: { label: 'Training run' },
    style: { background: '#1a1d23', border: '1px solid #f59e0b', borderRadius: 8, padding: '8px 12px', color: '#f1f5f9' },
  },
  {
    id: 'mv-1',
    position: { x: 440, y: 60 },
    data: { label: 'Model v1' },
    style: { background: '#1a1d23', border: '1px solid #22c55e', borderRadius: 8, padding: '8px 12px', color: '#f1f5f9' },
  },
  {
    id: 'dep-1',
    position: { x: 660, y: 60 },
    data: { label: 'Deployment (paper)' },
    style: { background: '#1a1d23', border: '1px solid #a855f7', borderRadius: 8, padding: '8px 12px', color: '#f1f5f9' },
  },
  {
    id: 'strat-1',
    position: { x: 880, y: 60 },
    data: { label: 'EMA + Forecast strategy' },
    style: { background: '#1a1d23', border: '1px solid #06b6d4', borderRadius: 8, padding: '8px 12px', color: '#f1f5f9' },
  },
]

const STUB_EDGES: Edge[] = [
  { id: 'e1', source: 'ds-1', target: 'run-1', style: { stroke: '#334155', strokeWidth: 1.5 } },
  { id: 'e2', source: 'run-1', target: 'mv-1', style: { stroke: '#334155', strokeWidth: 1.5 } },
  { id: 'e3', source: 'mv-1', target: 'dep-1', style: { stroke: '#334155', strokeWidth: 1.5 } },
  { id: 'e4', source: 'dep-1', target: 'strat-1', style: { stroke: '#334155', strokeWidth: 1.5 } },
]

// ── Per-model lineage panel ───────────────────────────────────────────────────

function ModelLineagePanel({ modelId }: { modelId: string }) {
  const { data, isLoading, isError } = useQuery({
    queryKey: ['models', modelId, 'lineage'],
    queryFn: () => modelsApi.lineage(modelId).then((r) => r.data),
  })

  const rawNodes = data?.nodes ?? []
  const rawEdges = data?.edges ?? []

  const initialNodes = useMemo(
    () => (rawNodes.length > 0 ? buildNodes(rawNodes) : STUB_NODES),
    [rawNodes],
  )
  const initialEdges = useMemo(
    () => (rawEdges.length > 0 ? buildEdges(rawEdges) : STUB_EDGES),
    [rawEdges],
  )

  const [nodes, , onNodesChange] = useNodesState(initialNodes)
  const [edges, , onEdgesChange] = useEdgesState(initialEdges)

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-full text-sm text-muted-foreground">
        Loading lineage…
      </div>
    )
  }

  if (isError) {
    return (
      <div className="flex items-center justify-center h-full text-sm text-destructive">
        Failed to load lineage graph
      </div>
    )
  }

  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      onNodesChange={onNodesChange}
      onEdgesChange={onEdgesChange}
      fitView
      fitViewOptions={{ padding: 0.2 }}
      nodesDraggable={false}
      nodesConnectable={false}
      elementsSelectable={false}
      panOnDrag
      zoomOnScroll
    >
      <Background color="var(--border, #1e293b)" gap={24} />
      <Controls showInteractive={false} />
      <MiniMap
        nodeColor={(n) => {
          const type = typeof n.data?.type === 'string' ? n.data.type : 'default'
          return nodeColor(type)
        }}
        maskColor="rgba(0,0,0,0.4)"
      />
    </ReactFlow>
  )
}

// ── Global lineage page ───────────────────────────────────────────────────────

export function ModelLineagePage() {
  // Optional ?model=<id> query param to show a specific model's lineage.
  const searchParams = new URLSearchParams(window.location.search)
  const focusId = searchParams.get('model')

  return (
    <div className="flex flex-col h-full min-h-[600px]">
      {/* Header */}
      <div className="flex items-center justify-between px-6 py-4 border-b bg-card/80 backdrop-blur">
        <div>
          <h1 className="text-lg font-semibold">Lineage Graph</h1>
          <p className="text-sm text-muted-foreground mt-0.5">
            {focusId
              ? `Showing lineage for model ${focusId}`
              : 'Global artifact flow: datasets → runs → versions → deployments → strategies'}
          </p>
        </div>
        <div className="flex items-center gap-3 text-xs text-muted-foreground">
          {/* Legend */}
          {[
            { label: 'Dataset', color: NODE_COLORS.dataset },
            { label: 'Training run', color: NODE_COLORS.training_run },
            { label: 'Model version', color: NODE_COLORS.model_version },
            { label: 'Deployment', color: NODE_COLORS.deployment },
            { label: 'Strategy', color: NODE_COLORS.strategy },
          ].map(({ label, color }) => (
            <span key={label} className="flex items-center gap-1">
              <span
                style={{ background: color, width: 8, height: 8, borderRadius: 2, display: 'inline-block' }}
              />
              {label}
            </span>
          ))}
        </div>
        <Link
          to="/mlops"
          className="text-xs text-muted-foreground hover:text-foreground transition-colors"
        >
          ← All models
        </Link>
      </div>

      {/* Canvas */}
      <div className="flex-1 bg-background">
        {focusId ? (
          <ModelLineagePanel modelId={focusId} />
        ) : (
          // Global view shows the stub graph until real cross-model lineage is wired.
          <GlobalLineageCanvas />
        )}
      </div>
    </div>
  )
}

function GlobalLineageCanvas() {
  const [nodes, , onNodesChange] = useNodesState(STUB_NODES)
  const [edges, , onEdgesChange] = useEdgesState(STUB_EDGES)

  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      onNodesChange={onNodesChange}
      onEdgesChange={onEdgesChange}
      fitView
      fitViewOptions={{ padding: 0.3 }}
      nodesDraggable={false}
      nodesConnectable={false}
      elementsSelectable={false}
      panOnDrag
      zoomOnScroll
    >
      <Background color="var(--border, #1e293b)" gap={24} />
      <Controls showInteractive={false} />
      <MiniMap maskColor="rgba(0,0,0,0.4)" />
    </ReactFlow>
  )
}
