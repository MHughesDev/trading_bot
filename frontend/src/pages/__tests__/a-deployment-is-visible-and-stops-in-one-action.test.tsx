/**
 * T-1.3: the Deployments page. "Every deployment is listed with its verdict and
 * divergence state; stopping one halts its submission; the stop-all action is
 * the kill switch's" and T-1.1's "the UI must send proposal_id".
 *
 * Asserted here is what the server would otherwise be left to refuse:
 *   * a row carries its verdict, divergence state and review-period countdown,
 *     and "not measured" is not drawn as "no halts";
 *   * a deployment without a proposal behind it carries the legacy marker;
 *   * the revoke button posts to that deployment's revoke route;
 *   * stop-all is dead until a reason is typed, and says it trips the kill switch;
 *   * an approved, undeployed proposal is deployed with its `proposal_id` in the
 *     body, and a pending one is decided through the decision route;
 *   * a list that did not load is "unknown", never "nothing is deployed".
 */
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({ get: vi.fn(), post: vi.fn() }))
vi.mock('@/lib/api', () => ({
  api: { get: h.get, post: h.post },
  getStoredToken: () => 'tok',
}))

import { DeploymentsPage } from '@/pages/DeploymentsPage'

const DAY = 86_400_000
const overview = [
  {
    deployment_id: 'd-1',
    strategy_name: 'btc-trend',
    revision: 'abcdef0123456789',
    instrument_id: 'BTC-USD',
    environment: 'paper',
    venue: 'sim',
    starting_capital: '10000',
    started_at: '2026-09-30T00:00:00Z',
    stopped_at: null,
    stop_reason: null,
    running: true,
    proposal_id: 'p-0',
    profile_id: 'strict_v1',
    gates_passed: 15,
    verdict: Array.from({ length: 16 }, (_, i) => ({ gate_no: i + 1, passed: i + 1 !== 15 })),
    legacy_ungated: false,
    size_halvings: 1,
    review_period_no: 1,
    review_period_ends_at: new Date(Date.now() + 5 * DAY).toISOString(),
    divergence_halts: 2,
    realised_vs_expected: [{ bar: '2026-10-01T00:00:00Z', realised: 0.002, expected: 0.001 }],
  },
  {
    deployment_id: 'd-2',
    strategy_name: 'eth-old',
    revision: 'ffff',
    instrument_id: 'ETH-USD',
    environment: 'paper',
    venue: 'sim',
    starting_capital: '5000',
    started_at: '2026-08-01T00:00:00Z',
    stopped_at: null,
    stop_reason: null,
    running: true,
    proposal_id: null,
    profile_id: null,
    gates_passed: null,
    verdict: null,
    legacy_ungated: true,
    size_halvings: 0,
    review_period_no: null,
    review_period_ends_at: null,
    divergence_halts: null,
    realised_vs_expected: [],
  },
]
const proposals = [
  {
    proposal_id: 'p-pending',
    strategy_name: 'sol-mom',
    revision: 'aaaabbbbcc',
    instrument_id: 'SOL-USD',
    profile_id: 'strict_v1',
    gates_passed: 16,
    starting_capital: '2500',
    review_period_days: 14,
    proposed_by_kind: 'agent',
    created_at: '2026-10-03T00:00:00Z',
    decision: 'pending',
    decided_by: null,
    decided_at: null,
    envelope: null,
    deployment_id: null,
  },
  {
    proposal_id: 'p-approved',
    strategy_name: 'ada-rev',
    revision: 'ddddeeeeff',
    instrument_id: 'ADA-USD',
    profile_id: 'strict_v1',
    gates_passed: 16,
    starting_capital: '1000',
    review_period_days: 14,
    proposed_by_kind: 'agent',
    created_at: '2026-10-02T00:00:00Z',
    decision: 'approved',
    decided_by: 'human:u1',
    decided_at: '2026-10-03T00:00:00Z',
    envelope: {},
    deployment_id: null,
  },
]

function routes(over: Record<string, () => Promise<unknown>> = {}) {
  h.get.mockImplementation((url: string) => {
    if (over[url]) return over[url]()
    if (url === '/api/deployment-overview') return Promise.resolve({ data: overview })
    if (url === '/api/deployment-proposals') return Promise.resolve({ data: proposals })
    if (url === '/api/deployment-ceilings')
      return Promise.resolve({ data: { declared: [], undeclared_refuses_every_deployment: true } })
    return Promise.reject({ response: { status: 404 } })
  })
}

function mount() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <DeploymentsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  h.get.mockReset()
  h.post.mockReset()
  h.post.mockResolvedValue({ data: {} })
})

