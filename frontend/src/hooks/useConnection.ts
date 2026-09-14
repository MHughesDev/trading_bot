import { useEffect, useRef, useState } from 'react'
import { getWsClient, wsBus } from '@/api/ws'
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
  const [state, setState] = useState<ConnectionState>(() =>
    getWsClient()?.isConnected() ? 'connected' : 'reconnecting',
  )
  const [latencyMs, setLatencyMs] = useState<number | null>(null)
  const [lastMessageAt, setLastMessageAt] = useState<number | null>(null)
  const beatRef = useRef<number | null>(null)

  useEffect(() => {
    // Re-check on mount too: a socket that opened between render and effect
    // would otherwise wait for the next heartbeat to be reported as live.
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
