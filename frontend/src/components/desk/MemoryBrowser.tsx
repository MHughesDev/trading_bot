// The memory browser (AGENT-012 §7; task S-6.18).
//
// The rule that shapes it: **a conflicting pair renders side by side and neither
// is hidden by ranking.** A memory surface that shows only the winner of a
// disagreement is a surface that quietly decides the disagreement, and the
// person never learns there was one.
import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { SearchInput } from '@/components/primitives/Field'
import { Tabs } from '@/components/primitives/Segmented'
import { EmptyState } from '@/components/primitives/States'
import { deskApi, type EvidenceClass, type ProvenanceMark } from '@/api/desk'
import { ProvenanceBadge } from './Shared'

// The tabs are the store's kinds (AGENT-007 section 2.2), and they did not use
// to be. There was a `playbooks` tab that no kind answers to, and no tab for
// `platform_defect` — so `report_evaluator_defect` wrote somewhere nobody could
// look, and a tab that could never fill sat next to six that could.
//
// `/api/memory/{bucket}` refuses an unknown bucket by name rather than
// returning an empty list, because the two render almost identically and mean
// opposite things: "you have recorded none of these" is a fact about your
// research, and "this tab is connected to nothing" is a fact about the platform.
// Only the first invites you to stop looking.
type Bucket = 'findings' | 'negative' | 'profiles' | 'questions' | 'preferences' | 'defects' | 'signals'

const BUCKETS: Array<{ value: Bucket; label: string }> = [
  { value: 'findings', label: 'Findings' },
  { value: 'negative', label: 'Negative results' },
  { value: 'profiles', label: 'Instrument profiles' },
  { value: 'questions', label: 'Open questions' },
  { value: 'preferences', label: 'Preferences' },
  { value: 'defects', label: 'Platform defects' },
  { value: 'signals', label: 'Signal registry' },
]

export interface MemoryRow {
  id: string
  subject: string
  claim: string
  confidence?: string
  n?: number | null
  window?: string
  valid_until?: string | null
  cited?: number
  /**
   * What `/api/memory/{bucket}` actually calls the cited count.
   *
   * The row was read as `cited` and served as `cited_by_chain`, so the count
   * never rendered on any tab — AGENT-012 §7 lists it as a column of the
   * findings table. A field-name miss fails silently in exactly the way this
   * platform's whole claim is against: the surface did not say "cited count
   * unknown", it said nothing, and a finding nothing has cited looked identical
   * to one cited nine times.
   */
  cited_by_chain?: number
  /**
   * The store's own staleness flag, which is **not** the confidence.
   *
   * `confidence` is how strong the claim was when it was made and `stale` is
   * whether it has outlived its window. A stale `established` finding rendered
   * as `established`, which is the platform asserting a measurement is current
   * when its own row says it is not.
   */
  stale?: boolean
  corroborations?: number
  /** Ids of rows this one disagrees with. */
  conflicts_with?: string[]
  provenance?: { run?: string; tool?: string; window?: string }
  /**
   * The provenance fence, when the row carries it (§6.3a).
   *
   * Optional because `/api/memory/{bucket}` does not serve it yet — `row()` in
   * `crates/api/src/routes/memory.rs` omits `evidence_class`, which the entry
   * has. Read here rather than hardcoded to `null` as it was, so the mark
   * reaches the reader the day the field is added instead of needing this
   * component found again.
   */
  mark?: ProvenanceMark | null
  evidence_class?: EvidenceClass
  /**
   * AGENT-007 section 2.2's right-hand column: the fields that belong to one kind
   * rather than to all seven.
   *
   * Stored since migration 0122 and served by nothing until 2026-09-17, so every
   * tab rendered the generic half of an entry and none of the half that makes it
   * actionable — an open question and a closed one looked identical.
   */
  extra?: Record<string, unknown>
}

/** An open question's own fields (AGENT-013 section 9). */
export interface QuestionFields {
  why_it_matters?: string
  blocking: boolean
  value?: string
  /** `'open'`, or `{ closed: { by, how } }`. */
  closedBy?: string
  closedHow?: string
}

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
 * Pairs every conflict exactly once.
 *
 * Both sides are returned together so the renderer cannot show one and rank the
 * other below the fold.
 */
export function conflictPairs(rows: MemoryRow[]): Array<[MemoryRow, MemoryRow]> {
  const byId = new Map(rows.map((r) => [r.id, r]))
  const seen = new Set<string>()
  const out: Array<[MemoryRow, MemoryRow]> = []
  for (const r of rows) {
    for (const other of r.conflicts_with ?? []) {
      const key = [r.id, other].sort().join('|')
      if (seen.has(key)) continue
      const o = byId.get(other)
      if (!o) continue
      seen.add(key)
      out.push([r, o])
    }
  }
  return out
}

