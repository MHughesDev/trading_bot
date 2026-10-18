/**
 * S-7.10 (surfaces): feeds, venues, coverage, defects and drill runs on the
 * Monitor page.
 *
 * Each is a place a failed read could have rendered as "nothing is wrong":
 *   * an empty feed table beside deployments that are running reads as fresh data;
 *   * an empty defect list reads as a clean platform;
 *   * a drill never run reads as a drill that passed.
 * So each panel is asserted in its unread state as well as its read one, and the
 * two controls (resolve a defect, run the drills) are asserted to send what the
 * server requires and to print what it refuses with.
 */
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({ get: vi.fn(), post: vi.fn() }))
vi.mock('@/lib/api', () => ({ api: { get: h.get, post: h.post }, getStoredToken: () => 'tok' }))

import { MonitorLive } from '@/components/desk/MonitorLive'

function serve(map: Record<string, unknown | (() => Promise<unknown>)>) {
  h.get.mockImplementation((url: string) => {
    const v = map[url]
    if (v === undefined) return Promise.reject({ response: { status: 404 } })
    return typeof v === 'function' ? (v as () => Promise<unknown>)() : Promise.resolve({ data: v })
  })
}

function mount() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MonitorLive />
    </QueryClientProvider>,
  )
}

beforeEach(() => {
  h.get.mockReset()
  h.post.mockReset()
})

describe('feeds', () => {
  it('prints the age and verdict of each feed, and names a deployment nothing watches', async () => {
    serve({
      '/api/monitor/feeds': {
        unwatched_deployments: ['orphan-strat'],
        feeds: [
          { symbol: 'BTC-USD', venue: 'coinbase', timeframe_secs: 60, last_bar_at: '2026-10-03T10:00:00Z', age_secs: 4000, session_open: null, affirmed_at: null, verdict: 'hard_stale', strategies: ['s1'] },
          { symbol: 'ETH-USD', venue: 'coinbase', timeframe_secs: 60, last_bar_at: '2026-10-03T10:00:00Z', age_secs: 20, session_open: null, affirmed_at: null, verdict: 'fresh', strategies: [] },
        ],
      },
    })
    mount()
    const table = await screen.findByTestId('feeds-table')
    expect(within(table).getByText('stale')).toBeTruthy()
    expect(within(table).getByText('67 min')).toBeTruthy()
    expect(within(table).getByText('no strategy')).toBeTruthy()
    expect(screen.getByTestId('unwatched-deployments').textContent).toMatch(/orphan-strat/)
  })

  it('an empty feed table says the monitor is not measuring, not that the data is fresh', async () => {
    serve({ '/api/monitor/feeds': { unwatched_deployments: [], feeds: [] } })
    mount()
    expect(await screen.findByText('No feed is gauged')).toBeTruthy()
    expect(screen.getByText(/not the data being fresh/)).toBeTruthy()
  })

  it('a feeds route that fails is unknown, not empty', async () => {
    serve({ '/api/monitor/feeds': () => Promise.reject({ response: { status: 500, data: { error: 'monitor_unavailable' } } }) })
    mount()
    expect(await screen.findByText('Could not read the feed ages')).toBeTruthy()
    expect(screen.queryByText('No feed is gauged')).toBeNull()
  })
})

describe('venues and coverage', () => {
  it('shows a venue verdict that is not clean in a non-green tone', async () => {
    serve({
      '/api/monitor/venues': {
        all_venues_reconciled: false,
        venues: [{ venue: 'kraken', verdict: 'mismatch', instruments_checked: 4, detail: 'ETH short by 0.2', checked_by: 'monitor', checked_at: '2026-10-03T10:00:00Z' }],
      },
    })
    mount()
    const t = await screen.findByTestId('venues-table')
    expect(within(t).getByText('mismatch').className).toMatch(/neg/)
    expect(within(t).getByText('ETH short by 0.2')).toBeTruthy()
    expect(screen.getByText('not all reconciled')).toBeTruthy()
  })

  it('names a deployment that is measured and never judged', async () => {
    serve({
      '/api/monitor/coverage': {
        deployments: [
          { strategy_id: 's1', instrument_id: 'BTC-USD', environment: 'paper', started_at: null, declared_families: [], judged: false, promotion_gate: { allowed: false, reason: 'no limits declared' }, feed_gauged: false },
        ],
      },
    })
    mount()
    const t = await screen.findByTestId('coverage-table')
    expect(within(t).getByText('measured, never judged')).toBeTruthy()
    expect(within(t).getByText('no gauge')).toBeTruthy()
    expect(within(t).getAllByText('no limits declared').length).toBeGreaterThan(0)
  })
})

