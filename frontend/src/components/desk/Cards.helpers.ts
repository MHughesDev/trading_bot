import type { SelectionLine } from './Cards'

export function str(v: unknown): string | null {
  return typeof v === 'string' && v.length > 0 ? v : null
}

export function num(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) ? v : null
}

export function bool(v: unknown): boolean {
  return v === true
}

/**
 * What a run produced, for the Report card (AGENT-013 §10; S-10.13, S-10.15).
 *
 * A run's cost is on `/desk/usage`; this is the other half — what came back for
 * it. Rendered as counts with their names rather than a score, because the
 * numbers measure different things and a weighted total would let a run with
 * nine questions and no findings read as productive.
 *
 * Absent is not zero. A run recorded before these columns existed has no yield,
 * and printing `0 findings` for it would state a measurement nobody made.
 */
export function yieldLine(detail: Record<string, unknown> | null | undefined): string | null {
  if (!detail) return null
  const parts: string[] = []
  const add = (key: string, one: string, many: string) => {
    const v = num(detail[key])
    if (v !== null && v > 0) parts.push(`${v} ${v === 1 ? one : many}`)
  }
  add('findings_established', 'established finding', 'established findings')
  add('findings_suggestive', 'suggestive finding', 'suggestive findings')
  add('findings_refuted', 'refuted finding', 'refuted findings')
  add('hypotheses_refuted', 'hypothesis refuted', 'hypotheses refuted')
  add('hypotheses_supported', 'hypothesis supported', 'hypotheses supported')
  add('negatives', 'negative result', 'negative results')
  add('questions_opened', 'question opened', 'questions opened')
  add('questions_closed', 'question closed', 'questions closed')
  add('contradictions_surfaced', 'contradiction', 'contradictions')
  add('proposals', 'proposal', 'proposals')
  const trials = num(detail.trials_spent)
  if (parts.length === 0) {
    // Barren is a real and important answer, and it is only sayable when the
    // run actually spent something. With no trial count there is nothing to
    // distinguish "produced nothing" from "was never measured".
    if (trials === null) return null
    return trials > 0
      ? `barren: ${trials} trial(s) spent and nothing the next run can use`
      : 'nothing produced, and no trial spent'
  }
  const cost = trials === null ? '' : ` · ${trials} trial(s)`
  return parts.join(' · ') + cost
}

/**
 * Read a selection note's per-record clauses.
 *
 * The scrubber's question is **"why did the model not know that?"**, and until
 * S-9.19 the note carried two lists of opaque ids — which answers "what was in
 * the prompt" and not that. The clause was computed during selection, rendered
 * into the prompt for the records that won, and thrown away at the boundary for
 * the ones that lost, which are exactly the records this is asked about.
 *
 * Defensive about shape because this reads a payload off the wire: a line
 * without a clause is dropped rather than rendered as an empty row, since an
 * empty row says "no reason" where the truth is "the note is older than this
 * field".
 */
export function selectionLines(detail: Record<string, unknown> | null | undefined): SelectionLine[] {
  const raw = detail?.reasons
  if (!Array.isArray(raw)) return []
  return raw.flatMap((r) => {
    if (typeof r !== 'object' || r === null) return []
    const row = r as Record<string, unknown>
    const id = str(row.id)
    const reason = str(row.reason)
    if (id === null || reason === null) return []
    return [{ id, selected: row.selected === true, reason, score: num(row.score) }]
  })
}

/**
 * The sentence that goes above the table.
 *
 * Leads with what was **left out**, because that is the question. A note where
 * nothing was left out says so explicitly rather than rendering an empty
 * section — the same rule the dossier's mandatory sections follow, for the same
 * reason: an omitted section and an empty one look identical and mean opposite
 * things.
 */
export function selectionSummary(detail: Record<string, unknown> | null | undefined): string {
  const lines = selectionLines(detail)
  const elided = lines.filter((l) => !l.selected).length
  const candidates = num(detail?.candidates)
  const omitted = num(detail?.lines_omitted) ?? 0
  const bound = Array.isArray(detail?.bound_by) ? (detail?.bound_by as unknown[]).filter((b) => typeof b === 'string') : []

  const head =
    elided === 0
      ? candidates === null
        ? 'Nothing was left out of this prompt.'
        : `All ${candidates} candidate record(s) reached this prompt — nothing was left out.`
      : `${elided} record(s) did not reach this prompt${candidates === null ? '' : ` of ${candidates} candidate(s)`}.`
  const cap = omitted > 0 ? ` ${omitted} more line(s) are not shown.` : ''
  const why = bound.length > 0 ? ` Bound by ${bound.join(', ')}.` : ''
  return head + cap + why
}

/**
 * A number without its sample size is not evidence (§19.1, principle 9).
 * Rendering "Sharpe 1.8" with no `n` beside it is the single easiest way for a
 * surface to launder a guess into a measurement, so the absence is printed.
 */
export function statLine(detail: Record<string, unknown> | null | undefined): string | null {
  if (!detail) return null
  const n = num(detail.n)
  const se = num(detail.se)
  if (n === null && se === null) return null
  const parts: string[] = []
  parts.push(n === null ? '(n unknown)' : `n=${n}`)
  if (se !== null) parts.push(`se=${se}`)
  return parts.join(' · ')
}

export function isIrreversibleEnvelope(envelope: string | null | undefined): boolean {
  return IRREVERSIBLE.has(envelope ?? '')
}

/** Envelopes whose actions cannot be undone, so one click must never commit. */
const IRREVERSIBLE = new Set(['SealedHoldout', 'Promotion', 'Tier3Memory'])
