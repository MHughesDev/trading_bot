/**
 * "Since you were away" must not be blind about being blind (AGENT-012 §2.2a,
 * §19; Set S task S-6.24).
 *
 * `/api/desk/digest` went out of its way to say what it cannot see: three of the
 * seven line kinds have no source in this deployment — `finding` and `refuted`
 * need phase 4's memory migrations, `monitor` needs a state-*change* log — and
 * the response carries `kinds_not_covered` with a reason for each, plus
 * `absent: true` for a gap under an hour rather than an empty list. There is a
 * Rust test asserting every kind is in exactly one of the two lists.
 *
 * **The frontend dropped both fields on the floor.** `AwayDigest` typed the
 * response as `{since, until, lines}`, so the panel never saw them, and it
 * returned `null` whenever the line list was empty. A digest that cannot see
 * refutations therefore rendered exactly like a night on which nothing was
 * refuted — which is the one distinction the operator reading it is making,
 * because they are deciding whether to stop looking.
 *
 * So these assert the three absences stay three different things: nothing
 * happened, I cannot see some of what happened, and I could not be read at all.
 */
import { render, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { describe, expect, it } from 'vitest'

import type { AwayDigest } from '@/api/desk'
import {
  AwayDigestPanel,
  MIN_GAP_MS,
  blindSpotNote,
  hasSomethingToSay,
  linesInGap,
} from '../AwayDigest'

const NOW = Date.parse('2026-09-20T18:00:00Z')
const LAST_VISIT = NOW - 8 * 60 * 60 * 1000

const GAPS: AwayDigest['kinds_not_covered'] = [
  { kind: 'finding', why: 'findings are not persisted in this deployment' },
  { kind: 'refuted', why: 'same source as `finding`' },
  { kind: 'monitor', why: 'the monitor tables record current state, not state changes' },
]

function digest(over: Partial<AwayDigest> = {}): AwayDigest {
  return {
    since: new Date(LAST_VISIT).toISOString(),
    until: new Date(NOW).toISOString(),
    lines: [
      {
        id: 'l_1',
        at: new Date(NOW - 60 * 60 * 1000).toISOString(),
        kind: 'concluded',
        text: 'the EMA-cross family did not clear Gate 3',
        href: '/desk/c/abc',
      },
    ],
    kinds_not_covered: GAPS,
    ...over,
  }
}

function panel(props: Partial<React.ComponentProps<typeof AwayDigestPanel>> = {}) {
  return render(
    <MemoryRouter>
      <AwayDigestPanel data={digest()} lastVisit={LAST_VISIT} now={NOW} {...props} />
    </MemoryRouter>,
  )
}

describe('the digest says what it cannot see', () => {
  it('names every uncovered kind and its reason', () => {
    const note = blindSpotNote(digest())
    expect(note).not.toBeNull()
    for (const g of GAPS) expect(note).toContain(g.kind)
    expect(note).toContain('not persisted')
  })

  it('says nothing when the server declared no blind spots', () => {
    // An omitted note and an empty one must not look alike either: this returns
    // null only because the server said the list was empty.
    expect(blindSpotNote(digest({ kinds_not_covered: [] }))).toBeNull()
  })

  it('renders the blind spots beside the lines, not behind a disclosure', () => {
    panel()
    expect(screen.getByTestId('digest-blind-spots').textContent).toContain('monitor')
    // And the line itself is still there, so the assertion above cannot be a
    // panel that rendered only its footnote.
    expect(screen.getByText(/did not clear Gate 3/)).toBeInTheDocument()
  })
})

describe('an empty digest and a blind one are different panels', () => {
  it('still renders with no lines when kinds are uncovered', () => {
    // This is the defect: the panel used to `return null` here, so the operator
    // was told nothing at all rather than "I cannot see three of the seven".
    panel({ data: digest({ lines: [] }) })
    expect(screen.getByTestId('digest-quiet')).toBeInTheDocument()
    expect(screen.getByTestId('digest-blind-spots')).toBeInTheDocument()
  })

  it('stays out of the way when there is nothing to say and nothing unseen', () => {
    expect(hasSomethingToSay(digest({ lines: [], kinds_not_covered: [] }), 0)).toBe(false)
    expect(hasSomethingToSay(digest({ lines: [] }), 0)).toBe(true)
    expect(hasSomethingToSay(digest(), 1)).toBe(true)
  })

  it('honours the server’s own "absent", not just the local gap arithmetic', () => {
    // The under-an-hour rule is enforced on both sides; the server is the one
    // that knows, and `absent` must win even when the browser's marker is stale.
    expect(hasSomethingToSay(digest({ absent: true, lines: [] }), 0)).toBe(false)
    const { container } = panel({ data: digest({ absent: true }) })
    expect(container.textContent).toBe('')
  })
})

describe('a digest that could not be read says so', () => {
  it('names its own absence rather than rendering nothing', () => {
    // `optional()` turns a 404 into null. A panel whose whole job is to save the
    // operator an hour of reconstruction must not hide the fact that it failed.
    panel({ data: null })
    expect(screen.getByTestId('digest-unavailable')).toBeInTheDocument()
  })

  it('does not report a request still in flight as one that failed', () => {
    const { container } = panel({ data: null, loading: true })
    expect(container.textContent).toBe('')
  })
})

describe('the gap rules that were already right stay right', () => {
  it('keeps only the lines inside the gap', () => {
    const inside = digest().lines
    const stale = [
      ...inside,
      { id: 'old', at: new Date(LAST_VISIT - 1000).toISOString(), kind: 'budget' as const, text: 'x', href: null },
    ]
    expect(linesInGap(stale, LAST_VISIT, NOW).map((l) => l.id)).toEqual(['l_1'])
  })

  it('is not shown for a gap under an hour', () => {
    const { container } = panel({ lastVisit: NOW - MIN_GAP_MS + 1000 })
    expect(container.textContent).toBe('')
  })
})
