import { useEffect, useRef, useState, type ReactNode } from 'react'
import { ChevronLeft, ChevronRight, GripVertical, X } from 'lucide-react'
import { IconButton } from '@/components/primitives/Button'
import { cn } from '@/lib/utils'

/* The chrome around one workspace panel: collapse, drag to reorder, resize,
   close. Width and collapsed state persist with the panel so a refresh returns
   the desk exactly as it was left. */

const MIN_WIDTH = 240
const MAX_WIDTH = 1400

export interface WorkspacePanelProps {
  title: ReactNode
  subtitle?: ReactNode
  width: number
  collapsed?: boolean
  onWidthChange: (w: number) => void
  onCollapsedChange: (c: boolean) => void
  onClose?: () => void
  actions?: ReactNode
  children: ReactNode
  isDragging?: boolean
  isDragOver?: boolean
  onDragStart?: (e: React.DragEvent) => void
  onDragOver?: (e: React.DragEvent) => void
  onDrop?: (e: React.DragEvent) => void
  onDragEnd?: () => void
}

export function WorkspacePanel({
  title,
  subtitle,
  width,
  collapsed = false,
  onWidthChange,
  onCollapsedChange,
  onClose,
  actions,
  children,
  isDragging,
  isDragOver,
  onDragStart,
  onDragOver,
  onDrop,
  onDragEnd,
}: WorkspacePanelProps) {
  const [resizing, setResizing] = useState(false)
  const start = useRef({ x: 0, w: 0 })

  useEffect(() => {
    if (!resizing) return
    function move(e: MouseEvent) {
      const next = Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, start.current.w + (e.clientX - start.current.x)))
      onWidthChange(next)
    }
    function up() {
      setResizing(false)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
    document.body.style.cursor = 'col-resize'
    document.body.style.userSelect = 'none'
    document.addEventListener('mousemove', move)
    document.addEventListener('mouseup', up)
    return () => {
      document.removeEventListener('mousemove', move)
      document.removeEventListener('mouseup', up)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
  }, [resizing, onWidthChange])

  if (collapsed) {
    return (
      <div className="panel ws-panel" style={{ width: 38 }}>
        <div style={{ padding: '6px 0', display: 'grid', placeItems: 'center' }}>
          <IconButton label="Expand panel" bare onClick={() => onCollapsedChange(false)}>
            <ChevronRight size={14} aria-hidden />
          </IconButton>
        </div>
        <div className="ws-collapsed">{title}</div>
      </div>
    )
  }

  return (
    <div
      className={cn('panel ws-panel', isDragging && 'dragging', isDragOver && 'dropzone')}
      style={{ width }}
      onDragOver={onDragOver}
      onDrop={onDrop}
    >
      <div className="panel-hd" style={{ paddingLeft: 6, gap: 6 }}>
        <span
          className="ws-grip"
          draggable={!!onDragStart}
          onDragStart={onDragStart}
          onDragEnd={onDragEnd}
          title="Drag to reorder"
          aria-hidden
        >
          <GripVertical size={13} />
        </span>
        <IconButton label="Collapse panel" bare onClick={() => onCollapsedChange(true)}>
          <ChevronLeft size={13} aria-hidden />
        </IconButton>
        <span className="panel-title truncate-1">{title}</span>
        {subtitle && <span className="lbl truncate-1">{subtitle}</span>}
        <span className="spacer" />
        {actions}
        {onClose && (
          <IconButton label="Close panel" bare onClick={onClose}>
            <X size={13} aria-hidden />
          </IconButton>
        )}
      </div>

      <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        {children}
      </div>

      <div
        className={cn('ws-resize', resizing && 'on')}
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize panel"
        onMouseDown={(e) => {
          e.preventDefault()
          start.current = { x: e.clientX, w: width }
          setResizing(true)
        }}
      />
    </div>
  )
}
