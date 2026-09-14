import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { addEdge, Background, BackgroundVariant, ReactFlow, ReactFlowProvider, useEdgesState, useNodesState, useReactFlow, type Connection, type Edge, type Node } from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { AlertTriangle, Check, Copy, FolderOpen, Maximize2, Minus, Play, Plus, Power, Save, Trash2, Wand2 } from 'lucide-react'
import { api, strategiesApi } from '@/lib/api'
import { useModeStore } from '@/store/mode'
import { useToast } from '@/hooks/useToast'
import { ActionNode, AIInferenceNode, ConditionNode, ExitNode, IndicatorNode, LogicNode, MarketDataNode, SizeNode } from '@/nodes'
import { compile, compileScanner, collectDataInputs } from '@/utils/compiler'
import { ruleSpecToDefinition, scannerToDefinition } from '@/utils/toDefinition'
import { fromDefinition } from '@/utils/fromDefinition'
import { autoLayout, findCycle, hasBackwardEdge } from '@/utils/autoLayout'
import { BlockPalette } from '@/components/strategy/BlockPalette'
import { Inspector } from '@/components/strategy/Inspector'
import { ValidationConsole, type ValidationMessage } from '@/components/strategy/ValidationConsole'
import { Button, IconButton } from '@/components/primitives/Button'
import { Badge } from '@/components/primitives/Badge'
import { Segmented } from '@/components/primitives/Segmented'
import { Input } from '@/components/primitives/Field'
import { MenuItem, MenuLabel, Popover, Tooltip } from '@/components/primitives/Overlay'
import { ConfirmDialog } from '@/components/primitives/Modal'
import { Label } from '@/components/primitives/Num'
import { relativeTime } from '@/lib/format'

/* =============================================================================
   Spec §4.3 — the strategy builder.

   strategy bar 52  [name ✎] [Draft] version/edited ⟶ [✓ n valid][n warnings] |
                    Validate · Run backtest · Save · Deploy to paper
   ┌───────────┬──────────────────────────────┬─────────────────┐
   │ BLOCKS    │ CANVAS (dot grid)            │ INSPECTOR       │
   │ 268       │ data → indicator → signal →  │ properties      │
   │ search    │ logic → intent → risk        │ connections     │
   │ 7 families│  ┌ zoom ┐   ┌ VALIDATION ┐   │ backtest preview│
   └───────────┴──────────────────────────────┴ risk guardrails ┘

   Below 1024px the canvas is read-only with an explanatory notice.
   ============================================================================= */

const nodeTypes = {
  market_data: MarketDataNode,
  indicator: IndicatorNode,
  condition: ConditionNode,
  ai_inference: AIInferenceNode,
  logic: LogicNode,
  action: ActionNode,
  size: SizeNode,
  exit: ExitNode,
}

let _seq = 200
const uid = () => `n-${++_seq}`

interface SavedStrategy {
  id: string
  strategy_id: string
}

type Mode = 'execution' | 'scanner'

