// The graph editor's model (Set U U-7.5): typed ports, and the canonical graph
// ↔ canvas mapping. Pure — no React — so the rule "an incompatible wire cannot
// be made" and the round trip are tested on their own.
import type { Arg, Catalogue, GraphNode, OpSpec, StrategyGraph, Ty, ValueType } from '@/api/strategyV3'
import type { NodeFamily } from '@/nodes/NodeShell'

/** A port's or output's type, reduced to what decides whether a wire fits. */
export type Kind =
  | { k: 'any' }
  | { k: 'num' | 'bool' | 'text' | 'cat' | 'probs' | 'dist' | 'vec' | 'record' | 'window' }
  | { k: 'list'; of: 'num' | 'bool' }

export function kindOfTy(t: Ty): Kind {
  if (typeof t === 'object') {
    if ('var' in t) return { k: 'any' }
    if ('cat_of' in t) return { k: 'cat' }
    return { k: 'probs' }
  }
  switch (t) {
    case 'num':
    case 'bool':
    case 'text':
    case 'window':
      return { k: t }
    case 'any_dist':
      return { k: 'dist' }
    case 'any_vec':
      return { k: 'vec' }
    case 'any_record':
    case 'from_contract':
    case 'from_schema':
      return { k: 'record' }
    case 'num_list':
      return { k: 'list', of: 'num' }
    case 'bool_list':
      return { k: 'list', of: 'bool' }
  }
}

export function kindOfValue(v: ValueType): Kind {
  switch (v.type) {
    case 'num':
    case 'bool':
    case 'text':
    case 'cat':
    case 'probs':
    case 'dist':
    case 'vec':
    case 'record':
      return { k: v.type }
    default:
      return { k: 'any' }
  }
}

/** Whether an output of kind `from` may be wired into a port of kind `to`.
 *  A list port takes a wire of its element type. */
export function compatible(from: Kind, to: Kind): boolean {
  if (from.k === 'any' || to.k === 'any') return true
  if (to.k === 'list') return from.k === to.of
  if (from.k === 'list') return false
  return from.k === to.k
}

export const OUT = 'out'

/** The typed outputs a node offers: one for an operator, one per contract
 *  output for a model. */
export function outputsOf(node: GraphNode, cat: Catalogue): { handle: string; kind: Kind }[] {
  if (node.op === 'model') {
    const ref = refLiteral(node.args?.ref)
    const m = cat.models.find((x) => x.reference === ref)
    return m ? m.contract.outputs.map((o) => ({ handle: o.name, kind: kindOfValue(o.value_type) })) : [{ handle: OUT, kind: { k: 'any' } }]
  }
  const spec = cat.operators.find((o) => o.name === node.op)
  return [{ handle: OUT, kind: spec ? kindOfTy(spec.output) : { k: 'any' } }]
}

/** The ports a wire may enter: an operator's non-literal ports, or a model's
 *  scalar contract inputs. */
export function wirePortsOf(node: GraphNode, cat: Catalogue): { port: string; kind: Kind; optional: boolean }[] {
  if (node.op === 'model') {
    const ref = refLiteral(node.args?.ref)
    const m = cat.models.find((x) => x.reference === ref)
    return (m?.contract.inputs ?? [])
      .filter((i) => i.shape === 'scalar' && i.value_type)
      .map((i) => ({ port: i.name, kind: kindOfValue(i.value_type as ValueType), optional: !!i.optional }))
  }
  const spec = cat.operators.find((o) => o.name === node.op)
  return (spec?.inputs ?? []).filter((p) => !p.literal).map((p) => ({ port: p.name, kind: kindOfTy(p.ty), optional: !!p.optional }))
}

/** The literal ports an operator takes, edited as values, not wires. */
export function literalPortsOf(spec: OpSpec | undefined): string[] {
  return (spec?.inputs ?? []).filter((p) => p.literal).map((p) => p.name)
}

function refLiteral(a: Arg | undefined): string | undefined {
  return a && typeof a === 'object' && !Array.isArray(a) && 'str' in a ? a.str : undefined
}

// ── the canvas ───────────────────────────────────────────────────────────────

export interface CanvasNode {
  id: string
  op: string
  /** Every argument that is not a wire from another node: literals, sources
   *  (`close`), parameters (`$fast`), `state.*`, lists and inline nodes. */
  literals: Record<string, Arg>
  position: { x: number; y: number }
}

export interface Wire {
  source: string
  /** `out`, or a model output's field. */
  sourceHandle: string
  target: string
  /** The port it enters. */
  targetHandle: string
}

/** A string argument that names another node of the graph, as `[node, field]`. */
export function wireOf(a: Arg, nodeIds: Set<string>): [string, string] | null {
  if (typeof a !== 'string' || a.startsWith('$')) return null
  const [head, field] = a.split('.', 2)
  if (!nodeIds.has(head)) return null
  return [head, field ?? OUT]
}

/** The graph's nodes as canvas nodes and wires. Positions are kept where the
 *  caller has them and laid out by dependency depth otherwise. */
