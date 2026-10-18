// Lineage graph (N-5.9): dataset -> features -> model version -> prediction
// series -> backtest, keyed by content hash. Built by `buildHashLineage` from the
// registry's versions; a link the registry did not record is a drawn gap, not an
// invented node. The same chain is also listed as text so it is readable without
// the canvas.
import { useMemo } from 'react'
import { ReactFlow, Background, Controls, type Edge, type Node, type NodeProps } from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { useModelAliases, useModelVersions } from '@/hooks/useMlOps'
import {
  LINK_NAMES,
  buildHashLineage,
  layoutHashLineage,
  type HashNode,
  type LinkKind,
} from '@/lib/models/lineage'

const COLORS: Record<LinkKind, string> = {
  dataset: 'var(--chart-1)',
  features: 'var(--chart-2)',
  model_version: 'var(--chart-3)',
  series: 'var(--chart-4)',
  backtest: 'var(--chart-5)',
}

function HashNodeView({ data }: NodeProps) {
  const d = data as { kind: LinkKind; label: string; recorded: boolean; hash: string | null }
  return (
    <div
      title={d.hash ?? undefined}
      style={{
        background: 'var(--node-bg)',
        border: `1px ${d.recorded ? 'solid' : 'dashed'} ${COLORS[d.kind]}`,
        borderRadius: 'var(--r-md)',
        minWidth: 150,
        color: d.recorded ? 'var(--fg-primary)' : 'var(--fg-tertiary)',
        fontSize: 12,
        padding: '6px 8px',
      }}
    >
      <div style={{ fontSize: 10, textTransform: 'uppercase', letterSpacing: '0.05em', color: COLORS[d.kind] }}>
        {LINK_NAMES[d.kind]}
      </div>
      {d.label}
    </div>
  )
}

const NODE_TYPES = { hash: HashNodeView }

export function LineageGraphView({ modelId }: { modelId: string }) {
  const versions = useModelVersions(modelId)
  const aliases = useModelAliases(modelId)

  const graph = useMemo(() => {
    if (!versions.data) return null
    const g = buildHashLineage(versions.data, aliases.data)
    const pos = layoutHashLineage(g.nodes)
    const nodes: Node[] = g.nodes.map((n: HashNode) => ({
      id: n.id,
      type: 'hash',
      position: pos.get(n.id) ?? { x: 0, y: 0 },
      data: { kind: n.kind, label: n.label, recorded: n.recorded, hash: n.hash },
    }))
    const edges: Edge[] = g.edges.map((e) => ({
      id: e.id,
      source: e.source,
      target: e.target,
      style: { stroke: 'var(--edge)', strokeWidth: 1.5 },
    }))
    return { raw: g, nodes, edges }
  }, [versions.data, aliases.data])

  if (versions.isLoading) return <PanelLoading lines={6} />
  if (versions.isError || !graph) {
    return <ErrorState message="Lineage could not be built." detail="The model's versions could not be read." onRetry={() => void versions.refetch()} />
  }
  if (graph.nodes.length === 0) {
    return <EmptyState message="No lineage yet" detail="Train a version; its dataset, features and results link here by hash." />
  }

  const gaps = graph.raw.nodes.filter((n) => !n.recorded).length
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <div style={{ display: 'flex', gap: 'var(--s-4)', flexWrap: 'wrap', fontSize: 'var(--t-11)' }}>
        {(Object.keys(LINK_NAMES) as LinkKind[]).map((k) => (
          <span key={k} style={{ display: 'inline-flex', gap: 4, alignItems: 'center' }}>
            <i style={{ width: 8, height: 8, borderRadius: 2, background: COLORS[k], display: 'inline-block' }} />
            {LINK_NAMES[k]}
          </span>
        ))}
        {gaps > 0 && (
          <span className="mut" data-testid="lineage-gaps">
            Dashed nodes are links the registry did not record ({gaps}).
          </span>
        )}
      </div>
      <div style={{ height: 420, border: '1px solid var(--line-hairline)', borderRadius: 'var(--r-md)' }}>
        <ReactFlow
          nodes={graph.nodes}
          edges={graph.edges}
          nodeTypes={NODE_TYPES}
          fitView
          fitViewOptions={{ padding: 0.2 }}
          nodesDraggable={false}
          nodesConnectable={false}
        >
          <Background color="var(--canvas-dot)" gap={24} />
          <Controls showInteractive={false} />
        </ReactFlow>
      </div>
      <ul aria-label="Lineage links" style={{ margin: 0, paddingLeft: 'var(--s-4)', fontSize: 'var(--t-12)' }}>
        {graph.raw.edges.map((e) => {
          const s = graph.raw.nodes.find((n) => n.id === e.source)
          const t = graph.raw.nodes.find((n) => n.id === e.target)
          return (
            <li key={e.id} data-testid="lineage-link">
              {s ? `${LINK_NAMES[s.kind]} ${s.label}` : e.source} to {t ? `${LINK_NAMES[t.kind]} ${t.label}` : e.target}
            </li>
          )
        })}
      </ul>
    </div>
  )
}
