// Lineage Graph -- /mlops/graph?model=<id>
//
// `GET /api/models/{id}/lineage` answers aliases only (its own `note` says the
// full graph waits on dataset refs), so the graph is DERIVED here from what the
// registry does serve -- versions, training runs, deployments, aliases and the
// strategies that reference the model -- and the page says so. Datasets are not
// drawn: nothing the registry serves names one.
import { useMemo, useState } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router-dom'
import { ReactFlow, Background, Controls, MiniMap, type Node, type Edge, type NodeProps } from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { Button } from '@/components/primitives/Button'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { Segmented } from '@/components/primitives/Segmented'
import { LineageGraphView } from '@/components/models/LineageGraphView'
import {
  useModel,
  useModels,
  useModelAliases,
  useModelDeployments,
  useModelRuns,
  useModelUsedBy,
  useModelVersions,
} from '@/hooks/useMlOps'
import { KIND_LABELS, deriveLineage, layoutLineage, type LineageNode } from '@/components/mlops/studio.helpers'

/* Entity identity uses the fixed categorical scale in slot order (spec §2.8.3),
   assigned once here so a deployment is the same colour on every graph. */
const NODE_COLORS: Record<LineageNode['type'], string> = {
  training_run: 'var(--chart-2)',
  model: 'var(--chart-1)',
  model_version: 'var(--chart-3)',
  deployment: 'var(--chart-4)',
  strategy: 'var(--chart-5)',
}

const NODE_NAMES: Record<LineageNode['type'], string> = {
  training_run: 'Training run',
  model: 'Model',
  model_version: 'Model version',
  deployment: 'Deployment',
  strategy: 'Strategy',
}

function LineageNodeView({ data }: NodeProps) {
  const d = data as { type: LineageNode['type']; label: string }
  return (
    <div
      style={{
        background: 'var(--node-bg)',
        border: `1px solid ${NODE_COLORS[d.type]}`,
        borderRadius: 'var(--r-md)',
        minWidth: 150,
        overflow: 'hidden',
        color: 'var(--fg-primary)',
        fontSize: 12,
      }}
    >
      <div
        style={{
          background: NODE_COLORS[d.type],
          color: 'var(--fg-on-accent)',
          padding: '2px 8px',
          fontWeight: 600,
          fontSize: 10,
          textTransform: 'uppercase',
          letterSpacing: '0.05em',
        }}
      >
        {NODE_NAMES[d.type]}
      </div>
      <div style={{ padding: '6px 8px' }}>{d.label}</div>
    </div>
  )
}

const NODE_TYPES = { lineage: LineageNodeView }

function ModelLineagePanel({ modelId }: { modelId: string }) {
  const model = useModel(modelId)
  const versions = useModelVersions(modelId)
  const runs = useModelRuns(modelId)
  const deployments = useModelDeployments(modelId)
  const aliases = useModelAliases(modelId)
  const usedBy = useModelUsedBy(modelId)
  const navigate = useNavigate()

  const graph = useMemo(() => {
    if (!model.data || !versions.data) return null
    const g = deriveLineage({
      modelId,
      modelName: model.data.display_name,
      versions: versions.data,
      runs: runs.data ?? [],
      deployments: deployments.data ?? [],
      aliases: aliases.data ?? {},
      usedBy: usedBy.data ?? [],
    })
    const pos = layoutLineage(g.nodes)
    const nodes: Node[] = g.nodes.map((n) => ({
      id: n.id,
      type: 'lineage',
      position: pos.get(n.id) ?? { x: 0, y: 0 },
      data: { type: n.type, label: n.label },
    }))
    const edges: Edge[] = g.edges.map((e) => ({
      id: e.id,
      source: e.source,
      target: e.target,
      style: { stroke: 'var(--edge)', strokeWidth: 1.5 },
    }))
    return { nodes, edges }
  }, [model.data, versions.data, runs.data, deployments.data, aliases.data, usedBy.data, modelId])

  if (model.isLoading || versions.isLoading) return <PanelLoading lines={6} />
  if (model.isError || versions.isError || !graph) {
    return (
      <ErrorState
        message="Lineage could not be built."
        detail="The model or its versions could not be read."
        onRetry={() => {
          void model.refetch()
          void versions.refetch()
        }}
      />
    )
  }
  if (graph.nodes.length <= 1) {
    return (
      <EmptyState
        message="No lineage yet"
        detail="Train a version and deploy it to a strategy; its history appears here."
      />
    )
  }

  return (
    <ReactFlow
      nodes={graph.nodes}
      edges={graph.edges}
      nodeTypes={NODE_TYPES}
      fitView
      fitViewOptions={{ padding: 0.2 }}
      nodesDraggable={false}
      nodesConnectable={false}
      panOnDrag
      zoomOnScroll
      onNodeClick={(_, node) => {
        if (node.id.startsWith('version:')) navigate(`/mlops/${modelId}?tab=versions`)
        else if (node.id.startsWith('deployment:')) navigate(`/mlops/${modelId}?tab=deployments`)
        else if (node.id.startsWith('run:')) navigate(`/mlops/${modelId}?tab=train`)
        else if (node.id.startsWith('model:')) navigate(`/mlops/${modelId}`)
      }}
    >
      <Background color="var(--canvas-dot)" gap={24} />
      <Controls showInteractive={false} />
      <MiniMap
        nodeColor={(n) => {
          const t = (n.data as { type?: LineageNode['type'] }).type
          return t ? NODE_COLORS[t] : 'var(--fg-tertiary)'
        }}
        maskColor="var(--bg-scrim)"
      />
    </ReactFlow>
  )
}

