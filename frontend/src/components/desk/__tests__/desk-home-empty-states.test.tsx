/**
 * "Nothing is waiting on you" is a claim, and the desk was making it without
 * looking (AGENT-012 §2, §14; Meridian §3.22; Set S task S-6.14).
 *
 * Desk home's "Needs you" panel rendered its empty state whenever
 * `approvals.data?.approvals ?? []` was empty — which is also what a request in
 * flight and a failed request look like at that boundary. So a run suspended in
 * `AwaitingApproval`, waiting on a human who cannot be told, appeared on the
 * operator's home screen as an evening with nothing to do.
 *
 * Underneath it the empty state printed "No decision has been recorded on this
 * desk yet", from a hardcoded `null` — an assertion about the desk's entire
 * history, made without a query. The acceptance asks the opposite: the empty
 * state says what was **last decided**, never a bare "No data".
 *
 * `lastDecision` reads the decision log out of `/api/approvals?state=all`, which
 * returns every approval newest-first with its `answer` and `answered_at`.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { LastDecision } from '../DeskHome'
import { lastDecision } from '@/components/desk/DeskHome.helpers'

const answered = [
  {
    approval_id: 'a_1',
    kind: 'spend_holdout_look',
    answer: 'approve',
    answered_by: 'mason',
    answered_at: '2026-09-20T09:00:00Z',
  },
  {
    approval_id: 'a_2',
    kind: 'promote_strategy',
    answer: 'deny',
    answered_by: 'mason',
    answered_at: '2026-09-20T17:30:00Z',
  },
]

describe('the last decision is read, not assumed', () => {
  it('names the most recently answered approval and what was chosen', () => {
    const text = lastDecision(answered)
    expect(text).toContain('promote_strategy')
    expect(text).toContain('deny')
    expect(text).toContain('mason')
  })

  it('ignores approvals that are still waiting', () => {
    const pending = [{ approval_id: 'a_3', kind: 'unseal_holdout', state: 'pending', answer: null }]
    expect(lastDecision(pending)).toBeNull()
    // …and finds one when the same list also holds an answered row, so the null
    // above is the filter working rather than the function returning null for
    // everything.
    expect(lastDecision([...pending, ...answered])).toContain('promote_strategy')
  })

  it('reads the newest by when it was answered, not by list order', () => {
    expect(lastDecision([...answered].reverse())).toContain('promote_strategy')
  })
})

describe('the three things an empty "Needs you" can mean', () => {
  it('shows the last decision when the log was read', () => {
    render(<LastDecision text="promote_strategy → deny by mason" known />)
    expect(screen.getByText(/Last decision: promote_strategy → deny/)).toBeInTheDocument()
  })

  it('says the log could not be read rather than claiming nothing was decided', () => {
    // This is the defect. The old copy asserted "No decision has been recorded
    // on this desk yet" in exactly this case — a statement about the desk's
    // history produced by a component that had been handed `null` on purpose.
    render(<LastDecision text={null} known={false} />)
    expect(screen.getByText(/could not be read/)).toBeInTheDocument()
    expect(screen.queryByText(/No approval on this desk has been answered yet/)).toBeNull()
  })

  it('says nothing has been answered only when that is what the log showed', () => {
    render(<LastDecision text={null} known />)
    expect(screen.getByText(/No approval on this desk has been answered yet/)).toBeInTheDocument()
  })

  it('never renders a bare "No data" (Meridian §3.22)', () => {
    const { container } = render(<LastDecision text={null} known />)
    expect(container.textContent).not.toMatch(/No data/i)
    expect(screen.getByText('Nothing is waiting on you')).toBeInTheDocument()
  })
})
