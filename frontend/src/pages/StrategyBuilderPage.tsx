import { useCallback, useRef, useState, useEffect } from 'react'
import type { DragEvent } from 'react'
import {
  ReactFlow, Background, BackgroundVariant, Controls, MiniMap,
  addEdge, useNodesState, useEdgesState, ReactFlowProvider, useReactFlow,
} from '@xyflow/react'
import type { Node, Edge, NodeTypes, Connection, NodeMouseHandler } from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { api } from '@/lib/api'
import { compile } from '@/utils/compiler'
import type { SavedStrategy } from '@/types/spec'
import {
  IndicatorNode, ConditionNode, AIForecastNode, LogicNode, ActionNode, SizeNode, ExitNode,
} from '@/nodes'
import { Palette } from '@/components/strategy/Palette'
import { NodeContextMenu } from '@/components/strategy/NodeContextMenu'
import type { NodeMenuState } from '@/components/strategy/NodeContextMenu'

const nodeTypes: NodeTypes = {
  indicator: IndicatorNode,
  condition: ConditionNode,
  ai_forecast: AIForecastNode,
  logic: LogicNode,
  action: ActionNode,
  size: SizeNode,
  exit: ExitNode,
}

const INITIAL_NODES: Node[] = [
  { id: 'ind-1', type: 'indicator', position: { x: 60,  y: 130 }, data: { indicatorId: 'ema_fast', kind: 'ema', period: 7  } },
  { id: 'ind-2', type: 'indicator', position: { x: 60,  y: 290 }, data: { indicatorId: 'ema_slow', kind: 'ema', period: 21 } },
  { id: 'cond-1', type: 'condition', position: { x: 360, y: 200 }, data: { conditionType: 'cross_above', rightMode: 'indicator', rightValue: 0 } },
  { id: 'act-1',  type: 'action',    position: { x: 660, y: 200 }, data: { side: 'buy' } },
  { id: 'size-1', type: 'size',      position: { x: 940, y: 120 }, data: { sizeType: 'percent_of_equity', value: 0.02  } },
  { id: 'exit-1', type: 'exit',      position: { x: 940, y: 240 }, data: { exitType: 'stop_loss',   value: 0.015 } },
  { id: 'exit-2', type: 'exit',      position: { x: 940, y: 360 }, data: { exitType: 'take_profit', value: 0.04  } },
]

const INITIAL_EDGES: Edge[] = [
  { id: 'e1', source: 'ind-1',  sourceHandle: 'value-out',  target: 'cond-1', targetHandle: 'left-in',   animated: false },
  { id: 'e2', source: 'ind-2',  sourceHandle: 'value-out',  target: 'cond-1', targetHandle: 'right-in',  animated: false },
  { id: 'e3', source: 'cond-1', sourceHandle: 'cond-out',   target: 'act-1',  targetHandle: 'action-in', animated: true  },
  { id: 'e4', source: 'act-1',  sourceHandle: 'size-out',   target: 'size-1', targetHandle: 'size-in',   animated: false },
  { id: 'e5', source: 'act-1',  sourceHandle: 'exit-out',   target: 'exit-1', targetHandle: 'exit-in',   animated: false },
  { id: 'e6', source: 'act-1',  sourceHandle: 'exit-out',   target: 'exit-2', targetHandle: 'exit-in',   animated: false },
]

const MINIMAP_COLOR = (node: Node) =>
  ({ indicator: '#7C3AED', condition: '#B45309', ai_forecast: '#0E7490', logic: '#1E40AF', action: '#15803D', size: '#0D9488', exit: '#9F1239' }[node.type ?? ''] ?? 'var(--tb-border-2)')

let _counter = 200
const genId = () => `n-${++_counter}`

function activeGraph(nodes: Node[], edges: Edge[]): { nodes: Node[]; edges: Edge[] } {
  const activeIds = new Set(nodes.filter(n => !n.data?.disabled).map(n => n.id))
  return {
    nodes: nodes.filter(n => activeIds.has(n.id)),
    edges: edges.filter(e => activeIds.has(e.source) && activeIds.has(e.target)),
  }
}

// ── Preview bar ───────────────────────────────────────────────────────────────

