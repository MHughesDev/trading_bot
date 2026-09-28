import type { ProvenanceMark } from '@/api/desk'
import type { Bucket, MemoryRow, QuestionFields } from './MemoryBrowser'

/**
 * Read a question out of an entry's `extra`.
 *
 * Returns `null` when the entry is not a question, so the generic row stays
 * generic: this is additive per kind rather than a second renderer.
 *
 * `blocking` defaults to **false** on a malformed row. A question that claimed
 * to block on nothing but a missing field would lead the next run's brief, which
 * is the one place a wrong default is expensive.
 */
export function questionFields(row: MemoryRow): QuestionFields | null {
  const e = row.extra
  if (!e || typeof e !== 'object') return null
  if (!('blocking' in e) && !('why_it_matters' in e)) return null
  const status = (e as { status?: unknown }).status
  const closed =
    status && typeof status === 'object' && 'closed' in (status as object)
      ? ((status as { closed: { by?: string; how?: string } }).closed ?? undefined)
      : undefined
  return {
    why_it_matters: typeof e.why_it_matters === 'string' ? e.why_it_matters : undefined,
    blocking: e.blocking === true,
    value: typeof e.value === 'string' ? e.value : undefined,
    closedBy: closed?.by,
    closedHow: closed?.how,
  }
}

/**
 * The provenance mark a memory row can honestly carry (§6.3a).
 *
 * Returns `null` when the row says nothing at all — an absent mark means "not
 * recorded", and a surface that filled it in with a default would be inventing
 * a trust claim on the reader's behalf, which is the failure the fence exists to
 * prevent committed one layer up.
 *
 * A row that records only an `evidence_class` gets `provenance: 'platform'`
 * because `ProvenanceMark` requires the field; nothing is drawn from it, since
 * `ProvenanceBadge` renders a provenance chip only for `untrusted`.
 *
 * `MemoryBrowser`'s row used to pass a literal `null` here, so a memory row
 * could not show a mark even once the store carried one.
 */
export function rowMark(row: MemoryRow): ProvenanceMark | null {
  if (row.mark) return row.mark
  if (row.evidence_class) return { provenance: 'platform', evidence_class: row.evidence_class }
  return null
}

/** The cited count, under either of the two names it has had. */
export function citedCount(row: MemoryRow): number | null {
  if (typeof row.cited === 'number') return row.cited
  if (typeof row.cited_by_chain === 'number') return row.cited_by_chain
  return null
}

/**
 * One declared disagreement, and whichever half of it this bucket holds.
 *
 * `other` is `null` when the id a row names is not in `rows` — a record in
 * another bucket, or one the store did not return. That is a third thing, and it
 * has to be, because it used to be nothing at all: see `conflictPairs`.
 */
export interface ConflictPair {
  /** The row that declares the conflict. */
  row: MemoryRow
  /** The id it says it disagrees with. */
  otherId: string
  /** That row, when this bucket holds it. */
  other: MemoryRow | null
}

/**
 * Pairs every declared conflict exactly once.
 *
 * Both sides are returned together so the renderer cannot show one and rank the
 * other below the fold — which is the acceptance clause (S-6.18).
 *
 * **A dangling counterpart is returned, not dropped.** This function used to do
 * `const o = byId.get(other); if (!o) continue`, so a row declaring
 * `conflicts_with: ['f_99']` where `f_99` is in a different bucket, or was simply
 * not in the response, vanished from the conflicts view along with the conflict
 * it declared — and `Conflicts (0)` then rendered over a bucket that says in its
 * own data that it disagrees with itself, under the sentence "Nothing in this
 * bucket disagrees with anything else". That is the failure shape this file's
 * header is about, committed by the ranking-proof function: an unresolvable
 * reference read as an absence of the thing it refers to. The renderer says which
 * half is missing instead.
 */
export function conflictPairs(rows: MemoryRow[]): ConflictPair[] {
  const byId = new Map(rows.map((r) => [r.id, r]))
  const seen = new Set<string>()
  const out: ConflictPair[] = []
  for (const r of rows) {
    for (const otherId of r.conflicts_with ?? []) {
      // A row that lists itself is a malformed row, not a disagreement.
      if (otherId === r.id) continue
      const key = [r.id, otherId].sort().join('|')
      if (seen.has(key)) continue
      seen.add(key)
      out.push({ row: r, otherId, other: byId.get(otherId) ?? null })
    }
  }
  return out
}

/** A `?bucket=` a digest line put in the URL, or `null` if it named none this page has. */
export function bucketFromParam(raw: string | null): Bucket | null {
  return BUCKETS.some((b) => b.value === raw) ? (raw as Bucket) : null
}

export const BUCKETS: Array<{ value: Bucket; label: string }> = [
  { value: 'findings', label: 'Findings' },
  { value: 'negative', label: 'Negative results' },
  { value: 'profiles', label: 'Instrument profiles' },
  { value: 'questions', label: 'Open questions' },
  { value: 'preferences', label: 'Preferences' },
  { value: 'defects', label: 'Platform defects' },
  { value: 'signals', label: 'Signal registry' },
]
