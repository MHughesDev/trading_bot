import { useEffect, useRef, useState } from 'react'
import { getWsClient, wsBus } from '@/api/ws'
import { getStoredToken } from '@/lib/api'
import type { ConnectionState } from '@/components/primitives/States'

/* Spec §5.2 — connection states.
   connected (pos dot + latency) · degraded (warn + "delayed") ·
   reconnecting (warn, animated) · disconnected (neg + persistent banner). */

export interface ConnectionInfo {
  state: ConnectionState
  /** Round-trip latency in ms, from heartbeat timing. null until measured. */
  latencyMs: number | null
  /** Epoch ms of the last frame or heartbeat received. */
  lastMessageAt: number | null
}

export function useConnection(): ConnectionInfo {
  // Seed from the client's CURRENT state, not from events seen since mount.
  // The socket is a module singleton that usually connected long before this
  // hook mounted, so an event-only view reports "reconnecting" forever on any
  // screen opened after the first one.
  //
  // A browser with no session token is the one case where there is no client
  // and never will be until someone signs in. That used to seed 'reconnecting',
  // which is a claim that a retry is in flight; nothing was retrying, because
  // `initWsClient` was handed the string 'dev-local' and the live lane refuses
  // it. Reported as its own state so the banner can say so.
  const [state, setState] = useState<ConnectionState>(() =>
    !getStoredToken()
      ? 'signed-out'
      : getWsClient()?.isConnected()
        ? 'connected'
        : 'reconnecting',
  )
  const [latencyMs, setLatencyMs] = useState<number | null>(null)
  const [lastMessageAt, setLastMessageAt] = useState<number | null>(null)
  const beatRef = useRef<number | null>(null)

  useEffect(() => {
    // Nothing can move a signed-out reading except signing in, which reloads.
    // Staying off the bus keeps a stray control frame from some other panel
    // from repainting "no credential" as "connected".
    if (!getStoredToken()) return
    // Re-check on mount too: a socket that opened between render and effect would
    // otherwise wait for the next heartbeat to be reported as live.
    //
    // This is the subscription case the rule's own message carves out, and it
    // cannot be derived. The ws client is a mutable external object with no
    // snapshot to read during render, and the whole point of this line is to close
    // the window *between* render and effect -- a value computed during render is
    // by definition on the wrong side of it. The updater returns `s` unchanged when
    // it is already `connected`, so this cascades exactly once, on the mount where
    // the socket beat us to it.
    // eslint-disable-next-line react-hooks/set-state-in-effect -- reads the ws client's live state at subscription time; there is no render-time snapshot of it.
    if (getWsClient()?.isConnected()) setState((s) => (s === 'connected' ? s : 'connected'))
    const off = wsBus.on((msg) => {
      const now = Date.now()
      // The bus carries both server frames and locally-synthesised control
      // messages ('connected' | 'disconnected' | 'error'); widen to match.
      switch (msg.type as string) {
        case 'connected':
          setState('connected')
          beatRef.current = now
          break
        case 'disconnected':
          setState('disconnected')
          break
        case 'error':
          setState('degraded')
          break
        case 'heartbeat': {
          setState('connected')
          setLastMessageAt(now)
          if (beatRef.current) setLatencyMs(Math.max(0, Math.min(9999, now - beatRef.current - 1000)))
          beatRef.current = now
          break
        }
        default:
          setLastMessageAt(now)
          setState((s) => (s === 'disconnected' ? 'reconnecting' : s === 'connected' ? s : 'connected'))
      }
    })
    return off
  }, [])

  // Degrade to "degraded" if nothing has arrived for a while while nominally up.
  useEffect(() => {
    const t = window.setInterval(() => {
      setState((s) => {
        if (s !== 'connected') return s
        if (lastMessageAt && Date.now() - lastMessageAt > 30_000) return 'degraded'
        return s
      })
    }, 5000)
    return () => window.clearInterval(t)
  }, [lastMessageAt])

  return { state, latencyMs, lastMessageAt }
}

/**
 * A quote is stale at 2s for a streaming feed and 15s for a polled one
 * (spec §5.2). Returns null when the figure is fresh.
 */
export function staleAge(lastUpdateMs: number | null | undefined, budgetMs = 2000, now = Date.now()): number | null {
  if (!lastUpdateMs) return null
  const age = now - lastUpdateMs
  return age > budgetMs ? age : null
}
