import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { DragEvent } from 'react'
import {
  ReactFlow, Background, BackgroundVariant, Controls,
  addEdge, useNodesState, useEdgesState, ReactFlowProvider, useReactFlow,
} from '@xyflow/react'
import type { Node, Edge, NodeTypes, Connection, NodeMouseHandler } from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { api, strategiesApi } from '@/lib/api'
import { compile, compileScanner } from '@/utils/compiler'
import { ruleSpecToDefinition, scannerToDefinition } from '@/utils/toDefinition'
import { fromDefinition } from '@/utils/fromDefinition'
import {
  IndicatorNode, ConditionNode, AIInferenceNode, LogicNode, ActionNode, SizeNode, ExitNode,
} from '@/nodes'
import { Palette } from '@/components/strategy/Palette'
import { NodeContextMenu } from '@/components/strategy/NodeContextMenu'
import type { NodeMenuState } from '@/components/strategy/NodeContextMenu'

const nodeTypes: NodeTypes = {
  indicator: IndicatorNode,
  condition: ConditionNode,
  ai_inference: AIInferenceNode,
  logic: LogicNode,
  action: ActionNode,
  size: SizeNode,
  exit: ExitNode,
}

const DEFAULT_STRATEGY_ID = 'ema_crossever'

let _counter = 200
const genId = () => `n-${++_counter}`

// ── Load picker ───────────────────────────────────────────────────────────────

interface SavedStrategy { id: string; strategy_id: string }

interface LoadPickerProps {
  onLoad: (nodes: Node[], edges: Edge[], name: string) => void
  onClose: () => void
}

