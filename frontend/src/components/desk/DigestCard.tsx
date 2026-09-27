// The digest, as a card in the journal (AGENT-014 §2.4, §9; AGENT-012 §2b;
// Set S task S-11.13).
//
// A continuous session that has run for a week is sixty conversations nobody is
// going to open. The digest is the one object that makes returning cheap: what
// ran and with what verdict, what was established and what was refuted, which
// negatives closed a new door, which questions opened and closed, what
// contradicts what, what is waiting for a decision, what it cost, what is next
// and why, and what stopped.
//
// # Every line opens the thing it names
//
// That is §2.4's sentence and §12.11's acceptance, and it is the reason
// `digest::Line` cannot be constructed without a reference. The card's job is to
// keep the promise on this side: a line whose reference carries an id renders as
// a control that opens it, and a line whose reference is empty says so in
// words. It does **not** render as a button that does nothing, which is the
// failure this repository keeps finding — a control that looks like a control
// and reaches nothing.
//
// # The budget line is rewritten here, deliberately
//
// The server's budget sentence is "3.00 spent over 2 runs — 1.50 per knowledge
// unit" with no unit in it, and the number underneath it is trials: there is no
// per-run USD on this platform, which is why `select.rs` scores cost in trials
// too. Printed unchanged on a card next to a charter that says $25/day, that is
// a number a reader spends as money. So this card builds the sentence itself
// from `budget` and `spend_unit` (`desk.ts`'s `sessionDigestBudgetLine`), and without a
// unit it prints no currency symbol and no bar. See `sessionDigestBudgetLine`.
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { EmptyState } from '@/components/primitives/States'
import type { SessionDigestLine, SessionDigestRef, SessionDigestRow, SessionDigestView } from '@/api/desk'
import { sessionDigestBudgetLine, sessionDigestHeading, sessionDigestTarget, sessionDigestView } from '@/api/desk'
import { ago } from '@/components/desk/Shared.helpers'

/** What a click on a line asks the surface to open. */
export type OpenDigestRef = (ref: SessionDigestRef) => void

function periodWords(view: SessionDigestView): string {
  if (!view.period) {
    // The seqs are always there — they are the digest's identity — so a missing
    // period is still addressable, just not in clock time.
    return `events ${view.from_seq}–${view.to_seq}`
  }
  const from = new Date(view.period.from)
  const to = new Date(view.period.to)
  const ok = Number.isFinite(from.getTime()) && Number.isFinite(to.getTime())
  return ok
    ? `${from.toLocaleString()} → ${to.toLocaleString()} · events ${view.from_seq}–${view.to_seq}`
    : `events ${view.from_seq}–${view.to_seq}`
}

function Line({ line, onOpen }: { line: SessionDigestLine; onOpen?: OpenDigestRef }) {
  const target = sessionDigestTarget(line.at)
  if (target.kind === 'none' || !onOpen) {
    return (
      <li>
        <span>{line.text}</span>{' '}
        <span className="sub" data-testid="digest-line-dead">
          ({target.kind === 'none' ? target.why : 'nothing on this screen can open it'})
        </span>
      </li>
    )
  }
  return (
    <li>
      <Button
        size="xs"
        variant="ghost"
        onClick={() => onOpen(line.at)}
        title={`open the ${line.at.kind} ${line.at.id}`}
      >
        {line.text}
      </Button>
    </li>
  )
}

export function DigestCard({ digest, onOpen }: { digest: SessionDigestView; onOpen?: OpenDigestRef }) {
  return (
    <article className="desk-card" data-testid="digest-card" aria-label="Digest">
      <div className="desk-card-hd">
        <Badge tone="info">digest</Badge>
        <span className="sub">{periodWords(digest)}</span>
        <span className="sub">{ago(digest.created_at)}</span>
      </div>

      {digest.sections.length === 0 && (
        // Not an empty card. `write` omits a section with nothing in it, so a
        // digest with no sections at all is a period in which nothing happened
        // — which is itself the thing the reader came to find out.
        <p className="sub" data-testid="digest-quiet">
          Nothing happened in this period: no run finished, nothing was found and nothing stopped.
        </p>
      )}

      {digest.sections.map((s) => (
        <section key={s.heading}>
          <span className="lbl">{sessionDigestHeading(s.heading)}</span>
          <ul>
            {s.heading === 'budget' ? (
              // One line, written from the numbers rather than from the
              // server's sentence, for the reason in the header. The reference
              // is the server's, so it still opens the session.
              <Line
                line={{ at: s.lines[0]?.at ?? { kind: 'session', id: '' }, text: sessionDigestBudgetLine(digest) }}
                onOpen={onOpen}
              />
            ) : (
              s.lines.map((l, i) => <Line key={`${l.at?.kind}-${l.at?.id}-${i}`} line={l} onOpen={onOpen} />)
            )}
          </ul>
          {s.heading === 'budget' && digest.spend_unit === null && (
            <p className="sub" data-testid="digest-unit-unknown">
              No unit was recorded for that number. It is not being shown against the day budget,
              because this platform measures trials per run and not dollars, and a trial count read
              as money is a number nobody measured.
            </p>
          )}
        </section>
      ))}

      {digest.unresolvable > 0 && (
        <p className="sub" data-testid="digest-unresolvable">
          {digest.unresolvable} line{digest.unresolvable === 1 ? '' : 's'} on this digest carr
          {digest.unresolvable === 1 ? 'ies' : 'y'} no id, so {digest.unresolvable === 1 ? 'it names' : 'they name'}{' '}
          nothing that can be opened.
        </p>
      )}
    </article>
  )
}

/**
 * The digests of one session, newest first, as the journal shows them.
 *
 * Three states rather than two, the way the Agenda tab has them: reading, the
 * request not answering, and a session that has genuinely not been reviewed
 * yet. The middle one is a failure and the last one is ordinary, and they must
 * not look the same.
 */
export function DigestCards({
  digests,
  loading = false,
  onOpen,
}: {
  digests: SessionDigestRow[] | null
  loading?: boolean
  onOpen?: OpenDigestRef
}) {
  if (loading) return <EmptyState message="Reading the digests…" />
  if (digests === null) {
    return (
      <EmptyState
        message="The digest request did not answer"
        detail="/api/research/sessions/{id}/digests is served, so this is the call failing rather than the session never having been reviewed. Unknown, not none."
      />
    )
  }
  if (digests.length === 0) {
    return (
      <p className="sub" data-testid="no-digests">
        No digest yet. One is written every `review_every`, and on a stall, a stop and a completion.
      </p>
    )
  }
  return (
    <div data-testid="digests">
      {digests.map((row) => {
        const view = sessionDigestView(row)
        return view ? <DigestCard key={row.digest_id} digest={view} onOpen={onOpen} /> : null
      })}
    </div>
  )
}
