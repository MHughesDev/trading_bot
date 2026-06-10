// Tile component for the scanner watch table — rounded, no column dividers,
// hover reveals a remove control.

import { useState } from 'react'
import { X } from 'lucide-react'
import { cn, fmtCurrency, pnlClass } from '@/lib/utils'

export interface WatchTileData {
  instrumentId: string
  last?: number
  change?: number
  rankedMetric?: number
  rankedMetricLabel?: string
}

interface WatchTileProps {
  data: WatchTileData
  onRemove?: (instrumentId: string) => void
  onClick?: (instrumentId: string) => void
}

export function WatchTile({ data, onRemove, onClick }: WatchTileProps) {
  const [hovered, setHovered] = useState(false)
  const changeVal = data.change ?? 0

  return (
    <div
      className={cn(
        'relative flex items-center gap-3 rounded-xl px-4 py-3 cursor-pointer',
        'bg-surface-2 border border-border transition-colors',
        hovered && 'border-border-2 bg-surface',
      )}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onClick={() => onClick?.(data.instrumentId)}
    >
      {/* Instrument name */}
      <span className="flex-1 text-sm font-semibold text-text font-mono truncate">
        {data.instrumentId}
      </span>

      {/* Last price */}
      <span className="text-sm font-mono text-text tabular-nums">
        {data.last != null ? fmtCurrency(data.last) : '—'}
      </span>

      {/* Change % */}
      <span className={cn('text-xs font-mono tabular-nums w-16 text-right', pnlClass(changeVal))}>
        {changeVal >= 0 ? '+' : ''}
        {changeVal.toFixed(2)}%
      </span>

      {/* Ranked metric */}
      {data.rankedMetric != null && (
        <span className="text-xs text-text-dim tabular-nums w-20 text-right">
          {data.rankedMetricLabel && (
            <span className="text-text-dim mr-1">{data.rankedMetricLabel}</span>
          )}
          {data.rankedMetric.toFixed(4)}
        </span>
      )}

      {/* Hover remove button */}
      {hovered && onRemove && (
        <button
          className="absolute right-2 top-2 rounded p-0.5 text-text-dim hover:text-red-400 hover:bg-red-400/10 transition-colors"
          onClick={(e) => {
            e.stopPropagation()
            onRemove(data.instrumentId)
          }}
          aria-label={`Remove ${data.instrumentId}`}
        >
          <X className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  )
}
