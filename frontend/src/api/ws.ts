// WebSocket client with automatic reconnect, snapshot-on-connect, and a
// lightweight message bus so multiple panels can share one connection.
import type { WsOutMessage, SubSpec } from '@/lib/types'

export type WsMessageHandler = (msg: WsOutMessage) => void

class WsMessageBus {
  private listeners = new Set<WsMessageHandler>()

  emit(msg: WsOutMessage) {
    this.listeners.forEach((l) => l(msg))
  }

  on(fn: WsMessageHandler): () => void {
    this.listeners.add(fn)
    return () => this.listeners.delete(fn)
  }
}

export const wsBus = new WsMessageBus()

interface PendingSub {
  panel_id: string
  specs: SubSpec[]
}

class WsClient {
  private ws: WebSocket | null = null
  private readonly url: string
  private reconnectDelay = 1000
  private closed = false
  private pending: PendingSub[] = []

  constructor(token: string) {
    const proto = window.location.protocol === 'https:' ? 'wss' : 'ws'
    this.url = `${proto}://${window.location.host}/ws/live?token=${encodeURIComponent(token)}`
    this.connect()
  }

  private connect() {
    if (this.closed) return
    const ws = new WebSocket(this.url)
    this.ws = ws

    ws.onopen = () => {
      this.reconnectDelay = 1000
      // Replay pending subscriptions on reconnect.
      for (const { panel_id, specs } of this.pending) {
        ws.send(JSON.stringify({ panel_id, subscribe: specs }))
      }
    }

    ws.onmessage = (e) => {
      try {
        const msg: WsOutMessage = JSON.parse(e.data as string)
        wsBus.emit(msg)
      } catch {
        // ignore malformed messages
      }
    }

    ws.onclose = () => {
      if (!this.closed) {
        setTimeout(() => {
          this.reconnectDelay = Math.min(this.reconnectDelay * 2, 30_000)
          this.connect()
        }, this.reconnectDelay)
      }
    }

    ws.onerror = () => ws.close()
  }

  subscribe(panel_id: string, specs: SubSpec[]) {
    // Track so we can replay on reconnect.
    this.pending = this.pending.filter((p) => p.panel_id !== panel_id)
    this.pending.push({ panel_id, specs })

    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({ panel_id, subscribe: specs }))
    }
  }

  unsubscribe(panel_id: string) {
    this.pending = this.pending.filter((p) => p.panel_id !== panel_id)
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({ panel_id, unsubscribe: true }))
    }
  }

  destroy() {
    this.closed = true
    this.ws?.close()
  }
}

let _client: WsClient | null = null

export function initWsClient(token: string) {
  _client?.destroy()
  _client = new WsClient(token)
}

export function getWsClient(): WsClient | null {
  return _client
}

export function destroyWsClient() {
  _client?.destroy()
  _client = null
}
