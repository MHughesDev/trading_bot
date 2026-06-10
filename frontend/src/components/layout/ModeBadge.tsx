import { useModeStore } from '@/store/mode'
import { cn } from '@/lib/utils'

export function ModeBadge({ className }: { className?: string }) {
  const { mode, toggleMode } = useModeStore()
  const isPaper = mode === 'PAPER'

  return (
    <button
      onClick={toggleMode}
      className={cn(
        'inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-semibold transition-colors border',
        isPaper
          ? 'bg-amber-500/10 text-amber-400 border-amber-500/30 hover:bg-amber-500/20'
          : 'bg-green-500/10 text-green-400 border-green-500/30 hover:bg-green-500/20',
        className,
      )}
      title={`Switch to ${isPaper ? 'Live' : 'Paper'} mode`}
    >
      <span
        className={cn(
          'h-1.5 w-1.5 rounded-full',
          isPaper ? 'bg-amber-400' : 'bg-green-400',
        )}
      />
      {mode}
    </button>
  )
}
