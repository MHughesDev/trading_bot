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
import { questionFields, rowMark, citedCount, conflictPairs, bucketFromParam } from './MemoryBrowser.helpers'
import { BUCKETS } from './MemoryBrowser.helpers'

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
export type Bucket = 'findings' | 'negative' | 'profiles' | 'questions' | 'preferences' | 'defects' | 'signals'

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
 * One memory entry, with the mark that says what kind of claim it is.
 *
 * Exported so the provenance fence can be asserted where a person meets it —
 * the rendered row — rather than only on `rowMark`, which is the half that was
 * already covered. `/api/memory/{bucket}` began serving `evidence_class` on
 * 2026-09-21, and the difference between a `model_summary` and a `direct_stat`
 * arriving and *reaching the reader* is one `<ProvenanceBadge>` nobody had
 * looked at since the field existed.
 */
export function MemoryRowCard({ row, onRetry }: { row: MemoryRow; onRetry?: (r: MemoryRow) => void }) {
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

/**
 * Why a bucket could not be read — one sentence per refusal, not one for all.
 *
 * The panel used to print **"/api/memory/{bucket} is served, so this is the
 * endpoint failing rather than the endpoint missing — check the API logs"** over
 * every failure. For six buckets that is true. For `preferences` it is false in
 * a way that costs somebody an afternoon: `GET /api/memory/preferences` answers
 * **405**, because `routes/mod.rs` registers
 *
 *   `.route("/api/memory/preferences", post(memory::record_preference))`
 *
 * before
 *
 *   `.route("/api/memory/{bucket}", get(memory::list))`
 *
 * and axum's matcher prefers a static segment over a parameter. The literal
 * route takes the path and then refuses the method, so `memory::list` is never
 * reached for that one bucket, and the API log records nothing because nothing
 * went wrong. Sending a reader to the logs for a route-table collision is the
 * worst kind of empty state: it is confident, actionable, and points away from
 * the cause.
 *
 * Keyed on the status the platform actually answered rather than on a list of
 * known-shadowed buckets, so this sentence corrects itself when the route does.
 */
export function BucketRefusal({ bucket, status }: { bucket: string; status: number | null }) {
  if (status === 405) {
    return (
      <EmptyState
        message={`The ${bucket} bucket cannot be read`}
        detail={
          <>
            <code>GET /api/memory/{bucket}</code> answers <strong>405 Method Not Allowed</strong>,
            which is not a server failure and not an empty bucket. Another route is registered on
            exactly this path for a different method — <code>POST /api/memory/preferences</code>{' '}
            records what a user told the agent — and a literal path wins over{' '}
            <code>/api/memory/&#123;bucket&#125;</code> in the router, so the list handler is never
            reached. Nothing here can work around it and there is nothing in the API log to find:
            it needs the route table changed. Entries written to this bucket are not lost, only
            unreadable from this page.
          </>
        }
      />
    )
  }
  if (status === 404) {
    return (
      <EmptyState
        message={`Nothing serves the ${bucket} bucket`}
        detail="The route is not mounted on this platform — a 404 here, where every mounted route answers 401 unauthenticated. That is a fact about the build, not about your research."
      />
    )
  }
  if (status === 401 || status === 403) {
    return (
      <EmptyState
        message="Not your memory to read"
        detail="The platform refused this read for want of a credential. Sign in again; the store is there."
      />
    )
  }
  return (
    <EmptyState
      message="Memory is unavailable"
      detail={
        status === null
          ? 'The request did not complete and carried no status — a dropped connection rather than an answer.'
          : `/api/memory/${bucket} is served and answered ${status}, so this is the endpoint failing rather than the endpoint missing — the API log has it.`
      }
    />
  )
}

/** The query string, or an empty one where there is no browser to ask. */
function deepLink(): URLSearchParams {
  try {
    return new URLSearchParams(window.location.search)
  } catch {
    return new URLSearchParams()
  }
}

export function MemoryBrowser({ onOpenConversation }: { onOpenConversation?: (seed: string) => void }) {
  // `?bucket=findings&find=f_31a` — how a digest line opens the record it names
  // (AGENT-014 §2.4). Read once as the initial state rather than held in the
  // URL, so the tabs and the search box still belong to the person once they
  // have arrived.
  //
  // Straight off `location.search` rather than through `useSearchParams`: this
  // browser is rendered on its own in three tests with no router around it, and
  // a hook that throws outside a `<Router>` would make a deep link cost four
  // passing tests their setup.
  const params = deepLink()
  const [bucket, setBucket] = useState<Bucket>(() => bucketFromParam(params.get('bucket')) ?? 'findings')
  const [q, setQ] = useState(() => params.get('find') ?? '')
  const [conflictsOnly, setConflictsOnly] = useState(false)
  // Closed questions are hidden by default and counted in the button, never
  // dropped from the response: a question nobody can find again is one the
  // platform cannot be asked "who answered this".
  const [hideClosed, setHideClosed] = useState(true)

  const data = useQuery({
    queryKey: ['desk', 'memory', bucket],
    queryFn: () => deskApi.memory(bucket),
  })

  const read = data.data ?? null
  const rows = useMemo(
    () => ((read?.ok ? read.data.rows : []) as unknown as MemoryRow[]),
    [read],
  )
  const closedCount = useMemo(
    () => rows.filter((r) => questionFields(r)?.closedBy).length,
    [rows],
  )
  const filtered = rows
    // The id is searched as well as the prose. Every other surface quotes a
    // record by its id — a digest line, an evidence chip, a dossier's funnel —
    // and pasting one of those into this box used to find nothing at all.
    .filter((r) => `${r.id} ${r.claim} ${r.subject}`.toLowerCase().includes(q.toLowerCase()))
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
  /** Whether `rows` is a fact about the bucket rather than the shape of a failure. */
  const readable = read?.ok === true

  return (
    <div className="desk-scroll">
      <Tabs<Bucket> value={bucket} onChange={setBucket} tabs={BUCKETS} ariaLabel="Memory" />
      <div className="desk-actions">
        <SearchInput value={q} onValueChange={setQ} placeholder="Search memory" />
        {/* Counted off `rows`, and `rows` is empty both when the bucket holds
            nothing and when the read failed. `Conflicts (0)` over a bucket
            nobody could read is a measured zero standing in for an absent
            measurement — the same defect as the budget meter's `0 / 40,000`,
            on the one button whose number is a claim that your memory does not
            disagree with itself. */}
        <Button size="xs" variant={conflictsOnly ? 'primary' : 'ghost'} onClick={() => setConflictsOnly((c) => !c)}>
          {readable ? `Conflicts (${pairs.length})` : 'Conflicts (unknown)'}
        </Button>
        {bucket === 'questions' && (
          <Button size="xs" variant={hideClosed ? 'ghost' : 'primary'} onClick={() => setHideClosed((h) => !h)}>
            {!readable
              ? 'Closed questions (unknown)'
              : hideClosed
                ? `Show closed (${closedCount})`
                : `Hide closed (${closedCount})`}
          </Button>
        )}
      </div>

      {read && !read.ok && <BucketRefusal bucket={bucket} status={read.status} />}

      {conflictsOnly ? (
        /* "Nothing in this bucket disagrees with anything else" is a statement
           about the bucket's contents, and an unread bucket has none to make it
           about. The refusal above is the whole answer in that case. */
        !readable ? null : pairs.length === 0 ? (
          <EmptyState message="No conflicts" detail="Nothing in this bucket disagrees with anything else." />
        ) : (
          pairs.map(({ row, other, otherId }) => (
            <section
              key={`${row.id}|${otherId}`}
              style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 'var(--s-3)', marginTop: 'var(--s-3)' }}
              aria-label={`conflict between ${row.id} and ${otherId}`}
            >
              <MemoryRowCard row={row} />
              {other ? (
                <MemoryRowCard row={other} />
              ) : (
                /* The half this bucket does not hold. Stated, because the
                   alternative was tried and it was worse: `conflictPairs` used to
                   drop the pair, so a declared disagreement whose counterpart sat
                   in another bucket disappeared and the button above read
                   `Conflicts (0)` over data that says otherwise. */
                <EmptyState
                  message={`${otherId} is not in this bucket`}
                  detail="This row records a disagreement with an id the bucket does not hold — another bucket, or a row the store did not return. The conflict is real and is counted; the other side of it is not on this page, so it is named rather than drawn."
                />
              )}
              <div className="full">
                <Button
                  size="xs"
                  onClick={() => onOpenConversation?.(`diagnose the conflict between ${row.id} and ${otherId}`)}
                >
                  Resolve
                </Button>
              </div>
            </section>
          ))
        )
      ) : data.isPending ? (
        <p className="sub">Reading {bucket}…</p>
      ) : rows.length === 0 && read?.ok ? (
        /* Four states, not two. An empty bucket used to render as a blank
           panel — the same blank as a request in flight and as one that failed
           — and this file's own header argues the distinction: "you have
           recorded none of these" is a fact about your research, and a blank is
           a fact about nothing. */
        <EmptyState
          message={`Nothing is recorded under ${bucket}`}
          detail="The store answered with an empty list, which is a fact about your research rather than about this page."
        />
      ) : !readable ? (
        /* Nothing more to say. The refusal above is the answer, and this branch
           used to stack a second empty state under it reading **"0 rows are in
           this bucket and none matches the filter"** — a count off an array
           that is the shape of a failure, contradicting the sentence directly
           above it. Found by rendering the pre-change component against a 405,
           not by reading it. */
        null
      ) : filtered.length === 0 ? (
        <EmptyState
          message="No row matches this search"
          detail={`${rows.length} ${rows.length === 1 ? 'row is' : 'rows are'} in this bucket and none matches the filter. Clear it to see them.`}
        />
      ) : (
        filtered.map((r) => (
          <MemoryRowCard
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
