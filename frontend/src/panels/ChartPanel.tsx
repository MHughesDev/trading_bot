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
// Returns null if the payload is missing required fields (L-11 safety).
function toOhlcvBar(b: Bar): OhlcvBar | null {
  const open = parseFloat(b.open)
  const high = parseFloat(b.high)
  const low = parseFloat(b.low)
  const close = parseFloat(b.close)
  const volume = parseFloat(b.volume)
  if (!b.time || isNaN(open) || isNaN(high) || isNaN(low) || isNaN(close)) return null
  return { ts: b.time, open, high, low, close, volume: isNaN(volume) ? 0 : volume }
}

// Validate that a WS payload looks like a Bar before casting (L-11).
function isBar(payload: unknown): payload is Bar {
  if (!payload || typeof payload !== 'object') return false
  const p = payload as Record<string, unknown>
  return (
    typeof p.time === 'string' &&
    typeof p.open === 'string' &&
    typeof p.high === 'string' &&
    typeof p.low === 'string' &&
    typeof p.close === 'string'
  )
}

type ConnectionState = 'live' | 'reconnecting' | 'error' | 'disconnected'

export function ChartPanel({ instrument, initialBars = [], priceLines = [] }: ChartPanelProps) {
  const panelId = useRef(`${PANEL_ID_PREFIX}${instrument}`).current
  // Seed bars from initialBars via useState initializer so live-accumulated
  // data is never overwritten by a re-passed prop reference (L-7 fix).
  const [bars, setBars] = useState<Bar[]>(() => initialBars)
  const [connState, setConnState] = useState<ConnectionState>('disconnected')

  useEffect(() => {
    const client = getWsClient()
    if (!client) return

    client.subscribe(panelId, [
      { lane: 'market.bars.1m', instrument },
    ])
    // Do NOT set connected here — wait for the actual 'connected' event (L-6).

    const unsub = wsBus.on((msg: WsOutMessage) => {
      // Handle control messages for connection state (L-5, L-14).
      if ((msg as { type: string }).type === 'connected') {
        setConnState('live')
        return
      }
      if ((msg as { type: string }).type === 'disconnected') {
        setConnState('reconnecting')
        return
      }
      if ((msg as { type: string }).type === 'error') {
        setConnState('error')
        return
      }

      if (
        msg.type === 'frame' &&
        msg.lane === 'market.bars.1m' &&
        msg.instrument === instrument
      ) {
        // Validate payload before casting (L-11).
        if (!isBar(msg.payload)) return
        const bar = msg.payload
        setBars((prev) => {
          if (prev.length > 0 && prev[prev.length - 1].time === bar.time) {
            return [...prev.slice(0, -1), bar]
          }
          return [...prev, bar].slice(-500)
        })
      }
      if (msg.type === 'heartbeat') {
        setConnState('live')
      }
    })

    return () => {
      unsub()
      client.unsubscribe(panelId)
      setConnState('disconnected')
    }
  }, [instrument, panelId])

  // Do NOT have an effect that resets bars from initialBars on prop change —
  // that overwrites accumulated live data when the parent re-renders (L-7).

  const ohlcvBars = bars.flatMap((b) => {
    const mapped = toOhlcvBar(b)
    return mapped ? [mapped] : []
  })

  const connLabel =
    connState === 'live' ? 'live'
    : connState === 'reconnecting' ? 'reconnecting'
    : connState === 'error' ? 'error'
    : 'disconnected'

  const connColor =
    connState === 'live' ? 'text-green-400'
    : connState === 'error' ? 'text-red-400'
    : 'text-text-dim'

  return (
    <Card>
      <CardHeader className="pb-2 flex-row items-center justify-between">
        <CardTitle className="text-sm">
          {instrument} — 1m
        </CardTitle>
        <span className={`text-xs ${connColor}`}>
          {connLabel}
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
