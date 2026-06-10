// Scanner panel — tile-based watch table populated by a discovery strategy.
// Strategy dropdown at top (apply-list filtered to discovery strategies only).
// Hover-to-remove on each tile.

import { useState, useCallback } from 'react'
import { useQuery } from '@tanstack/react-query'
import { strategiesApi } from '@/lib/api'
import { WatchTile, type WatchTileData } from './WatchTile'
import { cn } from '@/lib/utils'
import { ChevronDown } from 'lucide-react'

interface ScannerPanelProps {
  initialInstruments?: string[]
}

export function ScannerPanel({ initialInstruments = [] }: ScannerPanelProps) {
  const [selectedStrategyId, setSelectedStrategyId] = useState<string>('')
  const [tiles, setTiles] = useState<WatchTileData[]>(
    initialInstruments.map((id) => ({ instrumentId: id })),
  )

  const { data: strategiesResp } = useQuery({
    queryKey: ['strategies', 'apply-list', 'discovery'],
    queryFn: () => strategiesApi.list().then((r) => r.data),
  })

  const strategies = (
    Array.isArray(strategiesResp)
      ? strategiesResp
      : (strategiesResp as { strategies?: unknown[] })?.strategies ?? []
  ) as Array<{ id: string; strategy_id: string; strategy_kind?: string }>

  const discoveryStrategies = strategies.filter(
    (s) => !s.strategy_kind || s.strategy_kind === 'discovery',
  )

  const handleStrategyChange = useCallback((strategyId: string) => {
    setSelectedStrategyId(strategyId)
    // In a full implementation, populate tiles from strategy scanner output.
  }, [])

  const handleRemove = useCallback((instrumentId: string) => {
    setTiles((prev) => prev.filter((t) => t.instrumentId !== instrumentId))
  }, [])

  return (
    <div className="flex flex-col h-full">
      {/* Strategy selector */}
      <div className="shrink-0 px-3 py-2 border-b border-border">
        <div className="relative">
          <select
            value={selectedStrategyId}
            onChange={(e) => handleStrategyChange(e.target.value)}
            className={cn(
              'w-full appearance-none rounded-lg px-3 py-1.5 pr-8 text-sm',
              'bg-surface-2 border border-border text-text',
              'focus:outline-none focus:ring-1 focus:ring-accent',
            )}
          >
            <option value="">Select discovery strategy…</option>
            {discoveryStrategies.map((s) => (
              <option key={s.id} value={s.id}>
                {s.strategy_id}
              </option>
            ))}
          </select>
          <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
        </div>
      </div>

      {/* Tile table */}
      <div className="flex-1 overflow-y-auto p-3 space-y-2">
        {tiles.length === 0 ? (
          <div className="flex h-32 items-center justify-center text-text-dim text-sm">
            {selectedStrategyId ? 'No instruments found' : 'Select a strategy to populate'}
          </div>
        ) : (
          tiles.map((tile) => (
            <WatchTile
              key={tile.instrumentId}
              data={tile}
              onRemove={handleRemove}
            />
          ))
        )}
      </div>
    </div>
  )
}