function PreviewBar({ nodes, edges, name, editingId, onSaved }: {
  nodes: Node[]; edges: Edge[]; name: string
  editingId: string | null; onSaved: (s: SavedStrategy) => void
}) {
  const [explanation, setExplanation] = useState('')
  const [valid, setValid] = useState(false)
  const [errors, setErrors] = useState<string[]>([])
  const [warnings, setWarnings] = useState<string[]>([])
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState('')

  useEffect(() => {
    const active = activeGraph(nodes, edges)
    const { spec, errors: errs, warnings: warns } = compile(active.nodes, active.edges, name)
    setErrors(errs); setWarnings(warns)
    if (errs.length > 0 || !spec) { setExplanation(''); setValid(false); return }

    const t = setTimeout(async () => {
      try {
        const r = await api.post<{ valid: boolean; explanation?: string; errors?: string[] }>(
          '/strategies/custom/preview', spec)
        setExplanation(r.data.explanation ?? '')
        setValid(r.data.valid ?? false)
        if (r.data.errors?.length) setErrors(r.data.errors)
      } catch {
        setValid(false)
      }
    }, 500)
    return () => clearTimeout(t)
  }, [nodes, edges, name])

  async function handleSave() {
    const active = activeGraph(nodes, edges)
    const { spec, errors: errs } = compile(active.nodes, active.edges, name)
    if (!spec || errs.length > 0) return
    setSaving(true); setSaveError('')
    try {
      const r = editingId
        ? await api.put<SavedStrategy>(`/strategies/custom/${editingId}`, spec)
        : await api.post<SavedStrategy>('/strategies/custom', spec)
      onSaved(r.data)
    } catch (e) {
      setSaveError((e as Error).message)
    } finally { setSaving(false) }
  }

  const allErrors = errors

  return (
    <div style={{
      height: 68, background: 'var(--tb-surface)', borderTop: '1px solid var(--tb-border)',
      display: 'flex', alignItems: 'center', padding: '0 20px', gap: 16, flexShrink: 0,
    }}>
      <div style={{ flex: 1, minWidth: 0, overflow: 'hidden' }}>
        {allErrors.length > 0 ? (
          <div style={{ color: 'var(--tb-pnl-down)', fontSize: 12, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            ✗ {allErrors[0]}
            {allErrors.length > 1 && <span style={{ color: 'var(--tb-text-dim)', marginLeft: 8 }}>+{allErrors.length - 1} more</span>}
          </div>
        ) : explanation ? (
          <div style={{ color: 'var(--tb-text)', fontSize: 12, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            <span style={{ color: 'var(--tb-text-dim)', marginRight: 8 }}>Preview:</span>{explanation}
          </div>
        ) : (
          <div style={{ color: 'var(--tb-border-2)', fontSize: 12 }}>Connect nodes to build your strategy…</div>
        )}
        {warnings.length > 0 && (
          <div style={{ color: 'var(--tb-warning)', fontSize: 11, marginTop: 2 }}>⚠ {warnings[0]}</div>
        )}
        {saveError && <div style={{ color: 'var(--tb-pnl-down)', fontSize: 11, marginTop: 2 }}>{saveError}</div>}
      </div>
      <button
        onClick={handleSave}
        disabled={!valid || saving}
        style={{
          background: valid ? '#15803D' : 'var(--tb-background)', color: valid ? '#fff' : 'var(--tb-border-2)',
          border: `1px solid ${valid ? '#15803D' : 'var(--tb-border)'}`, borderRadius: 8,
          padding: '8px 22px', fontSize: 13, fontWeight: 600,
          cursor: valid && !saving ? 'pointer' : 'not-allowed',
          fontFamily: 'inherit', whiteSpace: 'nowrap', transition: 'all 0.15s', flexShrink: 0,
        }}
      >
        {saving ? 'Saving…' : editingId ? 'Save changes' : 'Save strategy'}
      </button>
    </div>
  )
}

// ── Canvas ────────────────────────────────────────────────────────────────────

function Canvas() {
  const wrapper = useRef<HTMLDivElement>(null)
  const [nodes, setNodes, onNodesChange] = useNodesState(INITIAL_NODES)
  const [edges, setEdges, onEdgesChange] = useEdgesState(INITIAL_EDGES)
  const [name, setName] = useState('EMA Crossover')
  const [editingId, setEditingId] = useState<string | null>(null)
  const [savedLabel, setSavedLabel] = useState('')
  const [menu, setMenu] = useState<NodeMenuState | null>(null)
  const { screenToFlowPosition } = useReactFlow()

  const onConnect = useCallback(
    (params: Connection) => setEdges(eds => addEdge({ ...params, id: `e-${Date.now()}` }, eds)),
    [setEdges],
  )

  const onNodeContextMenu = useCallback<NodeMouseHandler>((e, node) => {
    e.preventDefault()
    setMenu({ nodeId: node.id, x: e.clientX, y: e.clientY, disabled: !!(node.data as { disabled?: boolean }).disabled })
  }, [])

  const closeMenu = useCallback(() => setMenu(null), [])

  const duplicateNode = useCallback((id: string) => {
    setNodes(nds => {
      const src = nds.find(n => n.id === id)
      if (!src) return nds
      const clone: Node = {
        ...src,
        id: genId(),
        position: { x: src.position.x + 48, y: src.position.y + 48 },
        data: { ...src.data },
        selected: false,
      }
      return [...nds.map(n => ({ ...n, selected: false })), clone]
    })
  }, [setNodes])

  const toggleDisabled = useCallback((id: string) => {
    setNodes(nds => nds.map(n => n.id === id ? { ...n, data: { ...n.data, disabled: !n.data.disabled } } : n))
  }, [setNodes])

  const disconnectNode = useCallback((id: string) => {
    setEdges(eds => eds.filter(e => e.source !== id && e.target !== id))
  }, [setEdges])

  const deleteNode = useCallback((id: string) => {
    setNodes(nds => nds.filter(n => n.id !== id))
    setEdges(eds => eds.filter(e => e.source !== id && e.target !== id))
  }, [setNodes, setEdges])

  const onDragOver = useCallback((e: DragEvent) => { e.preventDefault(); e.dataTransfer.dropEffect = 'move' }, [])

  const onDrop = useCallback((e: DragEvent) => {
    e.preventDefault()
    const raw = e.dataTransfer.getData('application/reactflow')
    if (!raw) return
    const { type, data } = JSON.parse(raw) as { type: string; data: Record<string, unknown> }
    const position = screenToFlowPosition({ x: e.clientX, y: e.clientY })
    setNodes(nds => [...nds, { id: genId(), type, position, data: { ...data } }])
  }, [screenToFlowPosition, setNodes])

  function handleSaved(s: SavedStrategy) {
    setEditingId(s.id)
    setSavedLabel(`"${s.name}" saved`)
    setTimeout(() => setSavedLabel(''), 3000)
  }

  function handleClear() {
    if (!confirm('Clear the canvas and start fresh?')) return
    setNodes([]); setEdges([]); setEditingId(null); setName('My Strategy')
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', width: '100%', height: '100%', background: 'var(--tb-background)' }}>
      {/* Toolbar */}
      <div style={{
        height: 46, background: 'var(--tb-surface)', borderBottom: '1px solid var(--tb-border)',
        display: 'flex', alignItems: 'center', padding: '0 16px', gap: 10, flexShrink: 0,
      }}>
        <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--tb-text)' }}>Strategy Builder</span>
        <div style={{ flex: 1 }} />
        {savedLabel && <span style={{ fontSize: 11, color: 'var(--tb-pnl-up)', fontWeight: 500 }}>✓ {savedLabel}</span>}
        <input
          value={name}
          onChange={e => setName(e.target.value)}
          placeholder="Strategy name"
          style={{
            background: 'var(--tb-background)', border: '1px solid var(--tb-border-2)', borderRadius: 7,
            padding: '5px 12px', color: 'var(--tb-text)', fontSize: 13, fontFamily: 'inherit',
            outline: 'none', width: 200,
          }}
          onFocus={e => (e.target.style.borderColor = 'var(--tb-text-dim)')}
          onBlur={e => (e.target.style.borderColor = 'var(--tb-border-2)')}
        />
        <button
          onClick={handleClear}
          style={{
            background: 'transparent', border: '1px solid var(--tb-border-2)', borderRadius: 7,
            padding: '5px 12px', color: 'var(--tb-text-dim)', fontSize: 12, cursor: 'pointer', fontFamily: 'inherit',
          }}
          onMouseEnter={e => (e.currentTarget.style.borderColor = 'var(--tb-text-dim)')}
          onMouseLeave={e => (e.currentTarget.style.borderColor = 'var(--tb-border-2)')}
        >
          Clear
        </button>
      </div>

      {/* Palette + canvas */}
      <div style={{ display: 'flex', flex: 1, overflow: 'hidden', minHeight: 0 }}>
        <Palette />
        <div ref={wrapper} style={{ flex: 1, position: 'relative' }}>
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            onDrop={onDrop}
            onDragOver={onDragOver}
            onNodeContextMenu={onNodeContextMenu}
            onPaneClick={closeMenu}
            onMove={closeMenu}
            nodeTypes={nodeTypes}
            fitView
            fitViewOptions={{ padding: 0.15 }}
            deleteKeyCode="Delete"
            style={{ background: 'var(--tb-background)' }}
            defaultEdgeOptions={{ style: { stroke: 'var(--tb-text-dim)', strokeWidth: 2 } }}
          >
            <Background variant={BackgroundVariant.Dots} gap={24} size={1} color="var(--tb-border-2)" />
            <Controls position="bottom-right" />
            <MiniMap
              nodeColor={MINIMAP_COLOR}
              maskColor="rgba(3,7,18,0.7)"
              style={{ background: 'var(--tb-surface)', border: '1px solid var(--tb-border)', borderRadius: 8 }}
              position="top-right"
            />
          </ReactFlow>
          {menu && (
            <NodeContextMenu
              menu={menu}
              onClose={closeMenu}
              onDuplicate={duplicateNode}
              onToggleDisabled={toggleDisabled}
              onDisconnect={disconnectNode}
              onDelete={deleteNode}
            />
          )}
        </div>
      </div>

      <PreviewBar nodes={nodes} edges={edges} name={name} editingId={editingId} onSaved={handleSaved} />
    </div>
  )
}

export function StrategyBuilderPage() {
  return (
    <ReactFlowProvider>
      <Canvas />
    </ReactFlowProvider>
  )
}