describe('defects', () => {
  const defects = { defects: [{ defect_id: 'd1', source: 'fleet', summary: 'common-mode divergence', evidence: {}, filed_by: 'job', filed_at: '2026-10-03T08:00:00Z', status: 'open' }] }

  it('resolving needs a sentence and sends it', async () => {
    serve({ '/api/monitor/defects': defects })
    h.post.mockResolvedValue({ data: { defect_id: 'd1', status: 'resolved' } })
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Resolve' }))
    const dialog = await screen.findByRole('dialog')
    const submit = within(dialog).getByRole('button', { name: 'Resolve, as me' }) as HTMLButtonElement
    expect(submit.disabled).toBe(true)
    fireEvent.change(within(dialog).getByRole('textbox'), { target: { value: 'replayed; slippage model corrected' } })
    expect(submit.disabled).toBe(false)
    fireEvent.click(submit)
    await waitFor(() =>
      expect(h.post).toHaveBeenCalledWith(`/api/monitor/defects/${'d1'}/resolve`, { resolution: 'replayed; slippage model corrected' }),
    )
  })

  it('prints the refusal of an agent or service token', async () => {
    serve({ '/api/monitor/defects': defects })
    h.post.mockRejectedValue({ response: { status: 403, data: { error: 'monitor_control_required', detail: 'requires a human session' } } })
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Resolve' }))
    const dialog = await screen.findByRole('dialog')
    fireEvent.change(within(dialog).getByRole('textbox'), { target: { value: 'done' } })
    fireEvent.click(within(dialog).getByRole('button', { name: 'Resolve, as me' }))
    expect((await screen.findByTestId('resolve-defect-error')).textContent).toMatch(/requires a human session/)
  })

  it('an empty list is only empty when the route said so', async () => {
    serve({ '/api/monitor/defects': { defects: [] } })
    mount()
    expect(await screen.findByText('No open defect')).toBeTruthy()
  })
})

describe('drills', () => {
  it('a drill never run is "never", not a pass', async () => {
    serve({ '/api/monitor/drill-runs': { suite_due: true, interval_days: 7, drills: [{ drill: 'lease_loss', last_run_at: null, passed: null }] } })
    mount()
    const t = await screen.findByTestId('drills-table')
    expect(within(t).getByText('never')).toBeTruthy()
    expect(within(t).getByText('not recorded')).toBeTruthy()
    expect(screen.getByText('suite due')).toBeTruthy()
  })

  it('runs the four on demand and reports a failure with its reason', async () => {
    serve({ '/api/monitor/drill-runs': { suite_due: false, interval_days: 7, drills: [] } })
    h.post.mockResolvedValue({
      data: { passed: false, drills: [{ run_id: 'r1', drill: 'lease_loss', passed: false, duration_ms: 12, failure: 'order accepted while halted', defect_id: null }] },
    })
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Run the four now' }))
    await waitFor(() => expect(h.post).toHaveBeenCalledWith('/api/monitor/drill-runs'))
    expect((await screen.findByTestId('drill-run-result')).textContent).toMatch(/order accepted while halted/)
  })

  it('a refusal to run is printed', async () => {
    serve({ '/api/monitor/drill-runs': { suite_due: false, interval_days: 7, drills: [] } })
    h.post.mockRejectedValue({ response: { status: 409, data: { error: 'not_paper', detail: 'drills run only against paper' } } })
    mount()
    fireEvent.click(await screen.findByRole('button', { name: 'Run the four now' }))
    expect((await screen.findByTestId('drill-run-error')).textContent).toMatch(/drills run only against paper/)
  })
})
