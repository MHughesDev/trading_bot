/**
 * Four places invented a credential when the browser had none.
 *
 * `getStoredToken() ?? 'dev-local'` (and the header spelling
 * `token ? \`Bearer ${token}\` : 'Bearer dev-local'`) turned "this browser is not
 * signed in" into "this browser presented a token the server rejected". Those
 * are different facts with different fixes, and only one of them is true.
 *
 * It used to be *almost* harmless on the WebSocket lanes, because `/ws/live`
 * took the token string as the user id, so `dev-local` opened a real socket and
 * received a real feed. That is fixed server-side: `BearerToken` now resolves
 * the token against the `sessions` table and refuses anything that is not a
 * row. The REST routes never accepted it — verified against the running
 * platform on :7080, where `Authorization: Bearer dev-local` answers 401 on
 * `/api/settings`, `/api/strategies`, `/api/alerts` and
 * `/api/agent/sessions/{id}/events`.
 *
 * So the fallbacks' only remaining effect was to spend a request, or a
 * handshake, on a guaranteed refusal, and then report that refusal through the
 * same path a dropped connection uses.
 */
import { renderHook, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { api, setStoredToken, clearStoredToken } from '@/lib/api'
import { useSuiteProgress } from '@/hooks/useSuiteProgress'
import { useDeskStream } from '@/hooks/useDeskStream'
import { openTimeline } from '@/api/research'

const realFetch = globalThis.fetch
const realWebSocket = globalThis.WebSocket
const realAdapter = api.defaults.adapter

/** Built by interpolation — see the note in `a-stream-that-never-connected-is-not-fresh`. */
const eventsUrl = (id: string) => `/api/agent/sessions/${id}/events`

beforeEach(() => {
  clearStoredToken()
})

afterEach(() => {
  globalThis.fetch = realFetch
  globalThis.WebSocket = realWebSocket
  api.defaults.adapter = realAdapter
  clearStoredToken()
  vi.restoreAllMocks()
})

// ── The axios interceptor every REST call goes through ───────────────────────

describe('a REST call from a signed-out browser carries no Authorization header', () => {
  /** Captures the config the interceptor chain produced, without a network. */
  function captureHeaders() {
    const seen: Record<string, unknown>[] = []
    api.defaults.adapter = ((config: { headers: Record<string, unknown> }) => {
      seen.push(config.headers)
      return Promise.resolve({ data: {}, status: 200, statusText: 'OK', headers: {}, config })
    }) as never
    return seen
  }

  it('sends no header at all rather than a token the platform has never accepted', async () => {
    const seen = captureHeaders()
    await api.get('/api/settings')
    // Before: `Bearer dev-local`. A 401 either way, but one of them claims a
    // credential was offered.
    expect(seen[0].Authorization).toBeUndefined()
  })

  it('still sends the real token when there is one, which is the other half', async () => {
    setStoredToken('a-real-session-token')
    const seen = captureHeaders()
    await api.get('/api/settings')
    expect(seen[0].Authorization).toBe('Bearer a-real-session-token')
  })

  it('never puts the literal dev-local on the wire under any condition', async () => {
    const seen = captureHeaders()
    await api.get('/api/settings')
    setStoredToken('t')
    await api.get('/api/settings')
    expect(JSON.stringify(seen)).not.toContain('dev-local')
  })
})

// ── The backtest-suite progress lane ─────────────────────────────────────────

describe('the suite-progress lane opens no socket without a token', () => {
  function stubWebSocket() {
    const ctor = vi.fn((_url: string) => ({ close: vi.fn() }))
    globalThis.WebSocket = ctor as unknown as typeof WebSocket
    return ctor
  }

  it('does not hand the handshake an invented user id', () => {
    const ctor = stubWebSocket()
    renderHook(() => useSuiteProgress())
    // Before: one construction, with `?token=dev-local`.
    expect(ctor).not.toHaveBeenCalled()
  })

  it('opens with the real token when there is one', () => {
    setStoredToken('a-real-session-token')
    const ctor = stubWebSocket()
    renderHook(() => useSuiteProgress())
    expect(ctor).toHaveBeenCalledTimes(1)
    expect(String(ctor.mock.calls[0][0])).toContain('token=a-real-session-token')
  })

  it('reports no progress frame, which is what it reported before anyway', () => {
    stubWebSocket()
    const { result } = renderHook(() => useSuiteProgress())
    // Nothing regresses on the console: the lane was already delivering nothing
    // and `BackTestingPage` polls for the truth. `null` already means "no frame".
    expect(result.current).toBeNull()
  })
})

// ── The desk event stream ────────────────────────────────────────────────────

describe('the desk stream spends no request on a credential it does not have', () => {
  it('reports not-signed-in without asking the server', async () => {
    const fetchSpy = vi.fn((_url: string, _init?: RequestInit) => Promise.reject(new Error('refused')))
    globalThis.fetch = fetchSpy as unknown as typeof fetch

    const { result } = renderHook(() =>
      useDeskStream({ url: eventsUrl('s_1'), onEvent: () => {} }),
    )

    await waitFor(() => expect(result.current.state).toBe('closed'))
    // `closed`, not `stale`: a retry cannot mint a token, so a reconnect loop
    // here would be a poll against a permanent answer.
    expect(result.current.error).toBe('not signed in')
    // Before: one fetch carrying `Bearer dev-local`, whose 401 produced this
    // same pair — by a round trip, and only after the server said so.
    expect(fetchSpy).not.toHaveBeenCalled()
  })

  it('sends the real token and no fallback when there is one', async () => {
    setStoredToken('a-real-session-token')
    const fetchSpy = vi.fn((_url: string, _init?: RequestInit) => Promise.reject(new Error('refused')))
    globalThis.fetch = fetchSpy as unknown as typeof fetch

    renderHook(() => useDeskStream({ url: eventsUrl('s_2'), onEvent: () => {} }))

    await waitFor(() => expect(fetchSpy).toHaveBeenCalled())
    const headers = fetchSpy.mock.calls[0][1]?.headers as Record<string, string>
    expect(headers.Authorization).toBe('Bearer a-real-session-token')
  })
})

// ── The research timeline ────────────────────────────────────────────────────

describe('the research timeline says why it did not open', () => {
  it('reports not-signed-in rather than fetching with an invented bearer', async () => {
    const fetchSpy = vi.fn((_url: string, _init?: RequestInit) => Promise.reject(new Error('refused')))
    globalThis.fetch = fetchSpy as unknown as typeof fetch

    const reasons: string[] = []
    const close = openTimeline('s_3', () => {}, (r) => reasons.push(r))

    await waitFor(() => expect(reasons).toEqual(['not signed in']))
    expect(fetchSpy).not.toHaveBeenCalled()
    close()
  })
})
