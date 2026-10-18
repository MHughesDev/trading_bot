/**
 * S-7.10: "the desk banner reflects a trip within one event" — the client half.
 *
 * `GET /api/monitor/stream` was built and registered with no subscriber; the
 * desk kept polling. `MonitorNotifier` is the subscriber, mounted once in the
 * shell. These drive it through the hook's `onEvent` with the frames the server
 * sends and assert what a person would otherwise have to wait a poll for:
 *
 *   * a trip frame is on the Monitor page's live table the moment it arrives;
 *   * it is announced, and posts a notification through `notifyDecision`'s gate;
 *   * a DRILL announces marked DRILL and never posts a notification;
 *   * a blip is recorded and says nothing;
 *   * the stream's own state — `stale` before it ever connected — is printed
 *     beside the table, so an empty table is never read as a quiet platform;
 *   * the same event delivered twice announces once.
 */
import { act, render, screen, waitFor, within } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const h = vi.hoisted(() => ({
  onEvent: null as null | ((e: unknown) => void),
  stream: { state: 'live', cursor: null, staleForSecs: null, everReceived: true, error: null, reconnect: () => {} },
  announce: vi.fn(),
  deliver: vi.fn(),
  get: vi.fn(),
}))

vi.mock('@/hooks/useDeskStream', () => ({
  useDeskStream: (o: { url: string | null; onEvent: (e: unknown) => void }) => {
    h.onEvent = o.onEvent
    return h.stream
  },
}))
vi.mock('@/lib/announce', () => ({ announce: h.announce }))
vi.mock('@/lib/notify', async (orig) => ({
  ...(await orig<typeof import('@/lib/notify')>()),
  deliverNotification: h.deliver,
}))
vi.mock('@/lib/api', () => ({ api: { get: h.get, post: vi.fn() }, getStoredToken: () => 'tok' }))

import { MonitorNotifier } from '@/components/desk/MonitorNotifier'
import { MonitorLive } from '@/components/desk/MonitorLive'
import { MONITOR_STREAM } from '@/api/ops'

function mount() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return {
    qc,
    ...render(
      <QueryClientProvider client={qc}>
        <MonitorNotifier />
        <MonitorLive />
      </QueryClientProvider>,
    ),
  }
}

const ev = (over: Record<string, unknown> = {}) => ({
  event_id: 'e1',
  occurred_at: '2026-10-03T10:00:00Z',
  scope: 'global',
  family: 'lease',
  kind: 'tripped',
  metric: null,
  reason: 'heartbeat lost',
  actor: 'monitor',
  is_drill: false,
  ...over,
})

beforeEach(() => {
  h.announce.mockReset()
  h.deliver.mockReset()
  h.get.mockReset()
  // The measurement panels are not what is under test; they answer 404.
  h.get.mockRejectedValue({ response: { status: 404 } })
  h.stream = { state: 'live', cursor: null, staleForSecs: null, everReceived: true, error: null, reconnect: () => {} }
})

describe('one subscriber, on the monitor stream', () => {
  it('opens /api/monitor/stream and nothing else', () => {
    mount()
    expect(MONITOR_STREAM).toBe('/api/monitor/stream')
    expect(h.onEvent).toBeTypeOf('function')
  })

  it('puts a trip on the live table and announces it in the same frame', async () => {
    mount()
    act(() => h.onEvent?.(ev()))
    // react-query hands cache changes to observers on a macrotask; the
    // announcement above it is synchronous with the frame, which is the point.
    expect(h.announce).toHaveBeenCalledTimes(1)
    const table = await screen.findByTestId('monitor-live-events')
    expect(within(table).getByText('kill switch tripped')).toBeTruthy()
    expect(within(table).getByText('global')).toBeTruthy()
    expect(h.announce).toHaveBeenCalledWith(expect.stringMatching(/Kill switch tripped: platform-wide/))
    expect(h.deliver).toHaveBeenCalledTimes(1)
  })

  it('announces a replayed frame once', async () => {
    mount()
    act(() => h.onEvent?.(ev()))
    act(() => h.onEvent?.(ev()))
    expect(h.announce).toHaveBeenCalledTimes(1)
    const table = await screen.findByTestId('monitor-live-events')
    expect(within(table).getAllByRole('row')).toHaveLength(2) // header + one
  })

  it('announces a drill marked DRILL and posts no notification', async () => {
    mount()
    act(() => h.onEvent?.(ev({ event_id: 'd1', kind: 'drill', is_drill: true, scope: 'drill:weekly' })))
    expect(h.announce).toHaveBeenCalledWith(expect.stringMatching(/^DRILL:/))
    expect(h.deliver).not.toHaveBeenCalled()
    expect(await screen.findByText(/DRILL/, { selector: '.badge' })).toBeTruthy()
  })

  it('records a blip without saying anything', async () => {
    mount()
    act(() => h.onEvent?.(ev({ event_id: 'b1', kind: 'blip' })))
    expect(h.announce).not.toHaveBeenCalled()
    expect(h.deliver).not.toHaveBeenCalled()
    expect(within(await screen.findByTestId('monitor-live-events')).getByText('blip')).toBeTruthy()
  })

  it('shows the snapshot and a listener failure instead of swallowing them', async () => {
    mount()
    act(() => h.onEvent?.({ lease: 'held', orders_permitted: false, halted_scopes: ['global'] }))
    await waitFor(() => expect(screen.getByTestId('monitor-stream-words').textContent).toMatch(/halted: global/))
    expect(screen.getByTestId('monitor-stream-words').textContent).toMatch(/orders not permitted/)
    act(() => h.onEvent?.({ message: 'listener closed' }))
    expect((await screen.findByTestId('monitor-stream-error')).textContent).toMatch(/listener closed/)
  })

  it('says the stream has not connected, rather than leaving a quiet table', async () => {
    h.stream = { ...h.stream, state: 'stale', everReceived: false, staleForSecs: null }
    mount()
    await waitFor(() => expect(screen.getByTestId('monitor-stream-state').textContent).toBe('stale'))
    expect(screen.getByTestId('monitor-stream-words').textContent).toMatch(/has not connected yet/)
  })

  it('a frame it cannot read changes nothing', async () => {
    mount()
    act(() => h.onEvent?.({ nonsense: true }))
    expect(h.announce).not.toHaveBeenCalled()
    expect(await screen.findByText(/No event has arrived on this connection/)).toBeTruthy()
  })
})
