// Panel frame for the horizontal workspace — fixed width, full height.

import { useState } from 'react'
import { cn } from '@/lib/utils'
import { X, ChevronLeft, ChevronRight } from 'lucide-react'

interface PanelProps {
  title?: string
  width?: number
  onClose?: () => void
  children: React.ReactNode
  className?: string
}

export function Panel({ title, width = 480, onClose, children, className }: PanelProps) {
  const [collapsed, setCollapsed] = useState(false)

  if (collapsed) {
    return (
      <div
        className={cn(
          'flex flex-col h-full shrink-0 border-r border-border bg-surface overflow-hidden',
          className,
        )}
        style={{ width: 36 }}
      >
        <button
          onClick={() => setCollapsed(false)}
          className="flex flex-col items-center gap-2 py-3 w-full text-text-dim hover:text-text hover:bg-border transition-colors"
          aria-label="Expand panel"
        >
          <ChevronRight className="h-3.5 w-3.5 shrink-0" />
          {title && (
            <span
              className="text-xs font-semibold uppercase tracking-wider text-text-muted"
              style={{ writingMode: 'vertical-rl', transform: 'rotate(180deg)' }}
            >
              {title}
            </span>
          )}
        </button>
      </div>
    )
  }

  return (
    <div
      className={cn(
        'flex flex-col h-full shrink-0 border-r border-border bg-surface overflow-hidden',
        className,
      )}
      style={{ width }}
    >
      {(title || onClose) && (
        <div className="flex items-center justify-between px-3 py-2 border-b border-border shrink-0">
          <button
            onClick={() => setCollapsed(true)}
            className="rounded p-0.5 text-text-dim hover:text-text hover:bg-border transition-colors"
            aria-label="Collapse panel"
          >
            <ChevronLeft className="h-3.5 w-3.5" />
          </button>
          {title && (
            <span className="text-xs font-semibold text-text-muted uppercase tracking-wider ml-1.5 flex-1">
              {title}
            </span>
          )}
          {onClose && (
            <button
              onClick={onClose}
              className="rounded p-0.5 text-text-dim hover:text-text hover:bg-border transition-colors"
              aria-label="Close panel"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
      )}
      <div className="flex-1 overflow-y-auto">{children}</div>
    </div>
  )
}