function LoadPicker({ onLoad, onClose }: LoadPickerProps) {
  const [strategies, setStrategies] = useState<SavedStrategy[]>([])
  const [loading, setLoading] = useState(true)
  const [loadingId, setLoadingId] = useState<string | null>(null)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    strategiesApi.list().then((r) => {
      setStrategies((r.data as { strategies: SavedStrategy[] }).strategies ?? [])
    }).catch(() => {}).finally(() => setLoading(false))
  }, [])

  // Close on outside click
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as HTMLElement)) onClose()
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [onClose])

  async function handleSelect(s: SavedStrategy) {
    setLoadingId(s.id)
    try {
      const r = await strategiesApi.get(s.id)
      const def = (r.data as { definition: Parameters<typeof fromDefinition>[0] }).definition
      const { nodes, edges, name } = fromDefinition(def)
      onLoad(nodes, edges, name)
      onClose()
    } catch {
      // leave picker open so the user can try again
    } finally {
      setLoadingId(null)
    }
  }

  return (
    <div
      ref={ref}
      style={{
        position: 'absolute', top: 48, right: 0, zIndex: 50,
        background: 'var(--tb-surface)', border: '1px solid var(--tb-border-2)',
        borderRadius: 10, minWidth: 260, boxShadow: '0 8px 24px rgba(0,0,0,0.4)',
        display: 'flex', flexDirection: 'column', overflow: 'hidden',
      }}
    >
      <div style={{ padding: '10px 14px 6px', borderBottom: '1px solid var(--tb-border)', fontSize: 11, fontWeight: 600, color: 'var(--tb-text-dim)', textTransform: 'uppercase', letterSpacing: '0.06em' }}>
        Saved Strategies
      </div>
      {loading ? (
        <div style={{ padding: '14px', fontSize: 12, color: 'var(--tb-text-dim)' }}>Loading…</div>
      ) : strategies.length === 0 ? (
        <div style={{ padding: '14px', fontSize: 12, color: 'var(--tb-text-dim)' }}>No saved strategies yet.</div>
      ) : (
        <div style={{ maxHeight: 320, overflowY: 'auto' }}>
          {strategies.map((s) => (
            <button
              key={s.id}
              onClick={() => handleSelect(s)}
              disabled={loadingId === s.id}
              style={{
                display: 'block', width: '100%', textAlign: 'left',
                padding: '9px 14px', background: 'transparent', border: 'none',
                borderBottom: '1px solid var(--tb-border)', cursor: 'pointer',
                fontSize: 13, color: 'var(--tb-text)', fontFamily: 'inherit',
                opacity: loadingId === s.id ? 0.5 : 1,
              }}
              onMouseEnter={e => (e.currentTarget.style.background = 'var(--tb-background)')}
              onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
            >
              <span style={{ fontWeight: 600 }}>{s.strategy_id}</span>
              <span style={{ display: 'block', fontSize: 10, color: 'var(--tb-text-dim)', marginTop: 2, fontFamily: 'monospace' }}>{s.id.slice(0, 8)}…</span>
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

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
  const [nodes, setNodes, onNodesChange] = useNodesState<Node>([])
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([])
  const [name, setName] = useState('')
  const [editingId, setEditingId] = useState<string | null>(null)
  const [savedLabel, setSavedLabel] = useState('')
  const [menu, setMenu] = useState<NodeMenuState | null>(null)
  const [scannerMode, setScannerMode] = useState(false)
  const [saving, setSaving] = useState(false)
  const [showLoader, setShowLoader] = useState(false)
  const { screenToFlowPosition, fitView } = useReactFlow()

  // On mount, load the most recently saved strategy, falling back to the
  // default EMA Crossever template if none exist.
  useEffect(() => {
    async function loadInitial() {
      try {
        const listRes = await strategiesApi.list()
        const strategies = (listRes.data as { strategies: { id: string }[] }).strategies ?? []
        const targetId = strategies.length > 0 ? strategies[0].id : DEFAULT_STRATEGY_ID
        const r = await strategiesApi.get(targetId)
        const def = (r.data as { definition: Parameters<typeof fromDefinition>[0] }).definition
        const { nodes: n, edges: e, name: nm } = fromDefinition(def)
        setNodes(n)
        setEdges(e)
        setName(nm)
        setTimeout(() => fitView({ padding: 0.15 }), 50)
      } catch {
        setName('My Strategy')
      }
    }
    loadInitial()
  }, []) // eslint-disable-line react-hooks/exhaustive-deps

  const handleLoad = useCallback((newNodes: Node[], newEdges: Edge[], newName: string) => {
    setNodes(newNodes)
    setEdges(newEdges)
    setName(newName)
    setEditingId(null)
    // Let React settle before fitting the new graph into view.
    setTimeout(() => fitView({ padding: 0.15 }), 50)
  }, [setNodes, setEdges, fitView])

  // Live validity — scanner mode only requires conditions; execution mode needs
  // a full action/size/exit chain.  Validation is client-side; the Rust validator
  // runs again on save and is the source of truth.
  const valid = useMemo(() => {
    const active = activeGraph(nodes, edges)
    if (scannerMode) {
      const { indicators, allOf, anyOf, errors: errs } = compileScanner(active.nodes, active.edges, name)
      if (errs.length > 0) return false
      const { definition, errors: convErrs } = scannerToDefinition(name, indicators, allOf, anyOf)
      return !!definition && convErrs.length === 0
    }
    const { spec, errors: errs } = compile(active.nodes, active.edges, name)
    if (errs.length > 0 || !spec) return false
    const { definition, errors: convErrs } = ruleSpecToDefinition(spec)
    return !!definition && convErrs.length === 0
  }, [nodes, edges, name, scannerMode])

  async function handleSave() {
    const active = activeGraph(nodes, edges)
    let definition: ReturnType<typeof ruleSpecToDefinition>['definition']
    let warnings: string[] = []

    if (scannerMode) {
      const { indicators, allOf, anyOf, errors: errs } = compileScanner(active.nodes, active.edges, name)
      if (errs.length > 0) return
      const result = scannerToDefinition(name, indicators, allOf, anyOf)
      if (!result.definition || result.errors.length > 0) {
        setSavedLabel(result.errors[0] ?? 'Could not convert scanner strategy')
        setTimeout(() => setSavedLabel(''), 5000)
        return
      }
      definition = result.definition
      warnings = result.warnings
    } else {
      const { spec, errors: errs } = compile(active.nodes, active.edges, name)
      if (!spec || errs.length > 0) return
      const result = ruleSpecToDefinition(spec)
      if (!result.definition || result.errors.length > 0) {
        setSavedLabel(result.errors[0] ?? 'Could not convert strategy')
        setTimeout(() => setSavedLabel(''), 5000)
        return
      }
      definition = result.definition
      warnings = result.warnings
    }
    if (!definition) return
    setSaving(true)
    try {
      // Persist the canonical definition to the Rust strategy store so it is
      // available to the runtime, the MCP server, and the backtest picker
      // (one canonical surface — ADR-0010).
      const r = await api.post<{ id: string; strategy_id: string }>(
        '/api/strategies',
        definition,
      )
      setEditingId(r.data.id)
      const note = warnings.length > 0 ? ` (${warnings.length} note${warnings.length > 1 ? 's' : ''})` : ''
      setSavedLabel(`"${definition.strategy_id}" saved${note}`)
      setTimeout(() => setSavedLabel(''), 4000)
    } catch (e) {
      const msg =
        (e as { response?: { data?: { errors?: Array<{ message?: string }> } } })?.response?.data
          ?.errors?.[0]?.message ?? 'Save failed'
      setSavedLabel(msg)
      setTimeout(() => setSavedLabel(''), 5000)
    } finally { setSaving(false) }
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
        position: 'relative',
      }}>
        <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--tb-text)' }}>Strategy Builder</span>
        {/* Execution / Scanner mode toggle */}
        <div style={{
          display: 'flex', alignItems: 'center', borderRadius: 7,
          border: '1px solid var(--tb-border-2)', overflow: 'hidden', fontSize: 11, fontFamily: 'inherit',
        }}>
          {(['Execution', 'Scanner'] as const).map(mode => {
            const active = (mode === 'Scanner') === scannerMode
            return (
              <button
                key={mode}
                onClick={() => setScannerMode(mode === 'Scanner')}
                style={{
                  padding: '4px 10px', border: 'none', cursor: 'pointer', fontFamily: 'inherit', fontSize: 11,
                  background: active ? 'var(--tb-text-dim)' : 'transparent',
                  color: active ? 'var(--tb-background)' : 'var(--tb-text-dim)',
                  fontWeight: active ? 600 : 400,
                  transition: 'all 0.15s',
                }}
              >
                {mode}
              </button>
            )
          })}
        </div>
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
        {/* Load saved strategy */}
        <div style={{ position: 'relative' }}>
          <button
            onClick={() => setShowLoader(v => !v)}
            style={{
              background: 'transparent', border: '1px solid var(--tb-border-2)', borderRadius: 7,
              padding: '5px 12px', color: 'var(--tb-text-dim)', fontSize: 12, cursor: 'pointer', fontFamily: 'inherit',
            }}
            onMouseEnter={e => (e.currentTarget.style.borderColor = 'var(--tb-text-dim)')}
            onMouseLeave={e => (e.currentTarget.style.borderColor = 'var(--tb-border-2)')}
          >
            Load ▾
          </button>
          {showLoader && (
            <LoadPicker onLoad={handleLoad} onClose={() => setShowLoader(false)} />
          )}
        </div>
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
