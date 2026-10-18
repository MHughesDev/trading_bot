import type { DeskCard, ProvenanceMark, WorkingState } from '@/api/desk'
import type { IdResolver } from './Shared'
import { streamEvidence } from '@/lib/desk/evidence'

/**
 * The set of ids the platform can actually open.
 *
 * Built from what tools *returned* — the working state's evidence and findings —
 * never from what a card *mentions*. That distinction is the whole point: an id
 * the model wrote into a claim but no tool ever produced is a fabrication, and
 * the chip renders struck-through. Resolving a card's ids from the card itself
 * would make every id resolve and the check would assert nothing.
 *
 * It also carries the **provenance** of the tool result an id came out of
 * (§6.3a). The fence used to stop at the card border: a tool card whose content
 * was fetched was marked, and the ids it produced were then quoted into
 * findings, gates and the dossier's funnel as bare monospace, identical to an id
 * the platform measured for itself. The resolver is the one place that knows
 * which tool produced which id, so it is the place the mark has to travel from.
 */
/**
 * The age to put on the stale badge, or `null` when there is no age to give.
 *
 * The header rendered `` `${stream.staleForSecs ?? 0}s` ``, and `staleForSecs`
 * is measured from a clock that starts at mount — so a stream that dropped
 * before delivering a single frame showed **"0s old"**, which is what a
 * perfectly fresh stream shows. §14 asks a disconnected desk to keep the last
 * known state "marked stale with age"; where there is no last known state there
 * is no age, and the `??` made the two readings identical.
 */
export function staleAge(secs: number | null): string | null {
  if (secs === null || !Number.isFinite(secs)) return null
  return `${Math.max(0, Math.round(secs))}s`
}

export function makeResolver(state: WorkingState | null, cards: DeskCard[]): IdResolver {
  const known = new Map<string, { label: string; kind: string; mark?: ProvenanceMark | null }>()
  for (const e of state?.evidence ?? []) known.set(e.id, { label: `from ${e.tool}, step ${e.step}`, kind: 'evidence' })
  for (const f of state?.findings ?? []) known.set(f.id, { label: f.headline, kind: 'finding' })
  for (const c of cards) {
    if (c.kind !== 'tool') continue
    const produced = (c.detail?.produced_ids ?? c.detail?.ids) as unknown
    if (Array.isArray(produced)) {
      for (const id of produced) {
        if (typeof id === 'string') {
          known.set(id, { label: `returned by ${c.headline}`, kind: 'tool result', mark: c.mark ?? null })
        }
      }
    }
    const handle = c.detail?.handle
    if (typeof handle === 'string') {
      known.set(handle, { label: `full result of ${c.headline}`, kind: 'handle', mark: c.mark ?? null })
    }
  }
  // The stored ToolResult one level down — `content_json.content.ids` and
  // `.handle`, where the drivers actually write them. The two reads above look
  // at the outer object, where neither field has ever been, so without this
  // every id a tool returned rendered struck through as "not found".
  for (const e of streamEvidence(cards)) {
    if (known.has(e.id)) continue
    known.set(e.id, {
      label: e.key === 'handle' ? `full result of ${e.tool}, step ${e.step}` : `${e.key} from ${e.tool}, step ${e.step}`,
      kind: e.key === 'handle' ? 'handle' : 'tool result',
      mark: e.mark,
    })
  }
  return (id) => known.get(id) ?? null
}

const TABS = ['state', 'evidence', 'memory', 'tools', 'cost', 'agenda', 'charter'] as const

/** The right panel's opening tab, from `?tab=` on the current address. */
export function initialTab(search: string = typeof window === 'undefined' ? '' : window.location.search) {
  const t = new URLSearchParams(search).get('tab')
  return ((TABS as readonly string[]).includes(t ?? '') ? t : 'state') as (typeof TABS)[number]
}
