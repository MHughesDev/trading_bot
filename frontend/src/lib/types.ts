// Domain types mirroring the Rust backend.
// All money values are decimal strings — never parse to float for display.

export interface Bar {
  time: number // unix timestamp (seconds)
  open: string
  high: string
  low: string
  close: string
  volume: string
}

export interface BookLevel {
  price: string
  size: string
}

export interface OrderBookSnapshot {
  bids: BookLevel[]
  asks: BookLevel[]
  sequence: number
}

export interface Position {
  instrument_id: string
  qty: string
  avg_entry_price: string
}

export interface OrderStatus {
  id: string
  instrument_id: string
  side: 'buy' | 'sell'
  order_type: 'market' | 'limit'
  qty: string
  status: string
  created_at: string
}

export interface TradingStatus {
  active: boolean
}

// ── WebSocket protocol ────────────────────────────────────────────────────────

export type WsOutMessage =
  | {
      type: 'subscribed'
      sub_id: string
      panel_id: string
      lane: string
      instrument: string
    }
  | {
      type: 'frame'
      sub_id: string
      lane: string
      instrument: string
      payload: unknown
    }
  | { type: 'heartbeat'; ts: string }
  | { type: 'error'; code: string; message: string }

export interface SubSpec {
  lane: string
  instrument: string
  depth?: number
  max_fps?: number
}

export interface SubscribeMessage {
  panel_id: string
  subscribe: SubSpec[]
}

export interface UnsubscribeMessage {
  panel_id: string
  unsubscribe: true
}
