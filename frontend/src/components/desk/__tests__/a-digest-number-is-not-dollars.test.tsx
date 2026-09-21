/**
 * The digest card, and the number on it that looks like money and is not
 * (AGENT-014 §2.4, §9; Set S task S-11.13).
 *
 * `GET …/{id}/digests` has been served since S-11.14 and nothing called it, so
 * the one object that makes returning to a week-old session cheap did not
 * render anywhere. Building the card raised the trap that migration 0139 was
 * written for:
 *
 * **The digest's spend is counted in trials.** There is no per-run USD on this
 * platform — `llm_usage.cost_usd` keys on the auth session, not the run, which
 * is why `select.rs` scores cost in trials too — and the digest body's own
 * sentence reads `"3.00 spent over 2 runs"` with **no unit in it**. On a card
 * beside a charter that says `$25/day`, an unlabelled `3.00` is a trial count a
 * reader will spend as money. 0139 adds `session_digests.spend_unit` NOT NULL
 * with no default so no writer can leave the question open; the route does not
 * select it yet, so the card has to read it optionally and refuse to draw a
 * currency symbol without it.
 *
 * The second half is §2.4's other sentence — "each line opens the thing it
 * names", §12.11's "every line resolves on click". A line whose reference
 * carries an id has to be a control that opens it, and a line whose reference
 * is empty, or whose kind has no surface, has to say so in words rather than
 * render as a button that eats the click.
 */
import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import type { SessionDigestRow } from '@/api/desk'
import { sessionDigestTarget, sessionDigestView } from '@/api/desk'
import { DigestCard, DigestCards } from '../DigestCard'

function row(over: Partial<SessionDigestRow> = {}): SessionDigestRow {
  return {
    digest_id: 'd_1',
    from_seq: 10,
    to_seq: 24,
    created_at: '2026-09-21T09:00:00Z',
    body: {
      session: 's_1',
      period: {
        from: '2026-09-20T09:00:00Z',
        to: '2026-09-21T09:00:00Z',
        from_seq: 10,
        to_seq: 24,
      },
      budget: {
        runs_finished: 2,
        spent: 3,
        knowledge_units: 2,
        cost_per_knowledge_unit: 1.5,
      },
      sections: [
        {
          heading: 'budget',
          lines: [{ at: { kind: 'session', id: 's_1' }, text: '3.00 spent over 2 runs' }],
        },
        {
          heading: 'runs_finished',
          lines: [{ at: { kind: 'run', id: 'r_9' }, text: 'refuted — 1 knowledge unit' }],
        },
      ],
    },
    ...over,
  }
}

function view(over: Partial<SessionDigestRow> = {}) {
  const v = sessionDigestView(row(over))
  if (!v) throw new Error('the fixture must decode, or every assertion below is vacuous')
  return v
}

describe('the digest names the unit its spend is in, or says it does not know', () => {
  it('prints no currency symbol when the route did not send a unit', () => {
    render(<DigestCard digest={view()} />)
    const card = screen.getByTestId('digest-card')
    // Asserted present first: a card that rendered nothing would pass the
    // "contains no dollar sign" check for the wrong reason.
    expect(card.textContent).toContain('3.00')
    expect(card.textContent).not.toContain('$')
    expect(screen.getByTestId('digest-unit-unknown').textContent).toContain('trials per run')
  })

  it('says trials when the unit is trials, and still never a dollar sign', () => {
    render(<DigestCard digest={view({ spend_unit: 'trials' })} />)
    const card = screen.getByTestId('digest-card')
    expect(card.textContent).toContain('3.00 trials spent over 2 runs')
    expect(card.textContent).toContain('1.50 trials per knowledge unit')
    expect(card.textContent).not.toContain('$')
    expect(screen.queryByTestId('digest-unit-unknown')).toBeNull()
  })

  it('prints dollars only once something says the unit is dollars', () => {
    render(<DigestCard digest={view({ spend_unit: 'usd' })} />)
    expect(screen.getByTestId('digest-card').textContent).toContain('$3.00 spent over 2 runs')
    expect(screen.getByTestId('digest-card').textContent).toContain('$1.50 per knowledge unit')
  })

  it('reads the unit off the body too, because the driver writes it on the event', () => {
    const v = sessionDigestView(row({ body: { ...row().body!, spend_unit: 'trials' } }))
    expect(v?.spend_unit).toBe('trials')
  })

  it('refuses a unit nobody defined rather than falling back to dollars', () => {
    expect(sessionDigestView(row({ spend_unit: 'credits' }))?.spend_unit).toBeNull()
    expect(sessionDigestView(row())?.spend_unit).toBeNull()
  })

  it('does not print the server sentence, which carries no unit at all', () => {
    // The digest body's own budget line is `"{:.2} spent over 2 runs"`. Rendered
    // verbatim it is the defect; the card builds its own sentence instead.
    render(<DigestCard digest={view({ spend_unit: 'trials' })} />)
    expect(screen.queryByText('3.00 spent over 2 runs')).toBeNull()
  })

  it('says so rather than inventing a summary when the body carries no budget', () => {
    const bodyless = { ...row().body!, budget: undefined }
    render(<DigestCard digest={sessionDigestView(row({ body: bodyless }))!} />)
    expect(screen.getByTestId('digest-card').textContent).toContain('no budget summary')
  })
})

