/**
 * S-6.21 — "when the kill switch is tripped the banner appears on every route
 * within one event and cannot be dismissed while tripped" (§19.6).
 *
 * The banner was a three-second poll and said so: nothing pushed kill-switch
 * state to a browser. `GET /api/desk/kill-switch/events` now holds a `LISTEN`
 * on `monitor_kill_switch` and sends the halted scopes on every change, and the
 * banner applies a global halt from the frame itself — no refetch between the
 * event and the screen.
 */
import { act, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import { describe, expect, it, vi } from 'vitest'

const stream = vi.hoisted(() => ({ onEvent: null as null | ((e: unknown) => void), url: null as string | null }))
vi.mock('@/hooks/useDeskStream', () => ({
  useDeskStream: (o: { url: string | null; onEvent: (e: unknown) => void }) => {
    stream.onEvent = o.onEvent
    stream.url = o.url
    return { state: 'live', cursor: null, staleForSecs: null, everReceived: true, error: null, reconnect: () => {} }
  },
}))
const killSwitch = vi.hoisted(() => vi.fn())
vi.mock('@/api/desk', async (orig) => ({ ...(await orig<object>()), deskApi: { killSwitch } }))

import { KillSwitchBanner } from '../KillSwitchBanner'
import { KILL_SWITCH_QUERY_KEY, KILL_SWITCH_STREAM, applyKillSwitchFrame } from '../KillSwitchBanner.helpers'

describe('a frame from the monitor', () => {
  it('a global halt is applied from the frame, with no refetch in between', () => {
    const r = applyKillSwitchFrame(
      { trading_enabled: true, kill_switch_active: false },
      { readable: true, global_halted: true, halted_scopes: ['global'] },
    )
    expect(r.refetch).toBe(false)
    expect(r.next).toMatchObject({ kill_switch_active: true, trading_enabled: false, scope: 'global' })
  })

  it('a reset, a strategy-scope halt or an unreadable frame asks the status route instead', () => {
    for (const content of [
      { readable: true, global_halted: false, halted_scopes: [] },
      { readable: true, global_halted: false, halted_scopes: ['strategy:s1'] },
      { readable: false, message: 'permission denied' },
    ]) {
      expect(applyKillSwitchFrame({ trading_enabled: false, kill_switch_active: true }, content)).toEqual({
        next: null,
        refetch: true,
      })
    }
  })
})

describe('the banner listens', () => {
  it('subscribes to the push route and shows the halt on the frame, before any poll', async () => {
    killSwitch.mockResolvedValue({ trading_enabled: true, kill_switch_active: false })
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={qc}>
        <MemoryRouter>
          <KillSwitchBanner />
        </MemoryRouter>
      </QueryClientProvider>,
    )
    expect(stream.url).toBe(KILL_SWITCH_STREAM)
    // Armed: nothing drawn.
    await waitFor(() => expect(qc.getQueryState(KILL_SWITCH_QUERY_KEY)?.status).toBe('success'))
    expect(screen.queryByTestId('kill-switch-banner')).toBeNull()
    const calls = killSwitch.mock.calls.length

    act(() => {
      stream.onEvent?.({
        seq: 2,
        kind: 'kill_switch',
        content: { readable: true, global_halted: true, halted_scopes: ['global'], cause: 'global' },
        created_at: '2026-10-03T10:00:00Z',
      })
    })
    // Within one event: on screen, and the status route was not asked again.
    // (React Query notifies observers on its own tick, so this awaits a render,
    // not a request — the call count below is the part that says no request.)
    expect(await screen.findByTestId('kill-switch-banner')).toBeInTheDocument()
    expect(killSwitch.mock.calls.length).toBe(calls)
    // And nothing on it dismisses it.
    expect(screen.queryByRole('button')).toBeNull()
  })
})
