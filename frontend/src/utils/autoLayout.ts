import type { Edge, Node } from '@xyflow/react'
import { FAMILIES, TYPE_FAMILY } from '@/nodes'

/* =============================================================================
   Spec §3.21 — auto-layout.

   "The canvas auto-layout MUST place nodes so every edge flows left→right.
    A backward edge produces a loop that reads as a bug.
    Stages: data → indicators → signals → logic → intent → risk."

   Placement is by stage column, then by longest-path depth within the graph so
   a chain of indicators does not stack on top of itself. Rows are packed per
   column and centred on the tallest column.
   ============================================================================= */

const COL_W = 236
const ROW_H = 168
const ORIGIN_X = 60
const ORIGIN_Y = 48

function stageOf(node: Node): number {
  const family = TYPE_FAMILY[node.type ?? ''] ?? 'logic'
  return FAMILIES[family].stage
}

export function autoLayout(nodes: Node[], edges: Edge[]): Node[] {
  if (nodes.length === 0) return nodes

  // Longest path from any root, so dependent nodes always sit to the right of
  // everything that feeds them — this is what guarantees no backward edge.
  const depth = new Map<string, number>()
  const incoming = new Map<string, string[]>()
  for (const n of nodes) incoming.set(n.id, [])
  for (const e of edges) incoming.get(e.target)?.push(e.source)

  function resolve(id: string, seen: Set<string>): number {
    if (depth.has(id)) return depth.get(id)!
    if (seen.has(id)) return 0 // cycle guard — validation reports it separately
    seen.add(id)
    const parents = incoming.get(id) ?? []
    const d = parents.length === 0 ? 0 : Math.max(...parents.map((p) => resolve(p, seen) + 1))
    depth.set(id, d)
    return d
  }
  for (const n of nodes) resolve(n.id, new Set())

  // A node's column is the later of its family stage and its graph depth, so an
  // indicator fed by another indicator still moves right.
  const column = new Map<string, number>()
  for (const n of nodes) column.set(n.id, Math.max(stageOf(n), depth.get(n.id) ?? 0))

  const byColumn = new Map<number, Node[]>()
  for (const n of nodes) {
    const c = column.get(n.id)!
    const arr = byColumn.get(c) ?? []
    arr.push(n)
    byColumn.set(c, arr)
  }

  const tallest = Math.max(...[...byColumn.values()].map((a) => a.length))

  const placed: Node[] = []
  for (const [c, group] of [...byColumn.entries()].sort((a, b) => a[0] - b[0])) {
    // Keep a stable order inside a column so layout is deterministic.
    group.sort((a, b) => (a.position?.y ?? 0) - (b.position?.y ?? 0) || a.id.localeCompare(b.id))
    const offset = ((tallest - group.length) * ROW_H) / 2
    group.forEach((n, i) => {
      placed.push({
        ...n,
        position: { x: ORIGIN_X + c * COL_W, y: ORIGIN_Y + offset + i * ROW_H },
      })
    })
  }

  return placed
}

/** True when any edge runs right-to-left after layout — surfaced by validation. */
export function hasBackwardEdge(nodes: Node[], edges: Edge[]): boolean {
  const x = new Map(nodes.map((n) => [n.id, n.position?.x ?? 0]))
  return edges.some((e) => (x.get(e.source) ?? 0) > (x.get(e.target) ?? 0))
}

/** Depth-first cycle detection — a strategy graph must be acyclic. */
export function findCycle(nodes: Node[], edges: Edge[]): boolean {
  const out = new Map<string, string[]>()
  for (const n of nodes) out.set(n.id, [])
  for (const e of edges) out.get(e.source)?.push(e.target)

  const state = new Map<string, 0 | 1 | 2>()
  function visit(id: string): boolean {
    const s = state.get(id) ?? 0
    if (s === 1) return true
    if (s === 2) return false
    state.set(id, 1)
    for (const next of out.get(id) ?? []) if (visit(next)) return true
    state.set(id, 2)
    return false
  }
  return nodes.some((n) => visit(n.id))
}
