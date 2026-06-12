import { useCallback, useRef, useState, useEffect } from 'react'
import type { DragEvent } from 'react'
import {
  ReactFlow, Background, BackgroundVariant, Controls,
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

let _counter = 200
const genId = () => `n-${++_counter}`

function activeGraph(nodes: Node[], edges: Edge[]): { nodes: Node[]; edges: Edge[] } {
  const activeIds = new Set(nodes.filter(n => !n.data?.disabled).map(n => n.id))
  return {
    nodes: nodes.filter(n => activeIds.has(n.id)),
    edges: edges.filter(e => activeIds.has(e.source) && activeIds.has(e.target)),
  }
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
  const [valid, setValid] = useState(false)
  const [saving, setSaving] = useState(false)
  const { screenToFlowPosition } = useReactFlow()

  useEffect(() => {
    const active = activeGraph(nodes, edges)
    const { spec, errors: errs } = compile(active.nodes, active.edges, name)
    if (errs.length > 0 || !spec) { setValid(false); return }
    const t = setTimeout(async () => {
      try {
        const r = await api.post<{ valid: boolean }>('/strategies/custom/preview', spec)
        setValid(r.data.valid ?? false)
      } catch { setValid(false) }
    }, 500)
    return () => clearTimeout(t)
  }, [nodes, edges, name])

  async function handleSave() {
    const active = activeGraph(nodes, edges)
    const { spec, errors: errs } = compile(active.nodes, active.edges, name)
    if (!spec || errs.length > 0) return
    setSaving(true)
    try {
      const r = editingId
        ? await api.put<SavedStrategy>(`/strategies/custom/${editingId}`, spec)
        : await api.post<SavedStrategy>('/strategies/custom', spec)
      setEditingId(r.data.id)
      setSavedLabel(`"${r.data.name}" saved`)
      setTimeout(() => setSavedLabel(''), 3000)
    } catch { /* no-op */ } finally { setSaving(false) }
  }

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
        <button
          onClick={handleSave}
          disabled={!valid || saving}
          style={{
            background: valid ? '#15803D' : 'var(--tb-background)', color: valid ? '#fff' : 'var(--tb-border-2)',
            border: `1px solid ${valid ? '#15803D' : 'var(--tb-border)'}`, borderRadius: 7,
            padding: '5px 14px', fontSize: 12, fontWeight: 600,
            cursor: valid && !saving ? 'pointer' : 'not-allowed',
            fontFamily: 'inherit', whiteSpace: 'nowrap', transition: 'all 0.15s',
          }}
        >
          {saving ? 'Saving…' : editingId ? 'Save changes' : 'Save strategy'}
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
