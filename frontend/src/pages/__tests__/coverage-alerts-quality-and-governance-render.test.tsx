/**
 * P-0.2, FB-RX-003, FB-RX-004, FB-RX-005: the four read surfaces render what the
 * routes served, in the words the acceptance names.
 *
 *   * Coverage: thinnest first, depth in words, forward series with the age of
 *     their last row, and a research project's 403 stated rather than drawn as
 *     an empty census.
 *   * Execution quality: "No fills recorded yet", and an unmeasured statistic is
 *     never a number.
 *   * Alerts: a pinned trip has no live dismiss control; acknowledge and dismiss
 *     POST to the server and the inbox is re-read.
 *   * Governance: a rollback needs the confirm step before anything is sent.
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

import { CoveragePage } from '@/pages/CoveragePage'
import { ExecutionQualityPage } from '@/pages/ExecutionQualityPage'
import { AlertsPage } from '@/pages/AlertsPage'
import { GovernancePage } from '@/pages/GovernancePage'
import { AlertsBell } from '@/components/ops/AlertsBell'
import { ExecQualityCard } from '@/components/ops/ExecQualityCard'

function mount(ui: React.ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>{ui}</MemoryRouter>
    </QueryClientProvider>,
  )
}

function serve(map: Record<string, unknown | (() => Promise<unknown>)>) {
  h.get.mockImplementation((url: string) => {
    const v = map[url]
    if (v === undefined) return Promise.reject({ response: { status: 404 } })
    return typeof v === 'function' ? (v as () => Promise<unknown>)() : Promise.resolve({ data: v })
  })
}

beforeEach(() => {
  h.get.mockReset()
  h.post.mockReset()
})

const row = (instrument: string, days: number, over: Record<string, unknown> = {}) => ({
  instrument,
  venue: 'coinbase',
  timeframe: '1m',
  series_kind: 'ohlcv',
  as_of: '2026-10-03T00:00:00Z',
  first_bar: null,
  last_bar: null,
  expected_bars: 10,
  present_bars: 10,
  missing_bps: 150,
  continuous_days: days,
  derived_knowledge_bars: null,
  observed_knowledge_bars: null,
  last_row_age_secs: 120,
  accruing: true,
  strict_v1: { satisfied: false, shortfall_days: 1700, permits: `strict short for ${instrument}` },
  paper_v1: { satisfied: days >= 365, shortfall_days: 0, permits: `paper note for ${instrument}` },
  ...over,
})

describe('the coverage page', () => {
  const census = (over: Record<string, unknown> = {}) => ({
    census_as_of: '2026-10-03T00:00:00Z',
    census_taken: true,
    floors: { strict_v1_days: 1825, paper_v1_days: 365 },
    series: [row('DEEP', 900), row('THIN', 12, { accruing: false, last_row_age_secs: 86400 * 9 }), row('MID', 200)],
    forward: {
      measured: true,
      series: [
        { series: 'funding', venue: 'binance', instrument: 'BTC-PERP', last_event_time: 't', last_write_time: null, last_row_age_secs: 30, rows_written_last_beat: 1 },
        { series: 'oi', venue: 'binance', instrument: 'ETH-PERP', last_event_time: 't', last_write_time: null, last_row_age_secs: 86400 * 3, rows_written_last_beat: null },
      ],
    },
    ...over,
  })

  it('lists the thinnest series first, states depth in words and the server sentences', async () => {
    serve({ '/api/data/coverage': census() })
    mount(<CoveragePage />)
    const table = await screen.findByTestId('coverage-census')
    const rows = within(table).getAllByRole('row').slice(1)
    expect(rows.map((r) => r.textContent?.slice(0, 4))).toEqual(['THIN', 'MID ', 'DEEP'])
    expect(table.textContent).toMatch(/12 days of continuous history/)
    expect(table.textContent).toMatch(/strict short for THIN/)
    expect(table.textContent).toMatch(/not accruing/)
    expect(screen.getByTestId('census-summary').textContent).toMatch(/3 series/)
  })

  it('shows the age of every forward series, the stalest first', async () => {
    serve({ '/api/data/coverage': census() })
    mount(<CoveragePage />)
    const fw = await screen.findByTestId('forward-series')
    const rows = within(fw).getAllByRole('row').slice(1)
    expect(rows[0].textContent).toMatch(/oi/)
    expect(rows[0].textContent).toMatch(/3 days ago/)
    expect(rows[1].textContent).toMatch(/30 s ago/)
  })

  it('an unmeasured forward table is a missing measurement, not an empty list', async () => {
    serve({ '/api/data/coverage': census({ forward: { measured: false, reason: 'no heartbeat table', series: null } }) })
    mount(<CoveragePage />)
    expect(await screen.findByText(/Forward series are not measured/)).toBeTruthy()
    expect(screen.getByText(/no heartbeat table/)).toBeTruthy()
    expect(screen.queryByText(/No forward series has written/)).toBeNull()
  })

  it('no census taken is not a platform that holds nothing', async () => {
    serve({ '/api/data/coverage': census({ census_taken: false, series: [], census_as_of: null }) })
    mount(<CoveragePage />)
    expect(await screen.findByText(/No census has been taken/)).toBeTruthy()
    expect(screen.queryByTestId('coverage-census')).toBeNull()
  })

  it('states the desk-only refusal', async () => {
    serve({
      '/api/data/coverage': () =>
        Promise.reject({ response: { status: 403, data: { error: 'desk_only', message: 're-issue this request against your Desk project' } } }),
    })
    mount(<CoveragePage />)
    expect(await screen.findByText(/may not read the coverage census/)).toBeTruthy()
    expect(screen.getByText(/Desk project/)).toBeTruthy()
  })
})

describe('the execution quality page', () => {
  const measured = (v: number) => ({ status: 'measured', value: v })
  const unmeasured = (reason: string) => ({ status: 'unmeasured', reason })

  it('says no fills recorded yet when nothing filled', async () => {
    serve({
      '/api/execution/quality': {
        window_days: 30,
        rows: [{ strategy_id: 's', instrument_id: 'BTC-USD', orders: 2, fill_count: 0, fill_ratio: measured(0), slippage_bps_p50: unmeasured('r'), slippage_bps_p95: unmeasured('r'), submit_to_fill_ms_p50: unmeasured('r'), submit_to_fill_ms_p95: unmeasured('r') }],
      },
    })
    mount(<ExecutionQualityPage />)
    expect(await screen.findByText('No fills recorded yet')).toBeTruthy()
  })

  it('prints unmeasured slippage as words and names it once', async () => {
    serve({
      '/api/execution/quality': {
        window_days: 30,
        rows: [{ strategy_id: 's', instrument_id: 'BTC-USD', orders: 2, fill_count: 2, fill_ratio: measured(0.5), slippage_bps_p50: unmeasured('nothing records the prediction'), slippage_bps_p95: unmeasured('nothing records the prediction'), submit_to_fill_ms_p50: measured(1500), submit_to_fill_ms_p95: measured(20) }],
      },
    })
    mount(<ExecutionQualityPage />)
    const t = await screen.findByTestId('exec-quality-table')
    expect(t.textContent).toMatch(/50\.0%/)
    expect(t.textContent).toMatch(/not measured \/ not measured/)
    expect(t.textContent).not.toMatch(/0\.0 bps/)
    expect(screen.getByTestId('exec-unproduced').textContent).toMatch(/slippage versus the fill model/)
    expect(t.textContent).toMatch(/1\.50 s \/ 20 ms/)
  })

  it('does not draw a failed read as no fills', async () => {
    serve({ '/api/execution/quality': () => Promise.reject({ response: { status: 500 } }) })
    mount(<ExecutionQualityPage />)
    expect(await screen.findByText(/Could not read execution quality/)).toBeTruthy()
    expect(screen.queryByText('No fills recorded yet')).toBeNull()
  })
})

describe('the execution quality card under the terminal chart', () => {
  const m = (v: number) => ({ status: 'measured', value: v })
  const u = { status: 'unmeasured', reason: 'no prediction' }
  const r = (instrument: string, fills: number) => ({
    strategy_id: 'trend',
    instrument_id: instrument,
    orders: 2,
    fill_count: fills,
    fill_ratio: m(1),
    slippage_bps_p50: u,
    slippage_bps_p95: u,
    submit_to_fill_ms_p50: fills ? m(80) : u,
    submit_to_fill_ms_p95: fills ? m(90) : u,
  })

  it('shows only the instrument on screen', async () => {
    serve({ '/api/execution/quality': { window_days: 30, rows: [r('BTC-USD', 3), r('ETH-USD', 5)] } })
    mount(<ExecQualityCard instrument="BTC-USD" />)
    const card = await screen.findByTestId('exec-quality-card')
    await waitFor(() => expect(card.textContent).toMatch(/100\.0%/))
    expect(card.textContent).toMatch(/80 ms/)
    expect(card.textContent).toMatch(/not measured/)
    expect(card.querySelectorAll('tbody tr')).toHaveLength(1)
  })

  it('says no fills recorded yet for an instrument that has none', async () => {
    serve({ '/api/execution/quality': { window_days: 30, rows: [r('BTC-USD', 3), r('SOL-USD', 0)] } })
    mount(<ExecQualityCard instrument="SOL-USD" />)
    expect(await screen.findByText('No fills recorded yet')).toBeTruthy()
  })
})

describe('the alert inbox', () => {
  const item = (over: Record<string, unknown>) => ({
    source: 'monitor',
    id: 'a1',
    at: '2026-10-03T10:00:00Z',
    severity: 'warning',
    state: 'unread',
    title: 'breach — strategy:s1',
    detail: null,
    dismissible: true,
    pinned_reason: null,
    ...over,
  })

  it('a trip still in force is critical and cannot be dismissed from the page', async () => {
    serve({
      '/api/alert-inbox': {
        unread: 2,
        items: [
          item({ id: 'trip', severity: 'critical', title: 'tripped — global', dismissible: false, pinned_reason: 'the kill switch is still tripped for this scope' }),
          item({ id: 'br' }),
        ],
      },
    })
    mount(<AlertsPage />)
    const trip = await screen.findByTestId('alert-monitor-trip')
    expect(within(trip).getByText('critical')).toBeTruthy()
    expect(within(trip).getByRole('button', { name: 'Dismiss' }).hasAttribute('disabled')).toBe(true)
    expect(within(trip).getByTestId('alert-pinned').textContent).toMatch(/still tripped/)
    // The ordinary one can be.
    expect(within(screen.getByTestId('alert-monitor-br')).getByRole('button', { name: 'Dismiss' }).hasAttribute('disabled')).toBe(false)
  })

  it('posts the dismiss to the server and re-reads the inbox', async () => {
    serve({ '/api/alert-inbox': { unread: 1, items: [item({})] } })
    h.post.mockResolvedValue({ data: {} })
    mount(<AlertsPage />)
    fireEvent.click(await screen.findByRole('button', { name: 'Dismiss' }))
    await waitFor(() => expect(h.post).toHaveBeenCalledWith('/api/alert-inbox/monitor/a1/dismiss'))
    await waitFor(() => expect(h.get.mock.calls.filter(([u]) => u === '/api/alert-inbox').length).toBeGreaterThan(1))
  })

  it('shows the server refusal of a dismiss', async () => {
    serve({ '/api/alert-inbox': { unread: 1, items: [item({})] } })
    h.post.mockRejectedValue({ response: { status: 409, data: { error: 'pinned', detail: 'a trip cannot be dismissed while it is in force' } } })
    mount(<AlertsPage />)
    fireEvent.click(await screen.findByRole('button', { name: 'Dismiss' }))
    expect(await screen.findByText(/a trip cannot be dismissed while it is in force/)).toBeTruthy()
  })

  it('an empty inbox is a fact; a missing route is not', async () => {
    serve({ '/api/alert-inbox': { unread: 0, items: [] } })
    mount(<AlertsPage />)
    expect(await screen.findByText('Nothing needs you')).toBeTruthy()
  })

  it('the bell carries the unread count and says so when the inbox cannot be read', async () => {
    serve({ '/api/alert-inbox': { unread: 3, items: [item({ severity: 'critical' })] } })
    const first = mount(<AlertsBell />)
    expect((await screen.findByTestId('alerts-unread')).textContent).toMatch(/^3/)
    expect(screen.getByTestId('alerts-bell').getAttribute('aria-label')).toMatch(/3 unread, including a critical one/)
    first.unmount()

    serve({})
    mount(<AlertsBell />)
    await waitFor(() => expect(screen.getByTestId('alerts-bell').getAttribute('aria-label')).toMatch(/could not be read/))
    expect(screen.queryByTestId('alerts-unread')).toBeNull()
  })
})

describe('governance', () => {
  it('moves no alias until the rollback is confirmed', async () => {
    serve({
      '/api/models': { models: [{ model_id: 'm1', slug: 'ego', display_name: 'EgoGemma', model_kind: 'forecaster', status: 'active' }] },
      '/api/models/m1/aliases': { production: 14, candidate: 15 },
    })
    h.post.mockResolvedValue({ data: {} })
    mount(<GovernancePage />)
    fireEvent.click(screen.getByRole('radio', { name: 'Model aliases' }))
    const rollButtons = await screen.findAllByRole('button', { name: /Roll back…/ })
    fireEvent.click(rollButtons[0])
    const dialog = await screen.findByRole('dialog')
    expect(dialog.textContent).toMatch(/Moves the "production" alias of EgoGemma off version 14/)
    expect(h.post).not.toHaveBeenCalled()
    fireEvent.click(within(dialog).getByRole('button', { name: 'Roll back' }))
    await waitFor(() => expect(h.post).toHaveBeenCalledWith('/api/models/m1/aliases/production/rollback'))
  })

  it('states that there is no config history rather than drawing a diff of nothing', async () => {
    serve({})
    mount(<GovernancePage />)
    fireEvent.click(screen.getByRole('radio', { name: 'Config' }))
    expect(await screen.findByText(/No config history is recorded server-side/)).toBeTruthy()
  })

  it('reads the gate stack for the subject typed', async () => {
    serve({
      '/api/platform/health': { tenant: 't', generated_at: 'x', window_days: 30, signals: [] },
      '/api/platform/gates/exp-9': { subject: 'exp-9', recorded: 12, of: 16, passed: 12, complete: false, blocked_at: null, profile_id: 'p', verdicts: [] },
    })
    mount(<GovernancePage />)
    fireEvent.change(screen.getByPlaceholderText(/The id a run or proposal names/), { target: { value: 'exp-9' } })
    fireEvent.click(screen.getByRole('button', { name: 'Show gates' }))
    expect((await screen.findByTestId('gate-summary')).textContent).toMatch(/incomplete/)
  })

  it('lists the timeline with the denied count', async () => {
    serve({
      '/api/platform/timeline': {
        prevented: 1,
        limit: 200,
        rows: [{ kind: 'audit', at: '2026-10-03T10:00:00Z', what: 'place_order', phase: 'pre', verdict: 'denied', envelope: null, actor_kind: 'agent', actor_id: 'a1', on_behalf_of: null, propensity: null, exploration_flag: null, candidate_count: null, detail: {} }],
      },
    })
    mount(<GovernancePage />)
    fireEvent.click(screen.getByRole('radio', { name: 'Timeline' }))
    expect((await screen.findByTestId('timeline-prevented')).textContent).toMatch(/1 of the last 1 rows/)
  })
})
