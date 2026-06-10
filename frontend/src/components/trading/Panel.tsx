// Panel frame for the horizontal workspace — fixed width, full height.

import { cn } from '@/lib/utils'
import { X } from 'lucide-react'

interface PanelProps {
  title?: string
  width?: number
  onClose?: () => void
  children: React.ReactNode
  className?: string
}

export function Panel({ title, width = 480, onClose, children, className }: PanelProps) {
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
          {title && (
            <span className="text-xs font-semibold text-text-muted uppercase tracking-wider">
              {title}
            </span>
          )}
          {onClose && (
            <button
              onClick={onClose}
              className="ml-auto rounded p-0.5 text-text-dim hover:text-text hover:bg-border transition-colors"
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
