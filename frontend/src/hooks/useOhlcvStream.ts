// Subscribe to live 1-min OHLCV bars from the NATS.ws feed.
//
// Subject pattern: md.market.ohlcv.<venue>.<asset_class>.<instrument_id>
// e.g. md.market.ohlcv.kraken.crypto_spot_cex.BTC-USD

import { useEffect, useState } from 'react'
import { getNatsConnection } from '@/api/nats'

export interface OhlcvBar {
  time: number   // unix timestamp (seconds)
  open: number
  high: number
  low: number
  close: number
  volume: number
}

interface UseOhlcvStreamResult {
  bars: OhlcvBar[]
  connected: boolean
}

export function useOhlcvStream(
  venue: string,
  assetClass: string,
  instrument: string,
  maxBars = 500,
): UseOhlcvStreamResult {
  const [bars, setBars] = useState<OhlcvBar[]>([])
  const [connected, setConnected] = useState(false)

  useEffect(() => {
    if (!venue || !assetClass || !instrument) return

    const subject = `md.market.ohlcv.${venue}.${assetClass}.${instrument}`
    const conn = getNatsConnection()
    let active = true

    const unsub = conn.subscribe(subject, (payload: Uint8Array) => {
      if (!active) return
      try {
        const bar = JSON.parse(new TextDecoder().decode(payload)) as OhlcvBar
        setConnected(true)
        setBars((prev) => {
          // Replace last bar if same timestamp (in-progress bar update).
          if (prev.length > 0 && prev[prev.length - 1].time === bar.time) {
            const next = [...prev.slice(0, -1), bar]
            return next.length > maxBars ? next.slice(-maxBars) : next
          }
          const next = [...prev, bar]
          return next.length > maxBars ? next.slice(-maxBars) : next
        })
      } catch {
        // ignore malformed frames
      }
    })

    return () => {
      active = false
      unsub()
    }
  }, [venue, assetClass, instrument, maxBars])

  return { bars, connected }
}