export function toCanvas(graph: StrategyGraph, positions: Record<string, { x: number; y: number }> = {}): { nodes: CanvasNode[]; wires: Wire[] } {
  const ids = new Set(graph.nodes.map((n) => n.id))
  const wires: Wire[] = []
  const depth: Record<string, number> = {}
  const nodes: CanvasNode[] = []
  const rows: Record<number, number> = {}
  for (const n of graph.nodes) {
    const literals: Record<string, Arg> = {}
    let d = 0
    for (const [port, a] of Object.entries(n.args ?? {})) {
      const w = wireOf(a, ids)
      if (w) {
        wires.push({ source: w[0], sourceHandle: w[1], target: n.id, targetHandle: port })
        d = Math.max(d, (depth[w[0]] ?? 0) + 1)
      } else {
        literals[port] = a
      }
    }
    depth[n.id] = d
    const row = rows[d] ?? 0
    rows[d] = row + 1
    nodes.push({ id: n.id, op: n.op, literals, position: positions[n.id] ?? { x: 40 + d * 260, y: 40 + row * 130 } })
  }
  return { nodes, wires }
}

/** The canvas back into the graph's nodes — wires win over a literal on the
 *  same port. Everything that is not a node is carried through from `base`. */
export function fromCanvas(base: StrategyGraph, nodes: CanvasNode[], wires: Wire[]): StrategyGraph {
  const out: GraphNode[] = nodes.map((n) => {
    const args: Record<string, Arg> = { ...n.literals }
    for (const w of wires.filter((x) => x.target === n.id)) {
      args[w.targetHandle] = w.sourceHandle === OUT ? w.source : `${w.source}.${w.sourceHandle}`
    }
    return Object.keys(args).length ? { id: n.id, op: n.op, args } : { id: n.id, op: n.op }
  })
  return { ...base, nodes: out }
}

/** Whether a wire may be made: both ends exist, the port takes a wire, the
 *  types fit, and the target does not depend on the source already. */
export function canWire(graph: StrategyGraph, cat: Catalogue, w: Wire): { ok: true } | { ok: false; why: string } {
  if (w.source === w.target) return { ok: false, why: 'a node cannot feed itself' }
  const src = graph.nodes.find((n) => n.id === w.source)
  const tgt = graph.nodes.find((n) => n.id === w.target)
  if (!src || !tgt) return { ok: false, why: 'both ends must be nodes' }
  const out = outputsOf(src, cat).find((o) => o.handle === w.sourceHandle)
  const port = wirePortsOf(tgt, cat).find((p) => p.port === w.targetHandle)
  if (!out) return { ok: false, why: `${src.id} has no output ${w.sourceHandle}` }
  if (!port) return { ok: false, why: `${tgt.op}.${w.targetHandle} takes a value, not a wire` }
  if (!compatible(out.kind, port.kind)) {
    return { ok: false, why: `${src.id} gives ${describe(out.kind)}; ${tgt.op}.${w.targetHandle} takes ${describe(port.kind)}` }
  }
  // A wire that closes a loop would make a value depend on itself this tick:
  // refused when the source already depends on the target.
  const ids = new Set(graph.nodes.map((n) => n.id))
  const dependsOn = (from: string, seen = new Set<string>()): boolean => {
    if (from === w.target) return true
    if (seen.has(from)) return false
    seen.add(from)
    const n = graph.nodes.find((x) => x.id === from)
    return Object.values(n?.args ?? {}).some((a) => {
      const r = wireOf(a, ids)
      return r ? dependsOn(r[0], seen) : false
    })
  }
  if (dependsOn(w.source)) return { ok: false, why: 'that wire would make a loop; wrap one edge in prev(x, 1)' }
  return { ok: true }
}

export function describe(k: Kind): string {
  return k.k === 'list' ? `a list of ${k.of}` : k.k === 'any' ? 'anything' : k.k
}

/** The registry's operator families onto the canvas's seven. */
export function nodeFamily(family: string): NodeFamily {
  switch (family) {
    case 'source':
      return 'data'
    case 'compare':
      return 'signal'
    case 'model':
    case 'adapter':
      return 'ai'
    case 'logic':
    case 'branch':
    case 'absence':
      return 'logic'
    default:
      return 'indicator'
  }
}

export function showArg(a: Arg): string {
  if (typeof a === 'string') return a
  if (typeof a === 'number' || typeof a === 'boolean') return String(a)
  if (Array.isArray(a)) return `[${a.map(showArg).join(', ')}]`
  if ('str' in a) return `"${a.str}"`
  return `${a.op}(…)`
}

/** A literal as typed in the inspector: a number, true/false, a quoted string
 *  (a literal), or a reference (`close`, `$fast`, `state.flat`). */
export function parseLiteral(s: string): Arg {
  const t = s.trim()
  if (t === 'true' || t === 'false') return t === 'true'
  if (t !== '' && !Number.isNaN(Number(t))) return Number(t)
  if (t.length >= 2 && t.startsWith('"') && t.endsWith('"')) return { str: t.slice(1, -1) }
  return t
}
