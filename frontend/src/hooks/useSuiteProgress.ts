// Subscribes to the backtest-suite WS progress lane (/ws/backtest-suite) and
// surfaces the latest progress frame. Frames are already user-scoped server-side.
// On each frame the caller can refresh the relevant queries (the console polls as
// a fallback, so this is a liveness signal, not the source of truth).
import { useEffect, useRef, useState } from 'react'
import { getStoredToken } from '@/lib/api'

export interface SuiteProgress {
  experiment_id: string
  phase: string
  progress: number
  detail: string
  ts: string
}

export function useSuiteProgress(onProgress?: (p: SuiteProgress) => void): SuiteProgress | null {
  const [latest, setLatest] = useState<SuiteProgress | null>(null)
  // Refreshed in an effect rather than during render: a discarded render must
  // not be able to leave its callback behind for the socket to call. The only
  // reader is `onmessage`, which runs after commit, so nothing about the timing
  // the caller observes changes.
  const cb = useRef(onProgress)
  useEffect(() => {
    cb.current = onProgress
  })

  useEffect(() => {
    // No token, no socket. `?? 'dev-local'` used to open one anyway: the lane
    // resolves its query token against the `sessions` table and refuses
    // anything that is not a row, so the only thing the fallback produced was a
    // socket that opened and closed for reasons the console could not name.
    // There is nothing to display in its place — this hook is a liveness
    // signal, and the console's own polling is the source of truth — so the
    // return stays `null`, which already means "no frame".
    const token = getStoredToken()
    if (!token) return
    const proto = window.location.protocol === 'https:' ? 'wss' : 'ws'
    const url = `${proto}://${window.location.host}/ws/backtest-suite?token=${encodeURIComponent(token)}`
    let ws: WebSocket | null = null
    let closed = false

    try {
      ws = new WebSocket(url)
    } catch {
      return
    }

    ws.onmessage = (ev) => {
      try {
        const msg = JSON.parse(ev.data as string)
        if (msg.type === 'progress' && msg.payload) {
          const p = msg.payload as SuiteProgress
          if (!closed) {
            setLatest(p)
            cb.current?.(p)
          }
        }
      } catch {
        // ignore malformed frames
      }
    }

    return () => {
      closed = true
      ws?.close()
    }
  }, [])

  return latest
}