/** With no model chosen there is no cross-model graph to draw; offer the models. */
function ModelPicker() {
  const { data, isLoading, isError, refetch } = useModels()
  if (isLoading) return <PanelLoading lines={4} />
  if (isError) return <ErrorState message="Models could not be loaded." onRetry={() => void refetch()} />
  const models = data?.models ?? []
  if (models.length === 0) {
    return <EmptyState message="No models yet" detail="Create a model to see its lineage." />
  }
  return (
    <div style={{ padding: 'var(--s-5)' }}>
      <p className="mut" style={{ marginBottom: 'var(--s-3)' }}>
        Lineage is shown per model. Choose one.
      </p>
      <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }}>
        {models.map((m) => (
          <li key={m.model_id}>
            <Link to={`/mlops/graph?model=${m.model_id}`}>
              {m.display_name} <span className="mut">· {KIND_LABELS[m.model_kind]}</span>
            </Link>
          </li>
        ))}
      </ul>
    </div>
  )
}

export function ModelLineagePage() {
  const [params] = useSearchParams()
  const focusId = params.get('model')
  const navigate = useNavigate()
  // N-5.9: the content-hash chain is the default; the registry-derived graph stays.
  const [view, setView] = useState<'chain' | 'registry'>('chain')

  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Lineage</h1>
        <span className="lbl">
          {focusId
            ? 'Versions, training runs, deployments and the strategies that use this model'
            : 'Choose a model'}
        </span>
        <div className="spacer" />
        {focusId && (
          <Segmented<'chain' | 'registry'>
            ariaLabel="Lineage view"
            value={view}
            onChange={setView}
            options={[
              { value: 'chain', label: 'Content hashes' },
              { value: 'registry', label: 'Registry' },
            ]}
          />
        )}
        {focusId && (
          <Button size="sm" onClick={() => navigate(`/mlops/${focusId}`)}>
            Open cockpit
          </Button>
        )}
        <Button size="sm" onClick={() => navigate('/mlops')}>All models</Button>
      </div>
      <div className="page-body" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
        {focusId && view === 'registry' && (
          <div style={{ display: 'flex', gap: 'var(--s-4)', flexWrap: 'wrap', fontSize: 'var(--t-11)' }}>
            {(Object.keys(NODE_NAMES) as LineageNode['type'][]).map((t) => (
              <span key={t} style={{ display: 'inline-flex', gap: 4, alignItems: 'center' }}>
                <i style={{ width: 8, height: 8, borderRadius: 2, background: NODE_COLORS[t], display: 'inline-block' }} />
                {NODE_NAMES[t]}
              </span>
            ))}
            <span className="mut">Derived from the registry&apos;s versions, runs, deployments and used-by; datasets are not served, so none is drawn.</span>
          </div>
        )}
        <div style={{ flex: 1, minHeight: 520, border: '1px solid var(--line-hairline)', borderRadius: 'var(--r-md)' }}>
          {focusId ? (
            view === 'chain' ? <div style={{ padding: 'var(--s-3)' }}><LineageGraphView modelId={focusId} /></div> : <ModelLineagePanel modelId={focusId} />
          ) : (
            <ModelPicker />
          )}
        </div>
      </div>
    </>
  )
}