describe('the deployment list', () => {
  it('shows each deployment with its verdict, divergence state and review countdown', async () => {
    routes()
    mount()
    const row = await screen.findByTestId('deployment-d-1')
    expect(within(row).getByTestId('verdict').textContent).toBe('15 of 16 gates passed')
    expect(within(row).getByTestId('failed-gates').textContent).toMatch(/gate 15/)
    expect(within(row).getByTestId('divergence').textContent).toBe('2 halts')
    expect(row.textContent).toMatch(/1\/2 size/)
    expect(row.textContent).toMatch(/period 1: ends in 5 d/)
  })

  it('marks a deployment with no proposal behind it, and says "not measured" for no recorder', async () => {
    routes()
    mount()
    const row = await screen.findByTestId('deployment-d-2')
    expect(within(row).getByTestId('legacy-ungated')).toBeInTheDocument()
    expect(within(row).getByTestId('divergence').textContent).toBe('not measured')
    expect(within(row).getByTestId('verdict').textContent).toBe('no gate verdict')
    expect(within(screen.getByTestId('deployment-d-1')).queryByTestId('legacy-ungated')).toBeNull()
  })

  it('is unknown, not empty, when the overview did not load', async () => {
    routes({ '/api/deployment-overview': () => Promise.reject({ response: { status: 500, data: { message: 'db down' } } }) })
    mount()
    expect(await screen.findByText(/the deployments is unknown, not empty/)).toBeInTheDocument()
    expect(screen.queryByText('Nothing is deployed')).toBeNull()
  })
})

describe('stopping', () => {
  it('posts to that deployment’s revoke route', async () => {
    routes()
    mount()
    const row = await screen.findByTestId('deployment-d-1')
    fireEvent.click(within(row).getByRole('button', { name: /Stop…/ }))
    fireEvent.click(await screen.findByRole('button', { name: 'Stop it now' }))
    await waitFor(() => expect(h.post).toHaveBeenCalledWith('/api/deployments/d-1/revoke'))
  })

  it('stop-all is dead until a reason is given, and sends it', async () => {
    routes()
    mount()
    await screen.findByTestId('deployment-d-1')
    fireEvent.click(screen.getByRole('button', { name: /Stop everything/ }))
    const go = await screen.findByRole('button', { name: /Trip the kill switch and stop all/ })
    expect(go).toBeDisabled()
    expect(screen.getByText(/same action as the monitor/)).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText(/Why/), { target: { value: 'feed is wrong' } })
    expect(go).not.toBeDisabled()
    fireEvent.click(go)
    await waitFor(() =>
      expect(h.post).toHaveBeenCalledWith('/api/deployment-stop-all', { reason: 'feed is wrong' }),
    )
  })

  it('prints the server’s refusal of a stop-all (403) and keeps the dialog open', async () => {
    routes()
    h.post.mockRejectedValueOnce({ response: { status: 403, data: { message: 'a person does' } } })
    mount()
    await screen.findByTestId('deployment-d-1')
    fireEvent.click(screen.getByRole('button', { name: /Stop everything/ }))
    fireEvent.change(await screen.findByLabelText(/Why/), { target: { value: 'x' } })
    fireEvent.click(screen.getByRole('button', { name: /Trip the kill switch and stop all/ }))
    expect((await screen.findByTestId('stop-all-error')).textContent).toMatch(/a person does/)
  })
})

describe('proposals', () => {
  it('deploys an approved proposal with its proposal_id in the body', async () => {
    routes()
    mount()
    const row = await screen.findByTestId('proposal-p-approved')
    fireEvent.click(within(row).getByRole('button', { name: /Deploy under this approval/ }))
    await waitFor(() =>
      expect(h.post).toHaveBeenCalledWith('/api/deployments', {
        strategy_ref: 'ada-rev',
        instrument_id: 'ADA-USD',
        starting_capital: '1000',
        proposal_id: 'p-approved',
      }),
    )
  })

  it('decides a pending proposal through the decision route, and a decided one has no decide button', async () => {
    routes()
    mount()
    const pending = await screen.findByTestId('proposal-p-pending')
    expect(within(screen.getByTestId('proposal-p-approved')).queryByRole('button', { name: /Decide/ })).toBeNull()
    fireEvent.click(within(pending).getByRole('button', { name: /Decide…/ }))
    fireEvent.change(await screen.findByLabelText(/Note/), { target: { value: 'looks right' } })
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }))
    await waitFor(() =>
      expect(h.post).toHaveBeenCalledWith('/api/deployment-proposals/p-pending/decision', {
        decision: 'approved',
        note: 'looks right',
      }),
    )
  })

  it('says an undeclared ceiling refuses every deployment', async () => {
    routes()
    mount()
    expect(await screen.findByText(/refuses every deployment/)).toBeInTheDocument()
  })
})
