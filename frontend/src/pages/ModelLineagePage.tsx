// Lineage Graph — /mlops/graph
//
// Renders the global model lineage graph using @xyflow/react.  Each node
// represents an entity in the pipeline (dataset version → training run →
// model version → deployment → strategy); edges show data/artifact flow.
//
// Per-model lineage can be reached from the Cockpit; this page shows the
// full cross-model view.
import { ReactFlow, Background, Controls, MiniMap, useNodesState, useEdgesState, type Node, type Edge } from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { modelsApi, type LineageGraph } from '@/api/mlops'

/* Lineage node identity uses the fixed categorical Instrument scale in slot
   order (spec §2.8.3, V7). Slots are assigned once here and never cycled, so a
   dataset is the same colour on every lineage graph in the product. */

const NODE_COLORS: Record<string, string> = {
  dataset: 'var(--chart-1)',
  training_run: 'var(--chart-2)',
  model_version: 'var(--chart-3)',
  deployment: 'var(--chart-4)',
  strategy: 'var(--chart-5)',
  default: 'var(--fg-tertiary)',
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
              color: 'var(--fg-on-accent)',
              borderRadius: 'var(--r-xs)',
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
      background: 'var(--node-bg)',
      border: `1px solid ${nodeColor(n.type)}`,
      borderRadius: 'var(--r-md)',
      padding: '8px 12px',
      color: 'var(--fg-primary)',
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
    style: { stroke: 'var(--edge)', strokeWidth: 1.5 },
  }))
}

// ── Empty state when no lineage data is available ─────────────────────────────

function EmptyState({ title, message }: { title: string; message: string }) {
  return (
    <div className="flex flex-col items-center justify-center h-full gap-4">
      <div className="text-6xl">📊</div>
      <div className="text-center">
        <h3 className="text-lg font-semibold text-fg">{title}</h3>
        <p className="text-sm text-fg-tertiary max-w-sm mt-1">{message}</p>
      </div>
    </div>
  )
}

// ── Per-model lineage panel ───────────────────────────────────────────────────

function ModelLineagePanel({ modelId }: { modelId: string }) {
  const { data, isLoading, isError } = useQuery({
    queryKey: ['models', modelId, 'lineage'],
    queryFn: () => modelsApi.lineage(modelId).then((r) => r.data),
  })

  const rawNodes = data?.nodes ?? []
  const rawEdges = data?.edges ?? []

  const [nodes, , onNodesChange] = useNodesState(
    rawNodes.length > 0 ? buildNodes(rawNodes) : [],
  )
  const [edges, , onEdgesChange] = useEdgesState(
    rawEdges.length > 0 ? buildEdges(rawEdges) : [],
  )

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-full text-sm text-fg-tertiary">
        Loading lineage…
      </div>
    )
  }

  if (isError) {
    return (
      <EmptyState
        title="Lineage unavailable"
        message="Could not load the model's lineage graph. Try again later."
      />
    )
  }

  if (rawNodes.length === 0) {
    return (
      <EmptyState
        title="No lineage data yet"
        message="Once you train versions and deploy this model to a strategy, its lineage history will appear here."
      />
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
      <Background color="var(--canvas-dot)" gap={24} />
      <Controls showInteractive={false} />
      <MiniMap
        nodeColor={(n) => {
          const type = typeof n.data?.type === 'string' ? n.data.type : 'default'
          return nodeColor(type)
        }}
        maskColor="var(--bg-scrim)"
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
      <div className="flex items-center justify-between px-6 py-4 border-b bg-surface backdrop-blur">
        <div>
          <h1 className="text-lg font-semibold">Lineage Graph</h1>
          <p className="text-sm text-fg-tertiary mt-0.5">
            {focusId
              ? `Showing lineage for model ${focusId}`
              : 'Global artifact flow: datasets → runs → versions → deployments → strategies'}
          </p>
        </div>
        <div className="flex items-center gap-3 text-xs text-fg-tertiary">
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
          className="text-xs text-fg-tertiary hover:text-fg transition-colors"
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
  return (
    <EmptyState
      title="Global lineage coming soon"
      message="The cross-model artifact flow visualization is being developed. For now, view individual model lineage from the model detail pages."
    />
  )
}
