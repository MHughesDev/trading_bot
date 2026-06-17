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
  const cb = useRef(onProgress)
  cb.current = onProgress

  useEffect(() => {
    const token = getStoredToken() ?? 'dev-local'
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
