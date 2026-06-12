import { Pin, ExternalLink } from 'lucide-react'
import { useModeStore } from '@/store/mode'
import { cn } from '@/lib/utils'

export function ModeBadge({ className }: { className?: string }) {
  const { mode, pinned, toggleMode } = useModeStore()
  const isPaper = mode === 'PAPER'
  const other = isPaper ? 'LIVE' : 'PAPER'

  // Open the current page in a new window pinned to the other mode, so the
  // user can run PAPER and LIVE side by side on separate monitors.
  const openOtherWindow = () => {
    window.open(
      `${window.location.pathname}?mode=${other.toLowerCase()}`,
      '_blank',
      'noopener',
    )
  }

  return (
    <div className={cn('flex items-center gap-1', className)}>
      <button
        onClick={toggleMode}
        className={cn(
          'inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-semibold tracking-wide transition-colors border',
          isPaper
            ? 'bg-amber-500/10 text-amber-400 border-amber-500/30 hover:bg-amber-500/20'
            : 'bg-green-500/10 text-green-400 border-green-500/30 hover:bg-green-500/20',
        )}
        title={
          pinned
            ? `This window is pinned to ${mode}. Click to switch only this window.`
            : `Switch to ${other} mode (applies to all open tabs)`
        }
      >
        <span
          className={cn(
            'h-1.5 w-1.5 rounded-full shrink-0',
            isPaper ? 'bg-amber-400' : 'bg-green-400',
          )}
        />
        {mode}
        {pinned && <Pin className="h-2.5 w-2.5 shrink-0" />}
      </button>
      <button
        onClick={openOtherWindow}
        className="rounded-full p-1 text-text-dim hover:text-text hover:bg-border transition-colors"
        title={`Open a new window pinned to ${other} — trade ${mode} and ${other} side by side`}
        aria-label={`Open new ${other} window`}
      >
        <ExternalLink className="h-3 w-3" />
      </button>
    </div>
  )
}
