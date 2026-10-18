// The ids a run's tools returned, read off the stream (AGENT-012 §3.5, §6.3;
// task S-6.9).
//
// # Why this exists
//
// The Evidence tab listed `WorkingState.evidence`, and the working state comes
// from `GET /api/agent/runs/{id}/state` — which nothing mounts. So the tab was
// empty on every run this platform has ever had. The resolver had a second
// source, the tool cards on the stream, and it read them **one level too
// high**: a stored `tool_result` is
//
//     content_json = { name, is_error, content: <the ToolResult> }
//
// (`local_driver.rs` and `driver.rs` both write that shape), and the ToolResult
// is `{ summary, ids?: { key: id }, n?, se?, detail?, handle? }`
// (`mcp-server/src/spec.rs::to_value`). The resolver looked for `detail.ids` and
// `detail.handle` — the outer object — where neither ever is. Every id a tool
// returned therefore rendered as "not found", struck through, which is the
// fabrication mark, on ids the platform produced itself.
//
// The stored result *is* what an id resolves to: the summary, the counts, the
// keys it came back under, and the step and tool that produced it. Reading it
// here needs no new route, and it cannot disagree with the stream beside it
// because it is a fold of the same array.
import type { DeskCard, ProvenanceMark, WorkingState } from '@/api/desk'

/** One id a tool returned, with the stored result it resolves to. */
export interface StreamEvidence {
  id: string
  /** The key the tool returned it under (`backtest_id`), or `handle`. */
  key: string
  /** A grouping word: the key without `_id`, else the id's own prefix. */
  type: string
  tool: string
  step: number
  seq: number
  at: string
  summary: string | null
  n: number | null
  se: number | null
  /** Every id the same result returned, so the viewer can show the siblings. */
  siblings: Array<{ key: string; id: string }>
  /** The result's `detail`, when it carried one rather than a handle. */
  detail: unknown
  handle: string | null
  isError: boolean
  mark: ProvenanceMark | null
}

function record(v: unknown): Record<string, unknown> | null {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null
}

/**
 * The ToolResult inside a stored `tool_result`.
 *
 * The cloud driver has written `content` as a JSON *string* on some rows and as
 * an object on others; both are read. Anything that does not parse to an object
 * is not a ToolResult and yields nothing — a free-text tool reply has no ids to
 * resolve, and inventing some from its prose would be the resolver asserting
 * what the platform produced.
 */
export function storedResult(detail: Record<string, unknown> | null | undefined): Record<string, unknown> | null {
  if (!detail) return null
  const inner = detail.content
  if (typeof inner === 'string') {
    try {
      return record(JSON.parse(inner))
    } catch {
      return null
    }
  }
  return record(inner)
}

function typeOf(key: string, id: string): string {
  if (key !== 'handle' && key.endsWith('_id')) return key.slice(0, -3)
  if (key === 'handle') return 'handle'
  const prefix = id.split(/[_-]/)[0]
  return prefix && prefix !== id ? prefix : key
}

function num(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) ? v : null
}

/**
 * Every id the run's tools returned, first producer wins.
 *
 * "First producer" because the harness's own `results_by_id` keeps the first
 * result per id (`or_insert_with`), and the panel should open the result the
 * agent saw, not a later one that happened to mention the same id.
 */
export function streamEvidence(cards: DeskCard[]): StreamEvidence[] {
  const out = new Map<string, StreamEvidence>()
  for (const c of cards) {
    if (c.kind !== 'tool' || !c.detail) continue
    const result = storedResult(c.detail)
    if (!result) continue
    const tool = typeof c.detail.name === 'string' ? c.detail.name : c.headline
    const ids = record(result.ids)
    const siblings: Array<{ key: string; id: string }> = []
    if (ids) {
      for (const [key, v] of Object.entries(ids)) {
        if (typeof v === 'string' && v.length > 0) siblings.push({ key, id: v })
      }
    }
    const handle = typeof result.handle === 'string' && result.handle.length > 0 ? result.handle : null
    if (handle) siblings.push({ key: 'handle', id: handle })
    for (const s of siblings) {
      if (out.has(s.id)) continue
      out.set(s.id, {
        id: s.id,
        key: s.key,
        type: typeOf(s.key, s.id),
        tool,
        step: c.step,
        seq: c.seq,
        at: c.at,
        summary: typeof result.summary === 'string' && result.summary.length > 0 ? result.summary : null,
        n: num(result.n),
        se: num(result.se),
        siblings,
        detail: result.detail ?? null,
        handle,
        isError: c.detail.is_error === true,
        mark: c.mark ?? null,
      })
    }
  }
  return [...out.values()]
}

/**
 * One id the run produced.
 *
 * **There is deliberately no `view`.** The field was declared here as "rendered
 * in place when the id is opened", the tab kept an `openId` and toggled it on
 * every chip click, and the only caller never set it on any entry, so the click
 * opened an empty node. What an id opens now is not a caller-supplied node but
 * the **stored tool result** it came out of (`result`), read off the stream by
 * `streamEvidence` — data, rendered by this tab, never a view handed in.
 *
 * An entry with no `result` (one the working state lists but no stored result
 * on the stream returned) opens nothing here, and its click is only reported
 * upward. Asserted by `an_id_that_opens_nothing_does_not_pretend_to_open`.
 */
export interface EvidenceEntry {
  id: string
  type: string
  summary: string
  /** The stored ToolResult this id came out of, when the stream carries it. */
  result?: StreamEvidence
}

/**
 * Merges the two sources of ids into the tab's list.
 *
 * The stream's tool results come first and win: they are the stored result,
 * where the working state only knows a tool name and a step. A working-state id
 * no stored result returned is kept — without a viewer — rather than dropped.
 */
export function evidenceEntries(
  state: WorkingState | null,
  stream: StreamEvidence[],
): EvidenceEntry[] {
  const out = new Map<string, EvidenceEntry>()
  for (const e of stream) {
    out.set(e.id, {
      id: e.id,
      type: e.type,
      summary: `${e.key} from ${e.tool} at step ${e.step}`,
      result: e,
    })
  }
  for (const e of state?.evidence ?? []) {
    if (out.has(e.id)) continue
    out.set(e.id, { id: e.id, type: e.id.split('_')[0] ?? 'id', summary: `produced by ${e.tool} at step ${e.step}` })
  }
  return [...out.values()]
}

