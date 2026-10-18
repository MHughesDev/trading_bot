/**
 * T-0.3 / T-2.2: the Reconciliation page. "The halt is visible in the UI with
 * its reason" and "clearing it requires an explicit human action that is
 * recorded".
 *
 * What is asserted is the wiring the server would refuse on, none of which a
 * person can see from a 400:
 *   * the reason is on the screen beside the strategy it halted;
 *   * the clear button is dead until the strategy id is typed exactly AND a note
 *     is given, and the POST carries exactly those two fields;
 *   * a refusal (409) is printed, and the row is not removed locally;
 *   * a halt list that did not load is "unknown, not empty" — the sentence a
 *     healthy platform's empty list must not be confused with.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({ get: vi.fn(), post: vi.fn() }))
vi.mock('@/lib/api', () => ({
  api: { get: h.get, post: h.post },
  getStoredToken: () => 'tok',
}))

import { ReconciliationPage } from '@/pages/ReconciliationPage'

const halts = {
  blocked_orders: 4,
  halts: [
    {
      halt_id: 'h1',
      strategy_id: 'btc-trend-v2',
      kind: 'position',
      reason: 'internal position 3 vs broker 2',
      raised_at: '2026-10-03T09:00:00Z',
      in_force: true,
      persisted: true,
    },
  ],
}
const fleet = {
  window: { from: '2026-09-03T00:00:00Z', to: '2026-10-03T00:00:00Z', days: 30 },
  policy: { tolerance_per_bar: 0.0005, quorum: 3, min_bars: 30 },
  deployments: [],
  aggregate: null,
  verdict: 'not_computable',
  detail: 'no deployment has forward observations',
  defect_id: null,
}

function routes(over: Record<string, () => Promise<unknown>> = {}) {
  h.get.mockImplementation((url: string) => {
    if (over[url]) return over[url]()
    if (url === '/api/reconciliation/halts') return Promise.resolve({ data: halts })
    if (url === '/api/reconciliation/fleet') return Promise.resolve({ data: fleet })
    return Promise.reject({ response: { status: 404 } })
  })
}

function mount() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <ReconciliationPage />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  h.get.mockReset()
  h.post.mockReset()
})

describe('the halt list', () => {
  it('shows the halt with its reason and standing', async () => {
    routes()
    mount()
    expect(await screen.findByText('internal position 3 vs broker 2')).toBeTruthy()
    expect(screen.getByText('btc-trend-v2')).toBeTruthy()
    expect(screen.getByText('in force')).toBeTruthy()
    expect(screen.getByTestId('halt-count').textContent).toBe('1 in force')
  })

  it('says unknown, not empty, when the list could not be read', async () => {
    routes({ '/api/reconciliation/halts': () => Promise.reject({ response: { status: 500, data: { error: 'boom' } } }) })
    mount()
    expect(await screen.findByText(/Could not read the halt list/)).toBeTruthy()
    expect(screen.getByText(/the halt list is unknown, not empty/i)).toBeTruthy()
    expect(screen.queryByText(/No strategy is halted/)).toBeNull()
  })

  it('says the route is missing, not that nothing is halted', async () => {
    routes({ '/api/reconciliation/halts': () => Promise.reject({ response: { status: 404 } }) })
    mount()
    expect(await screen.findByText(/is not served by this build/)).toBeTruthy()
    expect(screen.queryByText(/No strategy is halted/)).toBeNull()
  })

  it('says an empty list is empty only when the route answered it', async () => {
    routes({ '/api/reconciliation/halts': () => Promise.resolve({ data: { blocked_orders: 0, halts: [] } }) })
    mount()
    expect(await screen.findByText(/No strategy is halted for divergence/)).toBeTruthy()
  })
})

describe('clearing a halt', () => {
  async function open() {
    routes()
    mount()
    fireEvent.click(await screen.findByRole('button', { name: /Clear…/ }))
    return screen.findByRole('dialog')
  }
  const clearButton = () => screen.getByRole('button', { name: 'Clear this halt' }) as HTMLButtonElement

  it('is dead until the id is typed exactly and a note is given', async () => {
    await open()
    expect(screen.getByTestId('clear-halt-reason').textContent).toMatch(/internal position 3 vs broker 2/)
    expect(clearButton().disabled).toBe(true)

    const [idBox, noteBox] = screen.getAllByRole('textbox')
    fireEvent.change(idBox, { target: { value: 'BTC-TREND-V2' } })
    fireEvent.change(noteBox, { target: { value: 'broker confirmed 3' } })
    // Right letters, wrong case: still a different strategy.
    expect(clearButton().disabled).toBe(true)
    expect(screen.getByTestId('clear-halt-why').textContent).toMatch(/character for character/)

    fireEvent.change(idBox, { target: { value: 'btc-trend-v2' } })
    expect(clearButton().disabled).toBe(false)
    fireEvent.change(noteBox, { target: { value: '   ' } })
    expect(clearButton().disabled).toBe(true)
    expect(h.post).not.toHaveBeenCalled()
  })

  it('posts exactly the confirmation and the note, and re-reads the list', async () => {
    await open()
    h.post.mockResolvedValue({ data: { halt_id: 'h1', strategy_id: 'btc-trend-v2', cleared_by: 'human:u', cleared_at: 'x', halt_reason: 'r' } })
    const [idBox, noteBox] = screen.getAllByRole('textbox')
    fireEvent.change(idBox, { target: { value: 'btc-trend-v2' } })
    fireEvent.change(noteBox, { target: { value: ' broker confirmed 3 ' } })
    fireEvent.click(clearButton())
    await waitFor(() => expect(h.post).toHaveBeenCalledTimes(1))
    expect(h.post).toHaveBeenCalledWith('/api/reconciliation/halts/btc-trend-v2/clear', {
      confirm_strategy: 'btc-trend-v2',
      note: 'broker confirmed 3',
    })
    // The halts query ran again: the registry decides, not the client.
    await waitFor(() => expect(h.get.mock.calls.filter(([u]) => u === '/api/reconciliation/halts').length).toBeGreaterThan(1))
  })

  it('prints the server refusal and keeps the dialog open', async () => {
    await open()
    h.post.mockRejectedValue({ response: { status: 409, data: { error: 'clear_refused', detail: 'the position still diverges' } } })
    const [idBox, noteBox] = screen.getAllByRole('textbox')
    fireEvent.change(idBox, { target: { value: 'btc-trend-v2' } })
    fireEvent.change(noteBox, { target: { value: 'looks fine' } })
    fireEvent.click(clearButton())
    expect((await screen.findByTestId('clear-halt-error')).textContent).toMatch(/the position still diverges/)
    expect(screen.getByRole('dialog')).toBeTruthy()
  })
})

describe('the fleet panel', () => {
  it('renders not computable as that, never as no divergence', async () => {
    routes()
    mount()
    const v = await screen.findByTestId('fleet-verdict')
    expect(v.textContent).toMatch(/Not computable/)
    expect(v.textContent).not.toMatch(/No divergence/)
  })
})
