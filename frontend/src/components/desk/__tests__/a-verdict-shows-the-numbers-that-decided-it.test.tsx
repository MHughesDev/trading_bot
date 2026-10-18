/**
 * S-10.15: a hypothesis card shows the platform's verdict with the metric,
 * comparator, threshold and observed values that decided it.
 *
 * The server (`GET /api/desk/runs/{id}/hypotheses`) reads the ledger; the card
 * must not invent a figure it was not sent. So: a refuted hypothesis names the
 * value that satisfied the falsifier; an open one says it has no verdict; a
 * falsifier that does not read shows the parser's complaint and no threshold;
 * and a failed read is not drawn as "no hypotheses".
 */
import { render, screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { HypothesisVerdict } from '@/api/desk'
import { StateTab } from '../RightPanel'

const base: HypothesisVerdict = {
  hypothesis_id: 'h-1',
  statement: 'momentum survives costs',
  role: 'primary',
  falsifier: 'sharpe <= 0',
  status: 'refuted',
  inconclusive_why: null,
  created_at: '2026-10-04T10:00:00Z',
  decided_at: '2026-10-04T11:00:00Z',
  parsed: { metric: 'sharpe', comparator: '<=', threshold: 0 },
  parse_error: null,
  observed: [
    { ticket_id: 'aaaaaaaa-1111', value: -0.2, recorded_at: '2026-10-04T10:30:00Z', fired: true },
    { ticket_id: 'bbbbbbbb-2222', value: 0.6, recorded_at: '2026-10-04T10:40:00Z', fired: false },
  ],
}

function tab(verdicts: HypothesisVerdict[] | 'unreadable' | null) {
  // No working state: the verdicts come from the ledger, not the snapshot.
  return render(<StateTab state={null} absence="not_served" verdicts={verdicts} />)
}

describe('the hypothesis verdict card', () => {
  it('shows the threshold and the value that decided a refutation', () => {
    tab([base])
    const card = screen.getByTestId('hypothesis-h-1')
    expect(within(card).getByText('refuted')).toBeInTheDocument()
    expect(within(card).getByTestId('falsifier-terms').textContent).toBe('refuted if sharpe <= 0')
    const rows = within(card).getByTestId('observed-values').querySelectorAll('li')
    expect(rows).toHaveLength(2)
    expect(rows[0].textContent).toMatch(/-0\.2 from ticket aaaaaaaa — satisfies the falsifier/)
    expect(rows[1].textContent).not.toMatch(/satisfies/)
  })

  it('says an open hypothesis has no verdict instead of showing a number', () => {
    tab([{ ...base, status: 'open', decided_at: null, observed: [] }])
    expect(screen.getByTestId('no-observations').textContent).toMatch(/no verdict/)
  })

  it('names why an inconclusive one is inconclusive', () => {
    tab([{ ...base, status: 'inconclusive', inconclusive_why: 'insufficient_data', observed: [] }])
    expect(screen.getByText(/inconclusive: insufficient data/)).toBeInTheDocument()
  })

  it('shows the parser’s complaint, and no threshold, for a falsifier that does not read', () => {
    tab([{ ...base, parsed: null, parse_error: 'no metric in `vibes` is one this platform computes', observed: [] }])
    expect(screen.getByTestId('falsifier-unreadable').textContent).toMatch(/does not read: no metric/)
    expect(screen.queryByTestId('falsifier-terms')).toBeNull()
  })

  it('does not draw a failed read as "no hypotheses"', () => {
    tab('unreadable')
    expect(screen.getByTestId('hypotheses-unreadable')).toBeInTheDocument()
    expect(screen.queryByTestId('hypothesis-verdicts')).toBeNull()
  })

  it('draws nothing when the route is not served or the run stated none', () => {
    const { unmount } = tab(null)
    expect(screen.queryByTestId('hypothesis-verdicts')).toBeNull()
    unmount()
    tab([])
    expect(screen.queryByTestId('hypothesis-verdicts')).toBeNull()
  })
})
