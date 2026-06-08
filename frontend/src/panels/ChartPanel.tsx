import { useEffect, useRef, useState } from 'react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { OhlcvChart } from '@/components/charts/OhlcvChart'
import { wsBus, getWsClient } from '@/api/ws'
import type { Bar, WsOutMessage } from '@/lib/types'

interface ChartPanelProps {
  instrument: string
  initialBars?: Bar[]
}

const PANEL_ID_PREFIX = 'chart_'

export function ChartPanel({ instrument, initialBars = [] }: ChartPanelProps) {
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
          return [...prev, bar].slice(-500) // keep last 500 bars
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

  // Update initial bars when prop changes.
  useEffect(() => {
    if (initialBars.length > 0) setBars(initialBars)
  }, [initialBars])

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
        {bars.length === 0 ? (
          <div className="flex h-72 items-center justify-center text-text-dim text-sm">
            Waiting for bars…
          </div>
        ) : (
          <OhlcvChart bars={bars} markers={[]} height={300} />
        )}
      </CardContent>
    </Card>
  )
}
