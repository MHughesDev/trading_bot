// One event stream for the desk (AGENT-012 §16; task S-6.2).
//
// This copies the pattern that already works in `api/research.ts`: a hand-rolled
// `fetch` SSE client rather than `EventSource`, because the platform
// authenticates with a bearer token and `EventSource` cannot set a request
// header. The alternative — the token in the query string — puts a credential in
// server logs, proxy logs and browser history every time anyone opens a
// conversation.
//
// What this adds over `openTimeline`: an event cursor the caller can read, and a
// `stale` state instead of a blank screen. A disconnected desk that blanks is
// worse than useless during a long run, because the operator cannot tell a
// finished run from a dropped socket.
import { useCallback, useEffect, useRef, useState } from 'react'
import { getStoredToken } from '@/lib/api'
import type { WireEvent } from '@/lib/desk/events'

export type StreamState = 'connecting' | 'live' | 'stale' | 'closed'

export interface DeskStream {
  state: StreamState
  /** The last event id the server sent, resumed from on reconnect. */
  cursor: string | null
  /**
   * Seconds since the last event, while stale.
   *
   * `null` when the stream dropped **before it ever delivered an event**, and
   * the caller must not print it as a number. `lastEventAt` starts at mount, so
   * folding the two together produced "3s old" over a conversation whose state
   * had never arrived at all — an age is a claim that there is something that
   * old, and there was not.
   */
  staleForSecs: number | null
  /** Whether any event has ever arrived on this stream. */
  everReceived: boolean
  error: string | null
  /** Forces a reconnect, for the "retry" affordance on a closed stream. */
  reconnect: () => void
}

interface Options {
  /** Absolute path of the SSE endpoint, or null to stay closed. */
  url: string | null
  onEvent: (e: WireEvent) => void
}

/** Splits an SSE frame into its `id:` and concatenated `data:` lines. */
export function parseFrame(frame: string): { id: string | null; data: string } {
  let id: string | null = null
  let data = ''
  for (const line of frame.split('\n')) {
    if (line.startsWith('id:')) id = line.slice(3).trim()
    else if (line.startsWith('data:')) data += line.slice(5).trim()
  }
  return { id, data }
}

