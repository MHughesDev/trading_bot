// WebSocket client with automatic reconnect, snapshot-on-connect, and a
// lightweight message bus so multiple panels can share one connection.
import type { WsOutMessage, SubSpec } from '@/lib/types'

export type WsMessageHandler = (msg: WsOutMessage) => void

// Synthetic control messages emitted on wsBus for connection state changes.
// These are NOT server messages — they have type 'connected' | 'disconnected' | 'error'.
export interface WsConnectedMessage { type: 'connected' }
export interface WsDisconnectedMessage { type: 'disconnected' }
export interface WsErrorMessage { type: 'error' }
export type WsControlMessage = WsConnectedMessage | WsDisconnectedMessage | WsErrorMessage

class WsMessageBus {
  private listeners = new Set<WsMessageHandler>()

  emit(msg: WsOutMessage | WsControlMessage) {
    this.listeners.forEach((l) => l(msg as WsOutMessage))
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
  /** The token this client was built for, so a re-init with the same token is a no-op. */
  readonly token: string
  private reconnectDelay = 1000
  private closed = false
  private pending: PendingSub[] = []

  constructor(token: string) {
    this.token = token
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
      // Notify panels that the connection is live (L-6: was set before onopen).
      wsBus.emit({ type: 'connected' })
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
      // Notify panels so they can show a reconnecting state (L-14).
      wsBus.emit({ type: 'disconnected' })
      if (!this.closed) {
        setTimeout(() => {
          this.reconnectDelay = Math.min(this.reconnectDelay * 2, 30_000)
          this.connect()
        }, this.reconnectDelay)
      }
    }

    ws.onerror = () => {
      // Emit an error event before closing so panels can show an error state (L-5).
      wsBus.emit({ type: 'error' })
      ws.close()
    }
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

  /** True when the socket is open right now. */
  isConnected(): boolean {
    return this.ws?.readyState === WebSocket.OPEN
  }

  destroy() {
    this.closed = true
    this.ws?.close()
  }
}

let _client: WsClient | null = null

/**
 * Ensure a live client exists for `token`.
 *
 * Idempotent on purpose. React runs a child's effects before its parent's, so
 * panels subscribe before the app shell calls this. Destroying and rebuilding
 * the socket here would silently drop every subscription made in that window —
 * which is exactly what made the order book (whose effect deps never change)
 * permanently empty while the watchlist (whose deps change when its symbol list
 * loads, causing a re-subscribe) appeared to work.
 */
export function initWsClient(token: string) {
  if (_client && _client.token === token) return
  _client?.destroy()
  _client = new WsClient(token)
}

// Eagerly create the client at module load time if the user has a stored token.
// This ensures getWsClient() is non-null when components first mount, avoiding
// a race where child useEffect() runs before the parent AppLayout's useEffect
// (React fires children's effects before parents').
{
  const stored = typeof window !== 'undefined' ? localStorage.getItem('auth_token') : null
  if (stored) {
    _client = new WsClient(stored)
  }
}

export function getWsClient(): WsClient | null {
  return _client
}

export function destroyWsClient() {
  _client?.destroy()
  _client = null
}
