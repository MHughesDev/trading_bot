import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import {
  Background,
  BackgroundVariant,
  ReactFlow,
  ReactFlowProvider,
  type Connection,
  type Edge,
  type EdgeChange,
  type Node,
  type NodeChange,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { Check, FileCode, Plus, Save, Trash2 } from 'lucide-react'
import {
  strategyV3Api,
  type Arg,
  type Catalogue,
  type Finding,
  type GraphNode,
  type StrategyGraph,
} from '@/api/strategyV3'
import { canWire, fromCanvas, literalPortsOf, outputsOf, parseLiteral, showArg, toCanvas, wirePortsOf, type Wire } from '@/components/strategy/v3/graph'
import { GraphOpNode, type GraphOpNodeData } from '@/components/strategy/v3/GraphOpNode'
import { Button } from '@/components/primitives/Button'
import { Badge } from '@/components/primitives/Badge'
import { Input, TextArea } from '@/components/primitives/Field'
import { useToast } from '@/hooks/useToast'

/* =============================================================================
   Set U U-7.5 — the graph editor for tbot.strategy/3 (FEAT-007 §10, ADR-0045).

   It edits the canonical graph. The palette is the catalogue the compiler checks
   against (GET /api/v3/catalogue); ports are typed and the canvas refuses a wire
   that does not fit; rules and the selection sink are forms; the SLv3 text view
   and the canvas are kept the same strategy through POST /api/v3/authoring/build
   — the server owns the language, so the browser never parses a strategy; and
   Save is POST /api/v3/strategies, the call the agent's save_strategy_v3 makes.
   ============================================================================= */

const nodeTypes = { op: GraphOpNode }

const EMPTY_CATALOGUE: Catalogue = { operators: [], features: [], models: [] }

const EMPTY: StrategyGraph = {
  format: 'tbot.strategy/3',
  kind: 'trading',
  name: 'new_strategy',
  asset_class: 'crypto_spot_cex',
  timeframe: '1h',
  nodes: [],
  decide: { rules: [], otherwise: 'hold' },
}

function freshId(graph: StrategyGraph, base: string): string {
  const stem = base.replace(/[^A-Za-z0-9_]/g, '_').replace(/^(\d)/, '_$1') || 'n'
  let i = 1
  while (graph.nodes.some((n) => n.id === `${stem}${i}`)) i += 1
  return `${stem}${i}`
}

type Rule = { name?: string; group?: string; when: Arg; then: Record<string, unknown> }

function Editor() {
  const toast = useToast()
  const cat = useQuery({ queryKey: ['v3-catalogue'], queryFn: strategyV3Api.catalogue, staleTime: 60_000 })
  const [graph, setGraph] = useState<StrategyGraph>(EMPTY)
  const [positions, setPositions] = useState<Record<string, { x: number; y: number }>>({})
  const [selected, setSelected] = useState<string | null>(null)
  const [text, setText] = useState('')
  const [textDirty, setTextDirty] = useState(false)
  const [findings, setFindings] = useState<Finding[]>([])
  const [refusal, setRefusal] = useState<string | null>(null)
  const [strategyId, setStrategyId] = useState('')
  const [search, setSearch] = useState('')
  const [saving, setSaving] = useState(false)
  const syncSeq = useRef(0)

  const catalogue: Catalogue = useMemo(() => cat.data ?? EMPTY_CATALOGUE, [cat.data])

  // Opened on a stored strategy (`?strategy=<id>`): edit it, and save back to it.
  useEffect(() => {
    const id = new URLSearchParams(window.location.search).get('strategy')
    if (!id) return
    strategyV3Api
      .get(id)
      .then((r) => {
        setGraph(r.graph)
        setStrategyId(id)
      })
      .catch(() => toast({ title: `Could not open ${id}`, variant: 'error' }))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Graph → text and findings, debounced. The server prints and validates.
  useEffect(() => {
    if (textDirty) return
    const seq = ++syncSeq.current
    const t = setTimeout(() => {
      strategyV3Api
        .build({ graph })
        .then((r) => {
          if (seq !== syncSeq.current) return
          setText(r.text)
          setFindings(r.findings)
          setRefusal(null)
        })
        .catch((e: Error & { findings?: Finding[] }) => {
          if (seq !== syncSeq.current) return
          setFindings(e.findings ?? [])
          setRefusal(e.message)
        })
    }, 350)
    return () => clearTimeout(t)
  }, [graph, textDirty])

  /** Text → graph: the canvas becomes what the text says. */
  const applyText = useCallback(() => {
    strategyV3Api
      .build({ text })
      .then((r) => {
        setGraph(r.graph)
        setFindings(r.findings)
        setRefusal(null)
        setTextDirty(false)
      })
      .catch((e: Error & { findings?: Finding[] }) => {
        setFindings(e.findings ?? [])
        setRefusal(e.message)
      })
  }, [text])

  const { nodes: cnodes, wires } = useMemo(() => toCanvas(graph, positions), [graph, positions])

  const rfNodes: Node[] = useMemo(
    () =>
      cnodes.map((n) => {
        const g = graph.nodes.find((x) => x.id === n.id) as GraphNode
        const spec = catalogue.operators.find((o) => o.name === n.op)
        const bad = findings.find((f) => f.at === n.id)
        const data: GraphOpNodeData = {
          op: n.op,
          family: spec?.family ?? 'model',
          inputs: wirePortsOf(g, catalogue),
          outputs: outputsOf(g, catalogue),
          literals: n.literals,
          invalid: bad ? `${bad.message} — ${bad.fix}` : undefined,
        }
        return { id: n.id, type: 'op', position: n.position, data, selected: n.id === selected }
      }),
    [cnodes, graph.nodes, catalogue, findings, selected],
  )
  const rfEdges: Edge[] = useMemo(
    () =>
      wires.map((w) => ({
        id: `${w.source}:${w.sourceHandle}->${w.target}:${w.targetHandle}`,
        source: w.source,
        sourceHandle: w.sourceHandle,
        target: w.target,
        targetHandle: w.targetHandle,
      })),
    [wires],
  )

  const commit = useCallback(
    (nextWires: Wire[]) => setGraph((g) => fromCanvas(g, toCanvas(g, positions).nodes, nextWires)),
    [positions],
  )

  const onNodesChange = useCallback((changes: NodeChange[]) => {
    for (const c of changes) {
      if (c.type === 'position' && c.position) {
        const p = c.position
        setPositions((ps) => ({ ...ps, [c.id]: p }))
      }
      if (c.type === 'select') setSelected(c.selected ? c.id : null)
    }
  }, [])

  const onEdgesChange = useCallback(
    (changes: EdgeChange[]) => {
      const removed = new Set(changes.filter((c) => c.type === 'remove').map((c) => c.id))
      if (removed.size) commit(wires.filter((w) => !removed.has(`${w.source}:${w.sourceHandle}->${w.target}:${w.targetHandle}`)))
    },
    [commit, wires],
  )

  const toWire = (c: Connection | Edge): Wire | null =>
    c.source && c.target && c.sourceHandle && c.targetHandle
      ? { source: c.source, sourceHandle: c.sourceHandle, target: c.target, targetHandle: c.targetHandle }
      : null

  // The canvas refuses an incompatible wire before it exists.
  const isValidConnection = useCallback(
    (c: Connection | Edge) => {
      const w = toWire(c)
      return !!w && canWire(graph, catalogue, w).ok
    },
    [graph, catalogue],
  )

  const onConnect = useCallback(
    (c: Connection) => {
      const w = toWire(c)
      if (!w) return
      const r = canWire(graph, catalogue, w)
      if (!r.ok) {
        toast({ title: 'That wire does not fit', description: r.why, variant: 'warning' })
        return
      }
      commit([...wires.filter((x) => !(x.target === w.target && x.targetHandle === w.targetHandle)), w])
    },
    [graph, catalogue, wires, commit, toast],
  )

  const addNode = (op: string, args: Record<string, Arg> = {}, base = op) => {
    setGraph((g) => {
      const id = freshId(g, base)
      setSelected(id)
      return { ...g, nodes: [...g.nodes, Object.keys(args).length ? { id, op, args } : { id, op }] }
    })
  }

  const setLiteral = (nodeId: string, port: string, raw: string) =>
    setGraph((g) => ({
      ...g,
      nodes: g.nodes.map((n) => {
        if (n.id !== nodeId) return n
        const args = { ...(n.args ?? {}) }
        if (raw.trim() === '') delete args[port]
        else args[port] = parseLiteral(raw)
        return Object.keys(args).length ? { ...n, args } : { id: n.id, op: n.op }
      }),
    }))

  const removeNode = (id: string) => {
    setGraph((g) => ({ ...g, nodes: g.nodes.filter((n) => n.id !== id) }))
    setSelected(null)
  }

  const save = async () => {
    if (!strategyId.trim()) {
      toast({ title: 'Name the strategy to save it', description: 'a strategy id such as ema_cross_v1', variant: 'warning' })
      return
    }
    setSaving(true)
    try {
      const r = await strategyV3Api.save(strategyId.trim(), graph)
      toast({ title: 'Saved', description: `revision ${r.revision_hash.slice(0, 12)}`, variant: 'success' })
    } catch (e) {
      const f = (e as { response?: { data?: { findings?: Finding[] } } }).response?.data?.findings
      if (f) setFindings(f)
      toast({ title: 'Not saved', description: f?.[0] ? `${f[0].message} — ${f[0].fix}` : String(e), variant: 'error' })
    } finally {
      setSaving(false)
    }
  }

  const sel = graph.nodes.find((n) => n.id === selected)
  const selSpec = sel ? catalogue.operators.find((o) => o.name === sel.op) : undefined
  const q = search.trim().toLowerCase()
  const ops = catalogue.operators.filter(
    (o) => (graph.kind === 'selection' || !o.cross_sectional) && (!q || o.name.includes(q) || o.use_when.toLowerCase().includes(q)),
  )
  const families = [...new Set(ops.map((o) => o.family))]
  const feats = catalogue.features.filter((f) => !q || f.example.includes(q) || f.family.includes(q)).slice(0, 40)
  const boolIds = graph.nodes.map((n) => n.id)
  const rules = ((graph.decide as { rules?: Rule[] } | undefined)?.rules ?? []) as Rule[]
  const setRules = (rs: Rule[]) => setGraph((g) => ({ ...g, decide: { rules: rs, otherwise: 'hold' } }))
  const select = graph.select as { score?: Arg; where?: Arg; top_n?: number; order?: string } | undefined

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* strategy bar */}
      <div className="flex flex-wrap items-center gap-2 border-b px-3 py-2" style={{ borderColor: 'var(--border)' }}>
        <Input aria-label="Strategy name" value={graph.name} onChange={(e) => setGraph({ ...graph, name: e.target.value })} style={{ width: 200 }} />
        <select aria-label="Kind" value={graph.kind} onChange={(e) => {
          const kind = e.target.value as StrategyGraph['kind']
          setGraph(kind === 'trading'
            ? { ...graph, kind, decide: graph.decide ?? { rules: [], otherwise: 'hold' }, select: undefined, universe: undefined }
            : { ...graph, kind, decide: undefined, universe: graph.universe ?? { kind: 'asset_class', asset_class: graph.asset_class }, select: graph.select ?? { score: graph.nodes[0]?.id ?? '' } })
        }}>
          <option value="trading">trading</option>
          <option value="selection">selection</option>
        </select>
        <Input aria-label="Asset class" value={graph.asset_class} onChange={(e) => setGraph({ ...graph, asset_class: e.target.value })} style={{ width: 160 }} />
        <select aria-label="Timeframe" value={graph.timeframe} onChange={(e) => setGraph({ ...graph, timeframe: e.target.value })}>
          {['1s', '1m', '5m', '15m', '1h', '4h', '1d'].map((t) => <option key={t}>{t}</option>)}
        </select>
        <span className="flex-1" />
        {refusal ? (
          <Badge tone="neg">not a strategy</Badge>
        ) : findings.length ? (
          <Badge tone="warn">{findings.length} to fix</Badge>
        ) : (
          <Badge tone="pos"><Check size={11} aria-hidden /> valid</Badge>
        )}
        <Input aria-label="Strategy id" placeholder="strategy id" value={strategyId} onChange={(e) => setStrategyId(e.target.value)} style={{ width: 170 }} />
        <Button variant="primary" icon={<Save size={13} />} loading={saving} onClick={save}>Save</Button>
      </div>

      <div className="grid min-h-0 flex-1" style={{ gridTemplateColumns: '250px 1fr 340px' }}>
        {/* palette — the catalogue, nothing else */}
        <aside className="min-h-0 overflow-auto border-r p-2" style={{ borderColor: 'var(--border)' }} aria-label="Palette">
          <Input placeholder="Search operators and features" value={search} onChange={(e) => setSearch(e.target.value)} />
          {cat.isError && <p className="mt-2 text-xs" style={{ color: 'var(--fg-neg)' }}>The catalogue did not load.</p>}
          {families.map((f) => (
            <div key={f} className="mt-3">
              <div className="text-[11px] uppercase tracking-wide" style={{ color: 'var(--fg-muted)' }}>{f}</div>
              {ops.filter((o) => o.family === f).map((o) => (
                <button key={o.name} type="button" className="block w-full truncate rounded px-1 py-0.5 text-left text-xs hover:bg-[var(--bg-hover)]" title={o.use_when} onClick={() => addNode(o.name)}>
                  <span className="mono">{o.name}</span>
                </button>
              ))}
            </div>
          ))}
          <div className="mt-3 text-[11px] uppercase tracking-wide" style={{ color: 'var(--fg-muted)' }}>features</div>
          {feats.map((f) => (
            <button key={f.family} type="button" className="block w-full truncate rounded px-1 py-0.5 text-left text-xs hover:bg-[var(--bg-hover)]" title={f.use_when} onClick={() => addNode('feature', { name: { str: f.example } }, f.example)}>
              <span className="mono">{f.example}</span>
            </button>
          ))}
          <div className="mt-3 text-[11px] uppercase tracking-wide" style={{ color: 'var(--fg-muted)' }}>models</div>
          {catalogue.models.length === 0 && <p className="text-xs" style={{ color: 'var(--fg-muted)' }}>No model version has a contract yet.</p>}
          {catalogue.models.map((m) => (
            <button key={m.reference} type="button" className="block w-full truncate rounded px-1 py-0.5 text-left text-xs hover:bg-[var(--bg-hover)]" title={m.kind} onClick={() => addNode('model', { ref: { str: m.reference } }, 'model')}>
              <span className="mono">{m.reference}</span>
            </button>
          ))}
        </aside>

        {/* canvas */}
        <div className="min-h-0" aria-label="Canvas">
          <ReactFlow
            nodes={rfNodes}
            edges={rfEdges}
            nodeTypes={nodeTypes}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            isValidConnection={isValidConnection}
            fitView
            deleteKeyCode={null}
          >
            <Background variant={BackgroundVariant.Dots} gap={16} size={1} />
          </ReactFlow>
        </div>

        {/* inspector, sink forms, text view, findings */}
        <aside className="min-h-0 overflow-auto border-l p-3" style={{ borderColor: 'var(--border)' }} aria-label="Inspector">
          {sel ? (
            <section>
              <div className="flex items-center gap-2">
                <span className="mono text-sm">{sel.id}</span>
                <Badge tone="outline">{sel.op}</Badge>
                <span className="flex-1" />
                <Button size="xs" variant="ghost" icon={<Trash2 size={12} />} onClick={() => removeNode(sel.id)}>Remove</Button>
              </div>
              {selSpec && <p className="mt-1 text-xs" style={{ color: 'var(--fg-muted)' }}>{selSpec.use_when}</p>}
              {[...new Set([...literalPortsOf(selSpec), ...Object.keys(toCanvas(graph).nodes.find((n) => n.id === sel.id)?.literals ?? {})])].map((port) => {
                const v = sel.args?.[port]
                return (
                  <label key={port} className="mt-2 block text-xs">
                    <span className="mono">{port}</span>
                    <Input
                      defaultValue={v === undefined ? '' : showArg(v)}
                      key={`${sel.id}:${port}:${v === undefined ? '' : showArg(v)}`}
                      onBlur={(e) => setLiteral(sel.id, port, e.target.value)}
                      placeholder='a number, "text", close, $param, state.flat'
                    />
                  </label>
                )
              })}
            </section>
          ) : (
            <p className="text-xs" style={{ color: 'var(--fg-muted)' }}>Select a node to edit its values. Drag from an output to a port to wire it; a wire whose types do not fit is refused.</p>
          )}

          {graph.kind === 'trading' ? (
            <section className="mt-4">
              <div className="flex items-center">
                <span className="text-xs uppercase tracking-wide" style={{ color: 'var(--fg-muted)' }}>Rules</span>
                <span className="flex-1" />
                <Button size="xs" icon={<Plus size={12} />} onClick={() => setRules([...rules, { when: boolIds[0] ?? '', then: { kind: 'hold' } }])}>Rule</Button>
              </div>
              {rules.map((r, i) => (
                <RuleForm key={i} rule={r} ids={boolIds} onChange={(nr) => setRules(rules.map((x, j) => (j === i ? nr : x)))} onRemove={() => setRules(rules.filter((_, j) => j !== i))} />
              ))}
            </section>
          ) : (
            <section className="mt-4 text-xs">
              <span className="uppercase tracking-wide" style={{ color: 'var(--fg-muted)' }}>Selection</span>
              <label className="mt-1 block">score
                <select className="block w-full" value={typeof select?.score === 'string' ? select.score : ''} onChange={(e) => setGraph({ ...graph, select: { ...select, score: e.target.value } })}>
                  {graph.nodes.map((n) => <option key={n.id}>{n.id}</option>)}
                </select>
              </label>
              <label className="mt-1 block">where
                <select className="block w-full" value={typeof select?.where === 'string' ? select.where : ''} onChange={(e) => setGraph({ ...graph, select: { ...select, where: e.target.value || undefined } })}>
                  <option value="">(none)</option>
                  {graph.nodes.map((n) => <option key={n.id}>{n.id}</option>)}
                </select>
              </label>
              <label className="mt-1 block">top
                <Input type="number" value={select?.top_n ?? ''} onChange={(e) => setGraph({ ...graph, select: { ...select, top_n: e.target.value ? Number(e.target.value) : undefined } })} />
              </label>
            </section>
          )}

          <section className="mt-4">
            <div className="flex items-center">
              <span className="text-xs uppercase tracking-wide" style={{ color: 'var(--fg-muted)' }}>SLv3</span>
              <span className="flex-1" />
              <Button size="xs" icon={<FileCode size={12} />} disabled={!textDirty} onClick={applyText}>Apply text</Button>
            </div>
            <TextArea
              aria-label="SLv3 text"
              className="mono mt-1 text-xs"
              rows={14}
              value={text}
              onChange={(e) => {
                setText(e.target.value)
                setTextDirty(true)
              }}
            />
          </section>

          {(refusal || findings.length > 0) && (
            <section className="mt-4" aria-label="Findings">
              {refusal && <p className="text-xs" style={{ color: 'var(--fg-neg)' }}>{refusal}</p>}
              {findings.map((f, i) => (
                <div key={i} className="mt-2 text-xs">
                  <span className="mono">{f.code}</span> <span style={{ color: 'var(--fg-muted)' }}>at {f.at}</span>
                  <div>{f.message}</div>
                  <div style={{ color: 'var(--fg-muted)' }}>{f.fix}</div>
                </div>
              ))}
            </section>
          )}
        </aside>
      </div>
    </div>
  )
}

/** One rule: its condition and a form for the common intents; any other
 *  intent is edited as its JSON. */
function RuleForm({ rule, ids, onChange, onRemove }: { rule: Rule; ids: string[]; onChange: (r: Rule) => void; onRemove: () => void }) {
  const kind = String(rule.then.kind ?? 'hold')
  const size = (rule.then.size ?? { mode: 'units', x: 1 }) as { mode: string; x?: Arg }
  const simple = ['hold', 'close', 'target', 'order'].includes(kind)
  const [json, setJson] = useState(JSON.stringify(rule.then))
  return (
    <div className="mt-2 rounded border p-2 text-xs" style={{ borderColor: 'var(--border)' }}>
      <div className="flex items-center gap-1">
        <span>when</span>
        <select value={typeof rule.when === 'string' ? rule.when : ''} onChange={(e) => onChange({ ...rule, when: e.target.value })}>
          {typeof rule.when !== 'string' && <option value="">(expression)</option>}
          {ids.map((id) => <option key={id}>{id}</option>)}
        </select>
        <span>→</span>
        <select value={kind} onChange={(e) => {
          const k = e.target.value
          onChange({ ...rule, then: k === 'target' ? { kind: k, size: { mode: 'equity_fraction', x: 0.5 } } : k === 'order' ? { kind: k, side: 'buy', size: { mode: 'units', x: 1 } } : { kind: k } })
        }}>
          {['hold', 'close', 'target', 'order', 'bracket', 'protect', 'add', 'reduce', 'cancel'].map((k) => <option key={k}>{k}</option>)}
        </select>
        <span className="flex-1" />
        <button type="button" aria-label="Remove rule" onClick={onRemove}><Trash2 size={11} /></button>
      </div>
      {(kind === 'target' || kind === 'order') && (
        <div className="mt-1 flex items-center gap-1">
          {kind === 'order' && (
            <select value={String(rule.then.side ?? 'buy')} onChange={(e) => onChange({ ...rule, then: { ...rule.then, side: e.target.value } })}>
              <option>buy</option>
              <option>sell</option>
            </select>
          )}
          <select value={size.mode} onChange={(e) => onChange({ ...rule, then: { ...rule.then, size: { mode: e.target.value, x: size.x ?? 1 } } })}>
            {['units', 'notional', 'equity_fraction'].map((m) => <option key={m}>{m}</option>)}
          </select>
          <Input
            aria-label="size"
            defaultValue={size.x === undefined ? '' : showArg(size.x)}
            onBlur={(e) => onChange({ ...rule, then: { ...rule.then, size: { mode: size.mode, x: parseLiteral(e.target.value) } } })}
            style={{ width: 90 }}
          />
        </div>
      )}
      {!simple && (
        <TextArea
          aria-label="intent JSON"
          className="mono mt-1"
          rows={3}
          value={json}
          onChange={(e) => setJson(e.target.value)}
          onBlur={() => {
            try {
              onChange({ ...rule, then: JSON.parse(json) })
            } catch {
              /* left as typed; the findings list names what is wrong once applied */
            }
          }}
        />
      )}
    </div>
  )
}

export function StrategyGraphPage() {
  return (
    <ReactFlowProvider>
      <Editor />
    </ReactFlowProvider>
  )
}
