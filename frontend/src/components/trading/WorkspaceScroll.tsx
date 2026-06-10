// Horizontal infinite-scroll workspace — fills viewport height, scrolls horizontally.

import { useRef } from 'react'
import { cn } from '@/lib/utils'

interface WorkspaceScrollProps {
  children: React.ReactNode
  className?: string
}

export function WorkspaceScroll({ children, className }: WorkspaceScrollProps) {
  const scrollRef = useRef<HTMLDivElement>(null)

  return (
    <div
      ref={scrollRef}
      className={cn(
        'flex h-full w-full overflow-x-auto overflow-y-hidden bg-background',
        // Smooth momentum scrolling on touch/trackpad.
        '[scroll-behavior:smooth]',
        className,
      )}
      style={{ scrollSnapType: 'none' }}
    >
      <div className="flex h-full">{children}</div>
    </div>
  )
}
