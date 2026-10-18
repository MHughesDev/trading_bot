/**
 * S-6.15 — "an approval answered inline disappears from the global inbox in the
 * same event".
 *
 * Pending approvals are listed under two cache keys: the global inbox's
 * `['approvals', …]` and desk home's `['desk', 'approvals', …]`. An answer from
 * any surface invalidated at most its own key, so the inbox kept an answered
 * approval until its own 15-second poll.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import { describe, expect, it, vi } from 'vitest'

const approval = {
  approval_id: 'a_1',
  kind: 'spend_holdout_look',
  state: 'pending',
  created_at: '2026-10-03T09:00:00Z',
  payload: { summary: 'look at the holdout once' },
  options: ['approve', 'deny'],
}
const desk = vi.hoisted(() => ({
  approvals: vi.fn(),
  answerApproval: vi.fn(),
  awayDigest: vi.fn(),
  running: vi.fn(),
  monitor: vi.fn(),
  killSwitch: vi.fn(),
  proposals: vi.fn(),
  sessions: vi.fn(),
}))
const research = vi.hoisted(() => ({ approvals: vi.fn(), answerApproval: vi.fn() }))
const convo = vi.hoisted(() => ({ list: vi.fn(), listProfiles: vi.fn() }))
vi.mock('@/api/desk', async (orig) => ({ ...(await orig<object>()), deskApi: desk }))
vi.mock('@/api/research', async (orig) => ({ ...(await orig<object>()), researchApi: research }))
vi.mock('@/api/agent', async (orig) => ({ ...(await orig<object>()), conversationsApi: convo }))

import { ApprovalsInbox } from '../ApprovalsInbox'
import { DeskHome } from '../DeskHome'
import { APPROVAL_KEYS, invalidateApprovals } from '@/lib/desk/approvals'

describe('one answer, every list', () => {
  it('invalidates the inbox key and the desk key together', () => {
    const qc = new QueryClient()
    qc.setQueryData(['approvals', 'all'], [approval])
    qc.setQueryData(['desk', 'approvals', 'pending'], { approvals: [approval] })
    invalidateApprovals(qc)
    expect(qc.getQueryState(['approvals', 'all'])?.isInvalidated).toBe(true)
    expect(qc.getQueryState(['desk', 'approvals', 'pending'])?.isInvalidated).toBe(true)
    expect(APPROVAL_KEYS).toHaveLength(2)
  })

  it('an approval answered on desk home leaves the global inbox without waiting for its poll', async () => {
    let pending = true
    desk.approvals.mockImplementation(() =>
      Promise.resolve({ data: { approvals: pending ? [approval] : [], kinds: [] } }),
    )
    desk.answerApproval.mockImplementation(() => {
      pending = false
      return Promise.resolve({ data: {} })
    })
    research.approvals.mockImplementation(() => Promise.resolve({ data: { approvals: pending ? [approval] : [] } }))
    desk.awayDigest.mockResolvedValue({ absent: true, lines: [] })
    desk.running.mockResolvedValue({ runs: [], not_recorded: [] })
    desk.monitor.mockResolvedValue({ strategies: [] })
    desk.killSwitch.mockResolvedValue({ trading_enabled: true, kill_switch_active: false })
    desk.proposals.mockResolvedValue({ pending: [] })
    desk.sessions.mockResolvedValue({ sessions: [] })
    convo.list.mockResolvedValue({ data: { conversations: [] } })
    convo.listProfiles.mockResolvedValue({ data: { profiles: [] } })

    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={qc}>
        <MemoryRouter>
          <DeskHome />
          <section data-testid="inbox">
            <ApprovalsInbox />
          </section>
        </MemoryRouter>
      </QueryClientProvider>,
    )
    // The inbox shows it.
    await waitFor(() => expect(research.approvals).toHaveBeenCalled())
    await screen.findAllByText(/look at the holdout once|spend_holdout_look/)
    const before = research.approvals.mock.calls.length

    // Answered from desk home's Needs you — a different surface, a different key.
    fireEvent.click(screen.getAllByRole('button', { name: 'Approve' })[0])
    await waitFor(() => expect(desk.answerApproval).toHaveBeenCalledWith('a_1', 'approve'))
    // The inbox re-read on the answer, not on its 15-second poll, and is empty.
    await waitFor(() => expect(research.approvals.mock.calls.length).toBeGreaterThan(before))
    await waitFor(() =>
      expect(screen.getByTestId('inbox').textContent).toMatch(/Nothing is waiting on you/),
    )
  })
})
