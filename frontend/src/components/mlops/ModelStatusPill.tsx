import { motion, useReducedMotion } from 'framer-motion'
import { cn } from '@/lib/utils'
import type { ModelStatus } from '@/api/mlops'

interface ModelStatusPillProps {
  status: ModelStatus
  className?: string
}

/* A model's lifecycle state wears the same badge tones as every other state in
   the product: a training run is an in-progress thing (accent), an evaluation
   is a caution (warn), an active model is positive, a failure is negative. The
   glow is a `terminal`-only affordance and resolves to `none` in `paper`. */
const STATUS_CONFIG: Record<
  ModelStatus,
  { label: string; tone: string; pulse?: boolean; glow?: string }
> = {
  draft: { label: 'Draft', tone: 'neutral' },
  training: { label: 'Training', tone: 'accent', pulse: true, glow: 'var(--glow-accent)' },
  evaluating: { label: 'Evaluating', tone: 'warn', pulse: true },
  candidate: { label: 'Candidate', tone: 'info' },
  active: { label: 'Active', tone: 'pos', glow: 'var(--glow-pos)' },
  archived: { label: 'Archived', tone: 'neutral' },
  failed: { label: 'Failed', tone: 'neg' },
}

export function ModelStatusPill({ status, className }: ModelStatusPillProps) {
  const shouldReduce = useReducedMotion()
  const cfg = STATUS_CONFIG[status]

  const pill = (
    <span
      className={cn('badge', cfg.tone, className)}
      style={cfg.glow ? { boxShadow: cfg.glow } : undefined}
    >
      {cfg.pulse && <i className="dot" aria-hidden />}
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
