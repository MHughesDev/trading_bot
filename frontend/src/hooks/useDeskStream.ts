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
  /** Seconds since the last event, while stale. */
  staleForSecs: number | null
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
  const [state, setState] = useState<StreamState>('connecting')
  const [error, setError] = useState<string | null>(null)
  const [cursor, setCursor] = useState<string | null>(null)
  const [staleForSecs, setStaleForSecs] = useState<number | null>(null)
  const [nonce, setNonce] = useState(0)

  const onEventRef = useRef(onEvent)
  onEventRef.current = onEvent
  const cursorRef = useRef<string | null>(null)
  const lastEventAt = useRef<number>(Date.now())

  const reconnect = useCallback(() => setNonce((n) => n + 1), [])

  useEffect(() => {
    if (!url) {
      setState('closed')
      return
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
        setStaleForSecs(null)
        onEventRef.current(parsed)
      } catch {
        // A malformed frame is dropped rather than killing the stream: losing one
        // event is recoverable, losing the connection loses the rest of the run.
      }
    }

    const run = async () => {
      while (!closed) {
        try {
          const token = getStoredToken()
          const headers: Record<string, string> = {
            Accept: 'text/event-stream',
            Authorization: token ? `Bearer ${token}` : 'Bearer dev-local',
          }
          if (cursorRef.current) headers['Last-Event-ID'] = cursorRef.current

          const res = await fetch(url, { headers, signal: controller.signal })
          if (!res.ok || !res.body) {
            if (res.status === 401 || res.status === 404) {
              // A permanent answer. Retrying turns it into a polling loop.
              setError(res.status === 401 ? 'not signed in' : 'this conversation is not available')
              setState('closed')
              return
            }
            throw new Error(`status ${res.status}`)
          }
          backoffMs = 1_000
          setState('live')
          setError(null)
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
          setError('the event stream dropped; reconnecting')
        }
        if (closed) return
        // Stale, not empty: the last state stays on screen with its age.
        setState('stale')
        setStaleForSecs(Math.round((Date.now() - lastEventAt.current) / 1000))
        await new Promise((r) => setTimeout(r, backoffMs))
        backoffMs = Math.min(backoffMs * 2, 15_000)
      }
    }

    void run()
    return () => {
      closed = true
      controller.abort()
    }
  }, [url, nonce])

  // Ages the stale marker once a second so "stale · 40s" is true rather than
  // frozen at the moment the socket dropped.
  useEffect(() => {
    if (state !== 'stale') return
    const t = setInterval(() => {
      setStaleForSecs(Math.round((Date.now() - lastEventAt.current) / 1000))
    }, 1_000)
    return () => clearInterval(t)
  }, [state])

  return { state, cursor, staleForSecs, error, reconnect }
}
