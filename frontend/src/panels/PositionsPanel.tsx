import { useEffect, useState } from 'react'
import { useRef } from 'react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { wsBus, getWsClient } from '@/api/ws'
import { formatSize, formatPrice, pnlClass } from '@/lib/format'
import type { Position, WsOutMessage } from '@/lib/types'

interface PositionsPanelProps {
  userId: string
}

const PANEL_ID = 'positions_panel'

// Validate that a WS payload looks like a Position before using it (L-11).
function isPosition(payload: unknown): payload is Position {
  if (!payload || typeof payload !== 'object') return false
  const p = payload as Record<string, unknown>
  return typeof p.instrument_id === 'string' && typeof p.qty === 'string'
}

type ConnectionState = 'live' | 'reconnecting' | 'error' | 'disconnected'

export function PositionsPanel({ userId }: PositionsPanelProps) {
  const panelId = useRef(PANEL_ID).current
  const [positions, setPositions] = useState<Position[]>([])
  const [connState, setConnState] = useState<ConnectionState>('disconnected')

  useEffect(() => {
    const client = getWsClient()
    if (!client) return

    // Positions are a private lane — subscribe scoped to our own user.
    client.subscribe(panelId, [
      { lane: 'positions.events', instrument: userId },
    ])
    // Do NOT set connected here — wait for the actual 'connected' event (L-6).

    const unsub = wsBus.on((msg: WsOutMessage) => {
      // Handle control messages (L-5, L-14).
      const type = (msg as { type: string }).type
      if (type === 'connected') { setConnState('live'); return }
      if (type === 'disconnected') { setConnState('reconnecting'); return }
      if (type === 'error') { setConnState('error'); return }

      if (msg.type === 'frame' && msg.lane === 'positions.events') {
        // Validate payload before use (L-11).
        if (!isPosition(msg.payload)) return
        const update = msg.payload
        setPositions((prev) => {
          const idx = prev.findIndex((p) => p.instrument_id === update.instrument_id)
          if (idx >= 0) {
            const next = [...prev]
            next[idx] = update
            return next
          }
          return [...prev, update]
        })
      }
      if (msg.type === 'heartbeat') setConnState('live')
    })

    return () => {
      unsub()
      client.unsubscribe(panelId)
      setConnState('disconnected')
    }
  }, [userId, panelId])

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
        <CardTitle className="text-sm">Positions</CardTitle>
        <span className={`text-xs ${connColor}`}>
          {connLabel}
        </span>
      </CardHeader>
      <CardContent className="p-0">
        {positions.length === 0 ? (
          <div className="px-4 py-6 text-sm text-text-dim text-center">No open positions.</div>
        ) : (
          <table className="w-full text-xs font-mono">
            <thead>
              <tr className="text-text-dim border-b border-border">
                <th className="px-3 py-1.5 text-left">Instrument</th>
                <th className="px-3 py-1.5 text-right">Qty</th>
                <th className="px-3 py-1.5 text-right">Avg Entry</th>
              </tr>
            </thead>
            <tbody>
              {positions.map((p) => {
                const qtyNum = parseFloat(p.qty)
                return (
                  <tr key={p.instrument_id} className="border-b border-border/50">
                    <td className="px-3 py-1.5 text-text">{p.instrument_id}</td>
                    <td className={`px-3 py-1.5 text-right ${pnlClass(qtyNum)}`}>
                      {formatSize(p.qty)}
                    </td>
                    <td className="px-3 py-1.5 text-right text-text-muted">
                      {formatPrice(p.avg_entry_price)}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        )}
      </CardContent>
    </Card>
  )
}