function Canvas() {
  const toast = useToast()
  const { mode: tradingMode } = useModeStore()
  const { screenToFlowPosition, fitView, zoomIn, zoomOut, getZoom } = useReactFlow()

  const [nodes, setNodes, onNodesChange] = useNodesState<Node>([])
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([])
  const [name, setName] = useState('My strategy')
  const [strategyMode, setStrategyMode] = useState<Mode>('execution')
  const [editingId, setEditingId] = useState<string | null>(null)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [lastSavedAt, setLastSavedAt] = useState<number | null>(null)
  const [deployOpen, setDeployOpen] = useState(false)
  const [zoom, setZoom] = useState(1)
  const wrapRef = useRef<HTMLDivElement>(null)

  /* --- continuous validation (spec §4.3) ---------------------------------- */
  const activeGraph = useCallback(() => {
    const disabled = new Set(nodes.filter((n) => (n.data as { disabled?: boolean }).disabled).map((n) => n.id))
    return {
      nodes: nodes.filter((n) => !disabled.has(n.id)),
      edges: edges.filter((e) => !disabled.has(e.source) && !disabled.has(e.target)),
    }
  }, [nodes, edges])

  const validation = useMemo(() => {
    const g = activeGraph()
    const messages: ValidationMessage[] = []
    // Validation is continuous: the run timestamp IS the moment this memo
    // recomputed, so it needs no separate state and no effect to set it.
    const at = Date.now()
    const push = (level: ValidationMessage['level'], text: string) =>
      messages.push({ id: `${level}-${messages.length}-${text.slice(0, 24)}`, level, text, at })

    if (g.nodes.length === 0) {
      push('info', 'Empty canvas. Drag a block from the left to begin.')
      return { messages, ok: false, errorCount: 0, warnCount: 0, definition: null as unknown, at }
    }

    if (findCycle(g.nodes, g.edges)) push('error', 'The graph contains a cycle. A strategy must flow one way.')
    else push('ok', `Graph is acyclic. ${g.nodes.length} blocks resolve in order.`)

    if (hasBackwardEdge(g.nodes, g.edges)) {
      push('warn', 'An edge flows right to left. Run Tidy to lay the graph out in stage order.')
    }

    const dataInputs = collectDataInputs(g.nodes)
    if (dataInputs.length === 0) {
      push('info', 'No market-data block. The strategy will bind to whatever instrument it is deployed on.')
    }

    let definition: unknown = null
    if (strategyMode === 'scanner') {
      const c = compileScanner(g.nodes, g.edges, name)
      c.errors.forEach((e) => push('error', e))
      c.warnings.forEach((w) => push('warn', w))
      if (c.errors.length === 0) {
        const d = scannerToDefinition(name, c.indicators, c.allOf, c.anyOf, 'crypto_spot_cex', dataInputs)
        d.errors.forEach((e) => push('error', e))
        d.warnings.forEach((w) => push('warn', w))
        if (d.errors.length === 0) definition = d.definition
      }
    } else {
      const c = compile(g.nodes, g.edges, name)
      c.errors.forEach((e) => push('error', e))
      c.warnings.forEach((w) => push('warn', w))
      if (c.spec) {
        const d = ruleSpecToDefinition(c.spec, 'crypto_spot_cex', dataInputs)
        d.errors.forEach((e) => push('error', e))
        d.warnings.forEach((w) => push('warn', w))
        if (d.errors.length === 0) definition = d.definition
      }
    }

    const errorCount = messages.filter((m) => m.level === 'error').length
    const warnCount = messages.filter((m) => m.level === 'warn').length
    return { messages, ok: errorCount === 0 && definition !== null, errorCount, warnCount, definition, at }
  }, [activeGraph, name, strategyMode])

  /* --- load saved strategies ---------------------------------------------- */
  const [saved, setSaved] = useState<SavedStrategy[]>([])
  useEffect(() => {
    void strategiesApi
      .list()
      .then((r) => {
        const list = (r.data as { strategies?: SavedStrategy[] }).strategies ?? []
        setSaved(list)
        if (list.length > 0) void load(list[0].id)
      })
      .catch(() => setSaved([]))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  async function load(id: string) {
    try {
      const r = await strategiesApi.get(id)
      const def = (r.data as { definition: Parameters<typeof fromDefinition>[0] }).definition
      const g = fromDefinition(def)
      setNodes(autoLayout(g.nodes, g.edges))
      setEdges(g.edges)
      setName(g.name)
      setEditingId(id)
      setSelectedId(null)
      window.setTimeout(() => fitView({ padding: 0.18, duration: 220 }), 40)
    } catch {
      toast({ title: 'Could not open that strategy', variant: 'error' })
    }
  }

  /* --- canvas interactions ------------------------------------------------ */
  const onConnect = useCallback(
    (c: Connection) => setEdges((eds) => addEdge({ ...c, id: `e-${Date.now()}` }, eds)),
    [setEdges],
  )

  const onDrop = useCallback(
    (event: React.DragEvent) => {
      event.preventDefault()
      const raw = event.dataTransfer.getData('application/reactflow')
      if (!raw) return
      const { type, data } = JSON.parse(raw) as { type: string; data: Record<string, unknown> }
      const position = screenToFlowPosition({ x: event.clientX, y: event.clientY })
      const id = uid()
      setNodes((nds) => [...nds, { id, type, position, data: { ...data } }])
      setSelectedId(id)
    },
    [screenToFlowPosition, setNodes],
  )

  const patchNode = useCallback(
    (id: string, patch: Record<string, unknown>) => {
      setNodes((nds) => nds.map((n) => (n.id === id ? { ...n, data: { ...n.data, ...patch } } : n)))
    },
    [setNodes],
  )

  const selected = useMemo(() => nodes.find((n) => n.id === selectedId) ?? null, [nodes, selectedId])

  function tidy() {
    setNodes((nds) => autoLayout(nds, edges))
    window.setTimeout(() => fitView({ padding: 0.18, duration: 220 }), 30)
  }

  function duplicateSelected() {
    if (!selected) return
    const id = uid()
    setNodes((nds) => [
      ...nds,
      { ...selected, id, position: { x: selected.position.x + 48, y: selected.position.y + 48 }, selected: false },
    ])
    setSelectedId(id)
  }

  function deleteSelected() {
    if (!selectedId) return
    setNodes((nds) => nds.filter((n) => n.id !== selectedId))
    setEdges((eds) => eds.filter((e) => e.source !== selectedId && e.target !== selectedId))
    setSelectedId(null)
  }

  /* --- keyboard map (spec §5.6) ------------------------------------------- */
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const el = e.target as HTMLElement | null
      if (el?.tagName === 'INPUT' || el?.tagName === 'TEXTAREA' || el?.isContentEditable) return
      const meta = e.metaKey || e.ctrlKey
      if (meta && e.key === '0') { e.preventDefault(); fitView({ padding: 0.18, duration: 220 }) }
      else if (meta && (e.key === '=' || e.key === '+')) { e.preventDefault(); zoomIn({ duration: 120 }) }
      else if (meta && e.key === '-') { e.preventDefault(); zoomOut({ duration: 120 }) }
      else if (meta && e.key.toLowerCase() === 'd') { e.preventDefault(); duplicateSelected() }
      else if (e.key === 'Delete' || e.key === 'Backspace') { deleteSelected() }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId, selected, fitView, zoomIn, zoomOut])

  /* --- save / deploy ------------------------------------------------------- */
  async function save() {
    if (!validation.definition) return
    setSaving(true)
    try {
      const r = await api.post('/api/strategies', validation.definition)
      const id = (r.data as { id: string }).id
      setEditingId(id)
      setLastSavedAt(Date.now())
      toast({ title: 'Strategy saved', description: `${name} is stored and ready to backtest.` })
      const list = await strategiesApi.list()
      setSaved((list.data as { strategies?: SavedStrategy[] }).strategies ?? [])
    } catch (e) {
      const err = e as { response?: { data?: { errors?: { message: string }[]; message?: string } } }
      toast({
        title: 'Save refused',
        description: err.response?.data?.errors?.[0]?.message ?? err.response?.data?.message ?? 'The validator rejected this strategy.',
        variant: 'error',
      })
    } finally {
      setSaving(false)
    }
  }

  const canvasZoom = Math.round(zoom * 100)

  return (
    <>
      {/* ── STRATEGY BAR ──────────────────────────────────────────────────── */}
      <div className="pagehead on-surface">
        <Input
          wrapperClassName="namefield"
          aria-label="Strategy name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          style={{ fontWeight: 'var(--w-semibold)' }}
        />
        <Badge tone={editingId ? 'neutral' : 'warn'}>{editingId ? 'Saved' : 'Draft'}</Badge>
        <Label>
          {lastSavedAt ? `Saved ${relativeTime(lastSavedAt)}` : 'Never saved'}
          {' · '}
          {strategyMode === 'scanner' ? 'Discovery' : 'Execution'}
        </Label>

        <div className="spacer" />

        <Segmented
          ariaLabel="Strategy kind"
          value={strategyMode}
          onChange={setStrategyMode}
          options={[
            { value: 'execution', label: 'Execution', title: 'Produces orders: conditions, action, size, exits.' },
            { value: 'scanner', label: 'Discovery', title: 'Produces signals only: conditions, no action leg.' },
          ]}
        />

        <div
          className="row"
          style={{ gap: 6, padding: '0 var(--s-3)', borderLeft: '1px solid var(--line-hairline)', borderRight: '1px solid var(--line-hairline)' }}
        >
          {validation.errorCount === 0 ? (
            <Badge tone="pos">
              <Check size={9} aria-hidden />
              {nodes.length} blocks valid
            </Badge>
          ) : (
            <Badge tone="neg">
              <AlertTriangle size={9} aria-hidden />
              {validation.errorCount} error{validation.errorCount === 1 ? '' : 's'}
            </Badge>
          )}
          {validation.warnCount > 0 && <Badge tone="warn">{validation.warnCount} warning{validation.warnCount === 1 ? '' : 's'}</Badge>}
        </div>

        <Popover
          ariaLabel="Open a saved strategy"
          width={260}
          trigger={({ onClick, ref, 'aria-expanded': expanded }) => (
            <button ref={ref} type="button" className="btn sm" aria-expanded={expanded} onClick={onClick}>
              <FolderOpen size={13} aria-hidden />
              Open
            </button>
          )}
        >
          {(close) => (
            <>
              <MenuLabel>Saved strategies</MenuLabel>
              {saved.length === 0 ? (
                <div className="empty" style={{ minHeight: 90 }}>
                  <span className="msg">Nothing saved yet</span>
                </div>
              ) : (
                saved.map((s) => (
                  <MenuItem
                    key={s.id}
                    selected={s.id === editingId}
                    onClick={() => {
                      void load(s.id)
                      close()
                    }}
                  >
                    {s.strategy_id}
                  </MenuItem>
                ))
              )}
            </>
          )}
        </Popover>

        <Button size="sm" icon={<Wand2 size={13} aria-hidden />} onClick={tidy} disabled={nodes.length === 0}>
          Tidy
        </Button>
        <Button size="sm" icon={<Play size={13} aria-hidden />} disabled={!validation.ok}>
          Run backtest
        </Button>
        <Button size="sm" icon={<Save size={13} aria-hidden />} loading={saving} disabled={!validation.ok} onClick={() => void save()}>
          Save
        </Button>
        <Button
          size="sm"
          variant="primary"
          icon={<Power size={13} aria-hidden />}
          disabled={!validation.ok || validation.errorCount > 0}
          onClick={() => setDeployOpen(true)}
        >
          Deploy to {tradingMode === 'LIVE' ? 'live' : 'paper'}
        </Button>
      </div>

      {/* ── BODY ──────────────────────────────────────────────────────────── */}
      <div className="strat-body">
        <BlockPalette />

        <div className="canvas-shell canvas-surface" ref={wrapRef}>
          <div className="mobile-canvas-notice">
            <div className="callout info" style={{ textAlign: 'left' }}>
              <AlertTriangle size={14} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
              <span>
                The strategy canvas is read-only on a narrow screen. Editing a graph needs the space to see
                it — open this on a desktop to make changes. Everything else on this page still works.
              </span>
            </div>
          </div>

          <div className="canvas-interactive" style={{ position: 'absolute', inset: 0 }}>
            <ReactFlow
              nodes={nodes}
              edges={edges}
              nodeTypes={nodeTypes}
              onNodesChange={onNodesChange}
              onEdgesChange={onEdgesChange}
              onConnect={onConnect}
              onDrop={onDrop}
              onDragOver={(e) => {
                e.preventDefault()
                e.dataTransfer.dropEffect = 'move'
              }}
              onNodeClick={(_, n) => setSelectedId(n.id)}
              onPaneClick={() => setSelectedId(null)}
              onMove={() => setZoom(getZoom())}
              deleteKeyCode={null}
              fitView
              fitViewOptions={{ padding: 0.18 }}
              proOptions={{ hideAttribution: true }}
              defaultEdgeOptions={{ type: 'default' }}
            >
              <Background variant={BackgroundVariant.Dots} gap={22} size={1} color="var(--canvas-dot)" />
            </ReactFlow>
          </div>

          <div className="canvas-tools">
            <IconButton label="Zoom in" onClick={() => { zoomIn({ duration: 120 }); setZoom(getZoom()) }}>
              <Plus size={13} aria-hidden />
            </IconButton>
            <IconButton label="Zoom out" onClick={() => { zoomOut({ duration: 120 }); setZoom(getZoom()) }}>
              <Minus size={13} aria-hidden />
            </IconButton>
            <span className="zoomlbl">{canvasZoom}%</span>
            <IconButton label="Fit to view" onClick={() => fitView({ padding: 0.18, duration: 220 })}>
              <Maximize2 size={13} aria-hidden />
            </IconButton>
            {selected && (
              <>
                <Tooltip content="Duplicate the selected block · ⌘D">
                  <IconButton label="Duplicate block" onClick={duplicateSelected}>
                    <Copy size={13} aria-hidden />
                  </IconButton>
                </Tooltip>
                <Tooltip content="Delete the selected block · Del">
                  <IconButton label="Delete block" onClick={deleteSelected}>
                    <Trash2 size={13} aria-hidden />
                  </IconButton>
                </Tooltip>
              </>
            )}
          </div>

          <ValidationConsole messages={validation.messages} lastRun={validation.at} className="validation-console" />
        </div>

        <Inspector
          className="strat-inspector"
          node={selected}
          nodes={nodes}
          edges={edges}
          onPatch={patchNode}
        />
      </div>

      <ConfirmDialog
        open={deployOpen}
        onCancel={() => setDeployOpen(false)}
        onConfirm={() => {
          setDeployOpen(false)
          toast({
            title: `Deployed to ${tradingMode === 'LIVE' ? 'live' : 'paper'}`,
            description: `${name} will run against ${tradingMode === 'LIVE' ? 'real funds' : 'the paper account'}.`,
          })
        }}
        title={tradingMode === 'LIVE' ? 'Deploy this strategy to live trading?' : 'Deploy to the paper account?'}
        confirmLabel={tradingMode === 'LIVE' ? 'Deploy to live' : 'Deploy to paper'}
        typeToConfirm={tradingMode === 'LIVE' ? 'DEPLOY' : undefined}
        consequence={
          tradingMode === 'LIVE' ? (
            <>
              <strong>{name}</strong> will place real orders with real funds, unattended, whenever its
              conditions hold. Account-level risk limits still clamp it, but the orders are real and cannot
              be undone.
            </>
          ) : (
            <>
              <strong>{name}</strong> will trade the paper account unattended. No live funds are at risk.
            </>
          )
        }
        alternative={
          tradingMode === 'LIVE' ? (
            <Button
              size="sm"
              onClick={() => {
                setDeployOpen(false)
                useModeStore.getState().setMode('PAPER')
              }}
            >
              Deploy to paper instead
            </Button>
          ) : undefined
        }
      />
    </>
  )
}

export function StrategyBuilderPage() {
  return (
    <ReactFlowProvider>
      <Canvas />
    </ReactFlowProvider>
  )
}
