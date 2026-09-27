import { useEffect, useRef, useState } from 'react'
import { getWsClient, wsBus } from '@/api/ws'

export function useOrderBook(instrument: string | undefined, depth = 25) {
  const [bids, setBids] = useState<RawLevel[]>([])
  const [asks, setAsks] = useState<RawLevel[]>([])
  const [lastUpdate, setLastUpdate] = useState<number | null>(null)
  const [subscribed, setSubscribed] = useState(false)
  const panelId = useRef(`book-${Math.random().toString(36).slice(2, 8)}`)

  useEffect(() => {
    if (!instrument) return
    const client = getWsClient()
    const id = panelId.current
    setBids([])
    setAsks([])
    setLastUpdate(null)
    setSubscribed(false)

    client?.subscribe(id, [{ lane: 'ui.orderbook.snapshot', instrument, depth, max_fps: 6 }])

    const off = wsBus.on((msg) => {
      if ((msg as { type: string }).type === 'subscribed') {
        const m = msg as unknown as { lane: string; instrument: string }
        if (m.lane === 'ui.orderbook.snapshot' && m.instrument === instrument) setSubscribed(true)
        return
      }
      if (msg.type !== 'frame') return
      const f = msg as unknown as { lane: string; instrument: string; payload: BookPayload }
      if (f.lane !== 'ui.orderbook.snapshot' || f.instrument !== instrument) return
      const p = f.payload
      if (Array.isArray(p?.bids)) setBids(p.bids)
      if (Array.isArray(p?.asks)) setAsks(p.asks)
      setLastUpdate(Date.now())
    })

    return () => {
      off()
      client?.unsubscribe(id)
    }
  }, [instrument, depth])

  return { bids, asks, lastUpdate, subscribed }
}

export interface RawLevel {
  price: string
  size: string
}

interface BookPayload {
  kind?: 'snapshot' | 'delta'
  bids?: RawLevel[]
  asks?: RawLevel[]
  sequence?: number
}
