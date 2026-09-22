/**
 * "Not signed in" and "the connection failed" are different facts, and the user
 * can act on one of them.
 *
 * With `initWsClient(getStoredToken() ?? 'dev-local')` removed, a browser with
 * no token has no `WsClient`. `useConnection` seeded its state from
 * `getWsClient()?.isConnected()`, so a missing client fell through to
 * `'reconnecting'` — and the banner then said "Reconnecting to market data…",
 * which is a claim that a retry is in flight. Nothing was retrying, and nothing
 * could: a reconnect cannot mint a credential.
 *
 * `signed-out` sits beside the other four connection states rather than inside
 * `disconnected` for the same reason `staleForSecs` is `null` rather than `0`
 * on a stream that never connected — the other four all describe a socket that
 * exists.
 */
import { render, renderHook, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useConnection } from '@/hooks/useConnection'
import { DisconnectedBanner } from '@/components/primitives/States'
import { setStoredToken, clearStoredToken } from '@/lib/api'

vi.mock('@/api/ws', () => ({
  // No socket exists in either case here; what differs is whether the browser
  // holds a credential that could open one.
  getWsClient: () => null,
  wsBus: { on: () => () => {} },
}))

beforeEach(() => clearStoredToken())
afterEach(() => {
  clearStoredToken()
  vi.restoreAllMocks()
})

describe('a browser with no session token reports signed-out, not reconnecting', () => {
  it('does not describe an absent credential as a retry in flight', () => {
    const { result } = renderHook(() => useConnection())
    // Before: 'reconnecting'.
    expect(result.current.state).toBe('signed-out')
  })

  it('still reports reconnecting when there IS a token and no open socket', () => {
    // The other half: a real connection failure must keep reading as one.
    // The fix is not "never say reconnecting", it is "never say reconnecting
    // for a browser that has nothing to reconnect with".
    setStoredToken('a-real-session-token')
    const { result } = renderHook(() => useConnection())
    expect(result.current.state).toBe('reconnecting')
  })

  it('claims no latency and no last message, because neither was measured', () => {
    const { result } = renderHook(() => useConnection())
    expect(result.current.latencyMs).toBeNull()
    expect(result.current.lastMessageAt).toBeNull()
  })
})

describe('the banner names the absent credential and offers the act that fixes it', () => {
  it('says the session is missing rather than that market data dropped', () => {
    render(<DisconnectedBanner state="signed-out" />)
    const banner = screen.getByTestId('connection-signed-out')
    expect(banner).toHaveTextContent(/no signed-in session/i)
    expect(banner).not.toHaveTextContent(/Reconnecting to market data/i)
    // A retry is the wrong verb for this state; signing in is the right one.
    expect(screen.getByRole('button', { name: /sign in/i })).toBeInTheDocument()
  })

  it('leaves the four socket states saying exactly what they said before', () => {
    const { rerender } = render(<DisconnectedBanner state="reconnecting" />)
    expect(screen.getByTestId('connection-banner')).toHaveTextContent(/Reconnecting to market data/i)

    rerender(<DisconnectedBanner state="disconnected" />)
    expect(screen.getByTestId('connection-banner')).toHaveTextContent(/Market data disconnected/i)

    rerender(<DisconnectedBanner state="degraded" />)
    expect(screen.getByTestId('connection-banner')).toHaveTextContent(/Market data delayed/i)

    rerender(<DisconnectedBanner state="connected" />)
    expect(screen.queryByTestId('connection-banner')).not.toBeInTheDocument()
  })

  it('offers "Reconnect now" only where reconnecting is a thing that can work', () => {
    const onRetry = vi.fn()
    const { rerender } = render(<DisconnectedBanner state="disconnected" onRetry={onRetry} />)
    expect(screen.getByRole('button', { name: /reconnect now/i })).toBeInTheDocument()

    rerender(<DisconnectedBanner state="signed-out" onRetry={onRetry} />)
    expect(screen.queryByRole('button', { name: /reconnect now/i })).not.toBeInTheDocument()
  })
})
