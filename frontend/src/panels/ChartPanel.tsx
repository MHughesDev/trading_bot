import { useEffect, useRef, useState } from 'react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { OhlcvChart, type Bar as OhlcvBar } from '@/components/charts/OhlcvChart'
import { wsBus, getWsClient } from '@/api/ws'
import type { Bar, WsOutMessage } from '@/lib/types'
import type { PriceLineAnnotation } from '@/components/charts/Annotations'

interface ChartPanelProps {
  instrument: string
  initialBars?: Bar[]
  priceLines?: PriceLineAnnotation[]
}

const PANEL_ID_PREFIX = 'chart_'

// Map the backend Bar (decimal string OHLCV, `time` field) to OhlcvChart's Bar.
function toOhlcvBar(b: Bar): OhlcvBar {
  return {
    ts: b.time,
    open: parseFloat(b.open),
    high: parseFloat(b.high),
    low: parseFloat(b.low),
    close: parseFloat(b.close),
    volume: parseFloat(b.volume),
  }
}

export function ChartPanel({ instrument, initialBars = [], priceLines = [] }: ChartPanelProps) {
  const panelId = useRef(`${PANEL_ID_PREFIX}${instrument}`).current
  const [bars, setBars] = useState<Bar[]>(initialBars)
  const [connected, setConnected] = useState(false)

  useEffect(() => {
    const client = getWsClient()
    if (!client) return

    client.subscribe(panelId, [
      { lane: 'market.bars.1m', instrument },
    ])
    setConnected(true)

    const unsub = wsBus.on((msg: WsOutMessage) => {
      if (
        msg.type === 'frame' &&
        msg.lane === 'market.bars.1m' &&
        msg.instrument === instrument
      ) {
        const bar = msg.payload as Bar
        setBars((prev) => {
          // Replace last bar if same timestamp, else append.
          if (prev.length > 0 && prev[prev.length - 1].time === bar.time) {
            return [...prev.slice(0, -1), bar]
          }
          return [...prev, bar].slice(-500)
        })
      }
      if (msg.type === 'heartbeat') {
        setConnected(true)
      }
    })

    return () => {
      unsub()
      client.unsubscribe(panelId)
      setConnected(false)
    }
  }, [instrument, panelId])

  useEffect(() => {
    if (initialBars.length > 0) setBars(initialBars)
  }, [initialBars])

  const ohlcvBars = bars.map(toOhlcvBar)

  return (
    <Card>
      <CardHeader className="pb-2 flex-row items-center justify-between">
        <CardTitle className="text-sm">
          {instrument} — 1m
        </CardTitle>
        <span className={`text-xs ${connected ? 'text-green-400' : 'text-text-dim'}`}>
          {connected ? 'live' : 'disconnected'}
        </span>
      </CardHeader>
      <CardContent className="p-0 pb-2">
        {ohlcvBars.length === 0 ? (
          <div className="flex h-72 items-center justify-center text-text-dim text-sm">
            Waiting for bars…
          </div>
        ) : (
          <OhlcvChart bars={ohlcvBars} markers={[]} priceLines={priceLines} height={300} />
        )}
      </CardContent>
    </Card>
  )
}
