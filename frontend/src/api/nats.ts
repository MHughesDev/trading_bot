// Raw NATS.ws client — connects to the NATS WebSocket endpoint exposed by
// the backend (Phase 2).  Implements the NATS text protocol over WebSocket
// with buffered line parsing and automatic reconnect.

type MsgHandler = (payload: Uint8Array) => void

interface Sub {
  sid: number
  subject: string
  handler: MsgHandler
}

const CRLF = '\r\n'

class NatsConnection {
  private ws: WebSocket | null = null
  private buf = ''
  private subs = new Map<number, Sub>()
  private nextSid = 1
  private reconnectMs = 1_000
  private closed = false
  private readonly wsUrl: string

  constructor(wsUrl: string) {
    this.wsUrl = wsUrl
    this.reconnect()
  }

  private reconnect() {
    if (this.closed) return
    try {
      const ws = new WebSocket(this.wsUrl)
      this.ws = ws

      ws.onopen = () => {
        this.reconnectMs = 1_000
        ws.send(`CONNECT {"verbose":false,"pedantic":false,"lang":"js","version":"0.1"}${CRLF}`)
        ws.send(`PING${CRLF}`)
        for (const sub of this.subs.values()) {
          ws.send(`SUB ${sub.subject} ${sub.sid}${CRLF}`)
        }
      }

      ws.onmessage = (ev: MessageEvent) => {
        if (ev.data instanceof ArrayBuffer) {
          this.buf += new TextDecoder().decode(new Uint8Array(ev.data as ArrayBuffer))
        } else {
          this.buf += ev.data as string
        }
        this.flush()
      }

      ws.onclose = () => {
        if (!this.closed) {
          setTimeout(() => {
            this.reconnectMs = Math.min(this.reconnectMs * 2, 30_000)
            this.reconnect()
          }, this.reconnectMs)
        }
      }

      ws.onerror = () => ws.close()
    } catch {
      // WebSocket constructor throws on invalid URL in some environments
    }
  }

  private flush() {
    for (;;) {
      const nl = this.buf.indexOf(CRLF)
      if (nl === -1) break
      const line = this.buf.slice(0, nl)
      this.buf = this.buf.slice(nl + 2)

      if (line === 'PING') {
        this.ws?.send(`PONG${CRLF}`)
        continue
      }

      if (line.startsWith('MSG ')) {
        // MSG <subject> <sid> [reply] <bytes>
        const parts = line.split(' ')
        const bytesStr = parts.length === 4 ? parts[3] : parts[4]
        const bytes = parseInt(bytesStr ?? '0', 10)
        const sid = parseInt(parts[2] ?? '0', 10)
        if (this.buf.length < bytes + 2) {
          this.buf = line + CRLF + this.buf
          break
        }
        const payloadStr = this.buf.slice(0, bytes)
        this.buf = this.buf.slice(bytes + 2)
        const sub = this.subs.get(sid)
        if (sub) {
          sub.handler(new TextEncoder().encode(payloadStr))
        }
      }
      // Ignore +OK, -ERR, INFO, PONG
    }
  }

  subscribe(subject: string, handler: MsgHandler): () => void {
    const sid = this.nextSid++
    this.subs.set(sid, { sid, subject, handler })
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(`SUB ${subject} ${sid}${CRLF}`)
    }
    return () => {
      this.subs.delete(sid)
      if (this.ws?.readyState === WebSocket.OPEN) {
        this.ws.send(`UNSUB ${sid}${CRLF}`)
      }
    }
  }

  close() {
    this.closed = true
    this.ws?.close()
  }
}

let _conn: NatsConnection | null = null

export function getNatsConnection(): NatsConnection {
  if (!_conn) {
    const proto = window.location.protocol === 'https:' ? 'wss' : 'ws'
    const url = `${proto}://${window.location.host}/nats`
    _conn = new NatsConnection(url)
  }
  return _conn
}

export { NatsConnection }