export function useDeskStream({ url, onEvent }: Options): DeskStream {
  // The reading is stored with the url it was taken from, so it cannot be read
  // as a reading about a different stream. `state` used to be a bare value that
  // an effect reset to `'closed'` when the url went away, which meant the render
  // that first saw a *new* url still reported the previous stream's state.
  const [socket, setSocket] = useState<{ url: string | null; state: StreamState }>({
    url: null,
    state: 'connecting',
  })
  const [socketError, setSocketError] = useState<string | null>(null)
  const [cursor, setCursor] = useState<string | null>(null)
  const [staleForSecs, setStaleForSecs] = useState<number | null>(null)
  const [everReceived, setEverReceived] = useState(false)
  const [nonce, setNonce] = useState(0)

  // An absent credential is a permanent answer and a *render-time* one: nothing
  // can mint a token without a reload, so this reads the same on every render of
  // this component's life. `closed` rather than `stale` for the reason the effect
  // below used to give -- reconnecting cannot produce a token, so a retry loop
  // here is a poll against a fact.
  const signedOut = !getStoredToken()
  const state: StreamState =
    !url || signedOut ? 'closed' : socket.url === url ? socket.state : 'connecting'
  const error = url && signedOut ? 'not signed in' : socketError

  const onEventRef = useRef(onEvent)
  onEventRef.current = onEvent
  const cursorRef = useRef<string | null>(null)
  const lastEventAt = useRef<number>(Date.now())
  const everReceivedRef = useRef(false)
  const urlOfStream = useRef<string | null>(null)

  const reconnect = useCallback(() => setNonce((n) => n + 1), [])

  useEffect(() => {
    // No url, nothing to open. `state` already reads `'closed'` from `url` alone,
    // so there is nothing to set here.
    if (!url) return
    const setState = (next: StreamState) => setSocket({ url, state: next })
    // A different conversation is a different stream: what the previous one had
    // received is not evidence about this one. Keyed on the url rather than on
    // this effect running, because a manual reconnect re-runs it and resumes the
    // *same* stream from its cursor — the events it already delivered are still
    // on the screen, so it has not stopped having received them.
    if (urlOfStream.current !== url) {
      urlOfStream.current = url
      everReceivedRef.current = false
      setEverReceived(false)
      setStaleForSecs(null)
      lastEventAt.current = Date.now()
    }
    const controller = new AbortController()
    let closed = false
    let backoffMs = 1_000
    setState('connecting')

    const handleFrame = (frame: string) => {
      const { id, data } = parseFrame(frame)
      if (id) {
        cursorRef.current = id
        setCursor(id)
      }
      if (!data) return
      try {
        const parsed = JSON.parse(data) as WireEvent
        lastEventAt.current = Date.now()
        everReceivedRef.current = true
        setEverReceived(true)
        setStaleForSecs(null)
        onEventRef.current(parsed)
      } catch {
        // A malformed frame is dropped rather than killing the stream: losing one
        // event is recoverable, losing the connection loses the rest of the run.
      }
    }

    // An absent credential is a permanent answer, and the cheapest place to give
    // it is before the first request -- sending `Bearer dev-local` asked the
    // platform a question it has always answered 401, and then reported that 401
    // as though a real credential had been refused. Both readings it produces are
    // derived above, from the token itself, so there is nothing to set here.
    if (signedOut) return () => {}

    const run = async () => {
      while (!closed) {
        try {
          const token = getStoredToken()
          const headers: Record<string, string> = {
            Accept: 'text/event-stream',
            ...(token ? { Authorization: `Bearer ${token}` } : {}),
          }
          if (cursorRef.current) headers['Last-Event-ID'] = cursorRef.current

          const res = await fetch(url, { headers, signal: controller.signal })
          if (!res.ok || !res.body) {
            if (res.status === 401 || res.status === 404) {
              // A permanent answer. Retrying turns it into a polling loop.
              setSocketError(res.status === 401 ? 'not signed in' : 'this conversation is not available')
              setState('closed')
              return
            }
            throw new Error(`status ${res.status}`)
          }
          backoffMs = 1_000
          setState('live')
          setSocketError(null)
          const reader = res.body.getReader()
          const decoder = new TextDecoder()
          let buffer = ''
          for (;;) {
            const { done, value } = await reader.read()
            if (done) break
            buffer += decoder.decode(value, { stream: true })
            let split = buffer.indexOf('\n\n')
            while (split !== -1) {
              handleFrame(buffer.slice(0, split))
              buffer = buffer.slice(split + 2)
              split = buffer.indexOf('\n\n')
            }
          }
        } catch (e) {
          if (closed || (e as Error).name === 'AbortError') return
          setSocketError('the event stream dropped; reconnecting')
        }
        if (closed) return
        // Stale, not empty: the last state stays on screen with its age — and
        // only when there is a last state. A stream that dropped before its
        // first frame has nothing to age.
        setState('stale')
        setStaleForSecs(
          everReceivedRef.current ? Math.round((Date.now() - lastEventAt.current) / 1000) : null,
        )
        await new Promise((r) => setTimeout(r, backoffMs))
        backoffMs = Math.min(backoffMs * 2, 15_000)
      }
    }

    void run()
    return () => {
      closed = true
      controller.abort()
    }
  }, [url, nonce, signedOut])

  // Ages the stale marker once a second so "stale · 40s" is true rather than
  // frozen at the moment the socket dropped.
  useEffect(() => {
    if (state !== 'stale') return
    if (!everReceivedRef.current) return
    const t = setInterval(() => {
      setStaleForSecs(Math.round((Date.now() - lastEventAt.current) / 1000))
    }, 1_000)
    return () => clearInterval(t)
  }, [state])

  return { state, cursor, staleForSecs, everReceived, error, reconnect }
}