describe('every line opens the thing it names, or says why it cannot', () => {
  it('renders a run line as a control and hands the reference back', () => {
    const onOpen = vi.fn()
    render(<DigestCard digest={view()} onOpen={onOpen} />)
    fireEvent.click(screen.getByText('refuted — 1 knowledge unit'))
    expect(onOpen).toHaveBeenCalledWith({ kind: 'run', id: 'r_9' })
  })

  it('does not render a line with no id as a control that swallows the click', () => {
    const onOpen = vi.fn()
    const body = {
      ...row().body!,
      sections: [{ heading: 'findings', lines: [{ at: { kind: 'finding', id: '' }, text: 'established: x' }] }],
    }
    render(<DigestCard digest={sessionDigestView(row({ body }))!} onOpen={onOpen} />)
    expect(screen.getByTestId('digest-line-dead').textContent).toContain('carries no id')
    fireEvent.click(screen.getByText('established: x'))
    expect(onOpen).not.toHaveBeenCalled()
    // And the card counts them, the way `Digest::unresolvable` does on the
    // other side, so the gap is readable rather than only invisible.
    expect(screen.getByTestId('digest-unresolvable').textContent).toContain('1 line')
  })

  it('names the missing surface for a pause line rather than pretending', () => {
    // A pause line opens a `session_events` row by `seq`, and nothing in the
    // desk renders one. That is a gap; a button that did nothing would hide it.
    const onOpen = vi.fn()
    const body = {
      ...row().body!,
      sections: [{ heading: 'paused', lines: [{ at: { kind: 'event', id: '17' }, text: 'stalled: barren' }] }],
    }
    render(<DigestCard digest={sessionDigestView(row({ body }))!} onOpen={onOpen} />)
    expect(screen.getByTestId('digest-line-dead').textContent).toContain('session log')
    // Not counted as unresolvable: the line has an id, the desk has no screen.
    expect(screen.queryByTestId('digest-unresolvable')).toBeNull()
  })

  it('sends each kind somewhere a person can actually get to', () => {
    expect(sessionDigestTarget({ kind: 'run', id: 'r_1' })).toEqual({ kind: 'run', runId: 'r_1' })
    expect(sessionDigestTarget({ kind: 'item', id: 'i_1' })).toEqual({ kind: 'panel', tab: 'agenda' })
    expect(sessionDigestTarget({ kind: 'session', id: 's_1' })).toEqual({ kind: 'panel', tab: 'charter' })
    expect(sessionDigestTarget({ kind: 'finding', id: 'f_31a' })).toEqual({
      kind: 'route',
      to: '/desk/memory?bucket=findings&find=f_31a',
    })
    expect(sessionDigestTarget({ kind: 'negative', id: 'n_2' }).kind).toBe('route')
    expect(sessionDigestTarget({ kind: 'question', id: 'q_2' }).kind).toBe('route')
    expect(sessionDigestTarget({ kind: 'proposal', id: 'p_2' })).toEqual({
      kind: 'route',
      to: '/desk/proposals',
    })
    expect(sessionDigestTarget({ kind: 'event', id: '17' }).kind).toBe('none')
  })
})

describe('the digest list tells three states apart', () => {
  it('says the request did not answer, not that no digest was ever written', () => {
    render(<DigestCards digests={null} />)
    expect(screen.getByText(/Unknown, not none/)).toBeInTheDocument()
  })

  it('has a reading state of its own', () => {
    render(<DigestCards digests={[]} loading />)
    expect(screen.getByText(/Reading the digests/)).toBeInTheDocument()
  })

  it('says a session has not been reviewed yet when the list is genuinely empty', () => {
    render(<DigestCards digests={[]} />)
    expect(screen.getByTestId('no-digests').textContent).toContain('No digest yet')
  })

  it('renders the digests it is given', () => {
    render(<DigestCards digests={[row()]} />)
    expect(screen.getAllByTestId('digest-card').length).toBe(1)
  })
})

describe('the sections come back in the order §2.4 lists them', () => {
  it('puts the budget after the runs, whatever order the row arrived in', () => {
    // The fixture deliberately sends budget first.
    expect(view().sections.map((s) => s.heading)).toEqual(['runs_finished', 'budget'])
  })

  it('keeps a heading it does not recognise instead of dropping its lines', () => {
    const body = {
      ...row().body!,
      sections: [
        { heading: 'something_later', lines: [{ at: { kind: 'run', id: 'r_1' }, text: 'later' }] },
        ...row().body!.sections!,
      ],
    }
    const v = sessionDigestView(row({ body }))!
    expect(v.sections.map((s) => s.heading)).toEqual(['runs_finished', 'budget', 'something_later'])
    render(<DigestCard digest={v} />)
    expect(screen.getByText('later')).toBeInTheDocument()
  })
})