function Row({ row, onRetry }: { row: MemoryRow; onRetry?: (r: MemoryRow) => void }) {
  const q = questionFields(row)
  const cited = citedCount(row)
  return (
    <div className="desk-card">
      <div className="desk-card-hd">
        <Badge tone="outline">{row.subject}</Badge>
        {row.confidence && <Badge tone={row.confidence === 'refuted' ? 'neg' : 'neutral'}>{row.confidence}</Badge>}
        {/* Beside the confidence, never instead of it: `established · stale` is
            a claim that was strong and has outlived its window, and collapsing
            the two would either hide the strength or hide the expiry. */}
        {row.stale === true && <Badge tone="warn">stale</Badge>}
        {q && (q.closedBy ? (
          <Badge tone="neutral">closed · {q.closedHow ?? 'answered'}</Badge>
        ) : (
          <Badge tone={q.blocking ? 'neg' : 'outline'}>{q.blocking ? 'blocking' : 'open'}</Badge>
        ))}
        {q?.value && <Badge tone="outline">{q.value}</Badge>}
        <ProvenanceBadge mark={rowMark(row)} />
      </div>
      <p>{row.claim}</p>
      {q && (
        <p className="sub">
          {/* The field the deliberate door exists for. A question whose stakes
              were never stated says so rather than showing an empty line — the
              run-end write records exactly that, and a reader deciding what to
              spend a run on needs to tell the two apart. */}
          why it matters: {q.why_it_matters ?? 'not stated'}
          {q.closedBy ? ` · closed by ${q.closedBy}` : ''}
        </p>
      )}
      <p className="sub">
        {row.n === null || row.n === undefined ? '(n unknown)' : `n=${row.n}`}
        {row.window ? ` · ${row.window}` : ''}
        {row.valid_until ? ` · valid until ${row.valid_until}` : ''}
        {/* Zero citations is a real and useful answer — a finding nothing has
            leaned on — so it is printed rather than hidden behind a truthiness
            check. An unknown count says so instead. */}
        {cited === null ? ' · cited count not recorded' : ` · cited ${cited}×`}
      </p>
      <p className="sub">
        provenance: run {row.provenance?.run ?? 'unknown'} · tool {row.provenance?.tool ?? 'unknown'} · data{' '}
        {row.provenance?.window ?? 'unknown'}
      </p>
      {onRetry && (
        <Button size="xs" variant="ghost" onClick={() => onRetry(row)}>
          Retry now
        </Button>
      )}
    </div>
  )
}

export function MemoryBrowser({ onOpenConversation }: { onOpenConversation?: (seed: string) => void }) {
  const [bucket, setBucket] = useState<Bucket>('findings')
  const [q, setQ] = useState('')
  const [conflictsOnly, setConflictsOnly] = useState(false)
  // Closed questions are hidden by default and counted in the button, never
  // dropped from the response: a question nobody can find again is one the
  // platform cannot be asked "who answered this".
  const [hideClosed, setHideClosed] = useState(true)

  const data = useQuery({
    queryKey: ['desk', 'memory', bucket],
    queryFn: () => deskApi.memory(bucket),
  })

  const rows = useMemo(() => (data.data?.rows ?? []) as unknown as MemoryRow[], [data.data])
  const closedCount = useMemo(
    () => rows.filter((r) => questionFields(r)?.closedBy).length,
    [rows],
  )
  const filtered = rows
    .filter((r) => `${r.claim} ${r.subject}`.toLowerCase().includes(q.toLowerCase()))
    // Open first, and blocking ahead of the rest — the same order the brief
    // uses (`questions::brief_order`), so what a run is shown first and what a
    // person is shown first are the same thing.
    .sort((a, b) => {
      const qa = questionFields(a)
      const qb = questionFields(b)
      if (!qa || !qb) return 0
      const rank = (x: QuestionFields) => (x.closedBy ? 2 : x.blocking ? 0 : 1)
      return rank(qa) - rank(qb)
    })
    .filter((r) => {
      if (!hideClosed) return true
      const f = questionFields(r)
      return !f || !f.closedBy
    })
  const pairs = useMemo(() => conflictPairs(rows), [rows])

  return (
    <div className="desk-scroll">
      <Tabs<Bucket> value={bucket} onChange={setBucket} tabs={BUCKETS} ariaLabel="Memory" />
      <div className="desk-actions">
        <SearchInput value={q} onValueChange={setQ} placeholder="Search memory" />
        <Button size="xs" variant={conflictsOnly ? 'primary' : 'ghost'} onClick={() => setConflictsOnly((c) => !c)}>
          Conflicts ({pairs.length})
        </Button>
        {bucket === 'questions' && (
          <Button size="xs" variant={hideClosed ? 'ghost' : 'primary'} onClick={() => setHideClosed((h) => !h)}>
            {hideClosed ? `Show closed (${closedCount})` : `Hide closed (${closedCount})`}
          </Button>
        )}
      </div>

      {data.data === null && (
        <EmptyState
          message="Memory is unavailable"
          detail="/api/memory/{bucket} is served, so this is the endpoint failing rather than the endpoint missing — check the API logs."
        />
      )}

      {conflictsOnly ? (
        pairs.length === 0 ? (
          <EmptyState message="No conflicts" detail="Nothing in this bucket disagrees with anything else." />
        ) : (
          pairs.map(([a, b]) => (
            <section
              key={`${a.id}|${b.id}`}
              style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-3)', marginTop: 'var(--s-3)' }}
              aria-label={`conflict between ${a.id} and ${b.id}`}
            >
              <Row row={a} />
              <Row row={b} />
              <div className="full">
                <Button
                  size="xs"
                  onClick={() => onOpenConversation?.(`diagnose the conflict between ${a.id} and ${b.id}`)}
                >
                  Resolve
                </Button>
              </div>
            </section>
          ))
        )
      ) : (
        filtered.map((r) => (
          <Row
            key={r.id}
            row={r}
            onRetry={
              bucket === 'negative' ? (row) => onOpenConversation?.(`retry: ${row.claim}`) : undefined
            }
          />
        ))
      )}
    </div>
  )
}
