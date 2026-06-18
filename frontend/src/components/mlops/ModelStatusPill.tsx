import { motion, useReducedMotion } from 'framer-motion'
import { cn } from '@/lib/utils'
import type { ModelStatus } from '@/api/mlops'

interface ModelStatusPillProps {
  status: ModelStatus
  className?: string
}

const STATUS_CONFIG: Record<
  ModelStatus,
  { label: string; className: string; pulse?: boolean; glow?: string }
> = {
  draft: {
    label: 'Draft',
    className: 'bg-[color:var(--tb-surface-2)] text-[color:var(--tb-text-muted)] border border-[color:var(--tb-border)]',
  },
  training: {
    label: 'Training',
    className: 'bg-blue-500/15 text-blue-400 border border-blue-500/30',
    pulse: true,
    glow: '0 0 8px rgba(59,130,246,0.4)',
  },
  evaluating: {
    label: 'Evaluating',
    className: 'bg-amber-500/15 text-amber-400 border border-amber-500/30',
    pulse: true,
    glow: '0 0 8px rgba(245,158,11,0.4)',
  },
  candidate: {
    label: 'Candidate',
    className: 'bg-purple-500/15 text-purple-400 border border-purple-500/30',
  },
  active: {
    label: 'Active',
    className: 'bg-[color:var(--tb-pnl-up)]/15 text-[color:var(--tb-pnl-up)] border border-[color:var(--tb-pnl-up)]/30',
    glow: '0 0 10px color-mix(in srgb, var(--tb-pnl-up) 40%, transparent)',
  },
  archived: {
    label: 'Archived',
    className: 'bg-[color:var(--tb-surface-2)] text-[color:var(--tb-text-dim)] border border-[color:var(--tb-border)] opacity-60',
  },
  failed: {
    label: 'Failed',
    className: 'bg-[color:var(--tb-pnl-down)]/10 text-[color:var(--tb-pnl-down)]/70 border border-[color:var(--tb-pnl-down)]/20 opacity-80',
  },
}

export function ModelStatusPill({ status, className }: ModelStatusPillProps) {
  const shouldReduce = useReducedMotion()
  const cfg = STATUS_CONFIG[status]

  const pill = (
    <span
      className={cn(
        'inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium font-mono',
        cfg.className,
        className,
      )}
      style={cfg.glow ? { boxShadow: cfg.glow } : undefined}
    >
      {cfg.pulse && (
        <span
          className={cn(
            'h-1.5 w-1.5 rounded-full',
            status === 'training' ? 'bg-blue-400' : 'bg-amber-400',
          )}
        />
      )}
      {cfg.label}
    </span>
  )

  if (cfg.pulse && !shouldReduce) {
    return (
      <motion.span
        animate={{ opacity: [1, 0.6, 1] }}
        transition={{ repeat: Infinity, duration: 1.6, ease: 'easeInOut' }}
      >
        {pill}
      </motion.span>
    )
  }

  return pill
}
