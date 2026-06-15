// Panel frame for the horizontal workspace — fixed width, full height, resizable.

import { useState, useCallback } from 'react'
import { cn } from '@/lib/utils'
import { X, ChevronLeft, ChevronRight, GripVertical } from 'lucide-react'

const MIN_WIDTH = 220
const MAX_WIDTH = 1400

interface PanelProps {
  title?: string
  width?: number
  onClose?: () => void
  children: React.ReactNode
  className?: string
  // drag-and-drop reordering
  isDragging?: boolean
  isDragOver?: boolean
  onDragStart?: (e: React.DragEvent) => void
  onDragOver?: (e: React.DragEvent) => void
  onDrop?: (e: React.DragEvent) => void
  onDragEnd?: () => void
}

export function Panel({
  title,
  width: initialWidth = 480,
  onClose,
  children,
  className,
  isDragging,
  isDragOver,
  onDragStart,
  onDragOver,
  onDrop,
  onDragEnd,
}: PanelProps) {
  const [collapsed, setCollapsed] = useState(false)
  const [width, setWidth] = useState(initialWidth)
  const [resizing, setResizing] = useState(false)

  const startResize = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      const startX = e.clientX
      const startWidth = width
      setResizing(true)
      document.body.style.cursor = 'col-resize'
      document.body.style.userSelect = 'none'

      const onMove = (ev: MouseEvent) => {
        const next = Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, startWidth + ev.clientX - startX))
        setWidth(next)
      }

      const onUp = () => {
        setResizing(false)
        document.body.style.cursor = ''
        document.body.style.userSelect = ''
        document.removeEventListener('mousemove', onMove)
        document.removeEventListener('mouseup', onUp)
      }

      document.addEventListener('mousemove', onMove)
      document.addEventListener('mouseup', onUp)
    },
    [width],
  )

  if (collapsed) {
    return (
      <div
        className={cn(
          'relative flex flex-col h-full shrink-0 border-r border-border bg-surface transition-opacity',
          isDragging && 'opacity-40',
          className,
        )}
        style={{ width: 36 }}
        onDragOver={onDragOver}
        onDrop={onDrop}
      >
        {/* Drop indicator */}
        {isDragOver && (
          <div className="absolute left-0 top-0 bottom-0 w-0.5 bg-blue-500 z-30 pointer-events-none" />
        )}
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
        'relative flex flex-col h-full shrink-0 border-r border-border bg-surface transition-opacity',
        isDragging && 'opacity-40',
        className,
      )}
      style={{ width }}
      onDragOver={onDragOver}
      onDrop={onDrop}
    >
      {/* Drop indicator — blue line on left edge of drop target */}
      {isDragOver && (
        <div className="absolute left-0 top-0 bottom-0 w-0.5 bg-blue-500 z-30 pointer-events-none" />
      )}

      {/* Header — drag handle when draggable */}
      {(title || onClose) && (
        <div
          className={cn(
            'flex items-center justify-between px-2 py-2 border-b border-border shrink-0 overflow-hidden',
            onDragStart && 'cursor-grab active:cursor-grabbing select-none',
          )}
          draggable={!!onDragStart}
          onDragStart={onDragStart}
          onDragEnd={onDragEnd}
        >
          <button
            onClick={() => setCollapsed(true)}
            onMouseDown={(e) => e.stopPropagation()}
            className="rounded p-0.5 text-text-dim hover:text-text hover:bg-border transition-colors shrink-0"
            aria-label="Collapse panel"
          >
            <ChevronLeft className="h-3.5 w-3.5" />
          </button>

          {onDragStart && (
            <GripVertical className="h-3.5 w-3.5 text-text-dim/50 shrink-0 mx-1" />
          )}

          {title && (
            <span className="text-xs font-semibold text-text-muted uppercase tracking-wider ml-0.5 flex-1 truncate">
              {title}
            </span>
          )}

          {onClose && (
            <button
              onClick={onClose}
              onMouseDown={(e) => e.stopPropagation()}
              className="rounded p-0.5 text-text-dim hover:text-text hover:bg-border transition-colors shrink-0"
              aria-label="Close panel"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
      )}

      {/* Content */}
      <div className="flex-1 overflow-y-auto overflow-x-hidden">{children}</div>

      {/* Resize handle */}
      <div
        onMouseDown={startResize}
        className={cn(
          'absolute right-0 top-0 bottom-0 z-20 cursor-col-resize',
          'w-[5px] transition-colors duration-100',
          resizing
            ? 'bg-blue-500/60'
            : 'bg-transparent hover:bg-blue-500/40',
        )}
        aria-hidden
      />
    </div>
  )
}
