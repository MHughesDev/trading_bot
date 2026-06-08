import { useEffect, useRef, useState } from 'react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { wsBus, getWsClient } from '@/api/ws'
import { formatPrice, formatSize } from '@/lib/format'
import type { BookLevel, OrderBookSnapshot, WsOutMessage } from '@/lib/types'

interface OrderBookPanelProps {
  instrument: string
  depth?: number
  maxFps?: number
}

const PANEL_ID_PREFIX = 'orderbook_'
const DEFAULT_DEPTH = 10
const DEFAULT_MAX_FPS = 20

function LevelRow({
  level,
  side,
  maxSize,
}: {
  level: BookLevel
  side: 'bid' | 'ask'
  maxSize: number
}) {
  const sizeNum = parseFloat(level.size)
  const pct = maxSize > 0 ? (sizeNum / maxSize) * 100 : 0
  const barColor = side === 'bid' ? 'bg-green-900/40' : 'bg-red-900/40'
  const priceColor = side === 'bid' ? 'text-green-400' : 'text-red-400'

  return (
    <div className="relative flex justify-between px-2 py-0.5 text-xs font-mono">
      <div
        className={`absolute inset-0 ${barColor}`}
        style={{ width: `${pct}%`, right: side === 'bid' ? 0 : 'auto', left: side === 'ask' ? 0 : 'auto' }}
      />
      <span className={`relative z-10 ${priceColor}`}>{formatPrice(level.price)}</span>
      <span className="relative z-10 text-text-muted">{formatSize(level.size)}</span>
    </div>
  )
}

export function OrderBookPanel({
  instrument,
  depth = DEFAULT_DEPTH,
  maxFps = DEFAULT_MAX_FPS,
}: OrderBookPanelProps) {
  const panelId = useRef(`${PANEL_ID_PREFIX}${instrument}`).current
  const [book, setBook] = useState<OrderBookSnapshot | null>(null)
  const [connected, setConnected] = useState(false)

  useEffect(() => {
    const client = getWsClient()
    if (!client) return

    client.subscribe(panelId, [
      {
        lane: 'ui.orderbook.snapshot',
        instrument,
        depth,
        max_fps: maxFps,
      },
    ])
    setConnected(true)

    const unsub = wsBus.on((msg: WsOutMessage) => {
      if (
        msg.type === 'frame' &&
        msg.lane === 'ui.orderbook.snapshot' &&
        msg.instrument === instrument
      ) {
        setBook(msg.payload as OrderBookSnapshot)
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
  }, [instrument, depth, maxFps, panelId])

  const bids = book?.bids.slice(0, depth) ?? []
  const asks = book?.asks.slice(0, depth) ?? []
  const maxSize = Math.max(
    ...bids.map((l) => parseFloat(l.size)),
    ...asks.map((l) => parseFloat(l.size)),
    0.001,
  )

  return (
    <Card>
      <CardHeader className="pb-2 flex-row items-center justify-between">
        <CardTitle className="text-sm">Order Book — {instrument}</CardTitle>
        <span className={`text-xs ${connected ? 'text-green-400' : 'text-text-dim'}`}>
          {connected ? `${maxFps} fps` : 'disconnected'}
        </span>
      </CardHeader>
      <CardContent className="p-0">
        {!book ? (
          <div className="flex h-40 items-center justify-center text-text-dim text-sm">
            Waiting for order book…
          </div>
        ) : (
          <div className="divide-y divide-border">
            <div>
              {asks
                .slice()
                .reverse()
                .map((level, i) => (
                  <LevelRow key={i} level={level} side="ask" maxSize={maxSize} />
                ))}
            </div>
            <div className="py-1 px-2 text-center text-xs text-text-dim">
              spread {formatPrice(
                String(parseFloat(asks[0]?.price ?? '0') - parseFloat(bids[0]?.price ?? '0')),
              )}
            </div>
            <div>
              {bids.map((level, i) => (
                <LevelRow key={i} level={level} side="bid" maxSize={maxSize} />
              ))}
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  )
}
