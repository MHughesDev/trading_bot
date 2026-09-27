/**
 * A conversation whose stream never delivered anything showed the badge a
 * perfectly fresh one shows (AGENT-012 §14, §16; Set S task S-6.2).
 *
 * The header rendered `<StaleBadge age={`${stream.staleForSecs ?? 0}s`} />`, and
 * `staleForSecs` was measured from `lastEventAt`, which is initialised to
 * `Date.now()` **at mount** and only moved by a frame arriving. So a stream that
 * dropped before its first frame produced "0s old" — and an age is a claim that
 * there is something that old. There was nothing. §14's rule for a disconnected
 * desk is "last known state stays, marked stale with age"; with no last known
 * state there is no age to mark, and the `??` made the two readings identical.
 *
 * The hook now reports `staleForSecs: null` until something has arrived, and
 * says separately whether anything ever did.
 */
import { renderHook, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { useDeskStream, parseFrame } from '@/hooks/useDeskStream'
import { staleAge } from '@/components/desk/Conversation.helpers'

vi.mock('@/lib/api', () => ({ getStoredToken: () => 'test-token' }))

/**
 * Built by interpolation rather than written out.
 *
 * `the_desk_calls_routes_that_exist` scans every path literal under
 * `frontend/src`, `.tsx` included, and normalises an interpolation to `{}` — so
 * a session id written out in full would not match the served
 * `/api/agent/sessions/{session_id}/events` and would read as a call to a route
 * nobody serves. A fixture id is a parameter; spelling it out would make a test
 * fixture look like a forward reference.
 */
const eventsUrl = (id: string) => `/api/agent/sessions/${id}/events`

const realFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = realFetch
  vi.restoreAllMocks()
})

/** An SSE body that yields the given frames and then ends. */
function bodyOf(frames: string[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder()
  return new ReadableStream({
    start(controller) {
      for (const f of frames) controller.enqueue(encoder.encode(`${f}\n\n`))
      controller.close()
    },
  })
}

describe('a stream that dropped before its first frame has no age', () => {
  it('reports null rather than a number measured from mount', async () => {
    globalThis.fetch = vi.fn(() => Promise.reject(new Error('socket refused'))) as typeof fetch

    const { result } = renderHook(() =>
      useDeskStream({ url: eventsUrl('s_1'), onEvent: () => {} }),
    )

    await waitFor(() => expect(result.current.state).toBe('stale'))
    // The `?? 0` in the header turned exactly this into "0s old".
    expect(result.current.staleForSecs).toBeNull()
    expect(result.current.everReceived).toBe(false)
  })

  it('reports a real age once a frame has arrived, which is the other half', async () => {
    const seen: unknown[] = []
    globalThis.fetch = vi.fn(() =>
      Promise.resolve({
        ok: true,
        status: 200,
        body: bodyOf(['id: 7\ndata: {"kind":"status"}']),
      }),
    ) as unknown as typeof fetch

    const { result } = renderHook(() =>
      useDeskStream({
        url: eventsUrl('s_2'),
        onEvent: (e) => seen.push(e),
      }),
    )

    // The frame lands, then the body ends and the loop marks the stream stale.
    await waitFor(() => expect(seen).toHaveLength(1))
    await waitFor(() => expect(result.current.state).toBe('stale'))
    expect(result.current.everReceived).toBe(true)
    expect(typeof result.current.staleForSecs).toBe('number')
    expect(result.current.cursor).toBe('7')
  })
})

describe('the frame parser the ages rest on', () => {
  it('keeps the id and concatenates the data lines', () => {
    expect(parseFrame('id: 12\ndata: {"a":1}')).toEqual({ id: '12', data: '{"a":1}' })
  })

  it('reports no id when the frame carries none', () => {
    expect(parseFrame('data: {}').id).toBeNull()
  })
})

describe('the label the header draws from it', () => {
  it('gives no age at all when nothing was ever received', () => {
    expect(staleAge(null)).toBeNull()
  })

  it('gives a real age when there is one, including a genuine zero', () => {
    // A stream that dropped a moment after a frame really is 0s old, and that
    // reading has to survive: the fix is not "never print zero", it is "never
    // print zero for something nobody measured".
    expect(staleAge(0)).toBe('0s')
    expect(staleAge(41)).toBe('41s')
  })

  it('refuses a non-finite age rather than printing NaNs', () => {
    expect(staleAge(Number.NaN)).toBeNull()
  })
})
