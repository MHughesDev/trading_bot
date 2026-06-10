import { cva } from 'class-variance-authority'

export const badgeVariants = cva(
  'inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-semibold transition-colors',
  {
    variants: {
      variant: {
        default: 'bg-blue-500/20 text-blue-400',
        active: 'bg-emerald-500/20 text-emerald-400',
        inactive: 'bg-border-2 text-text-muted',
        warning: 'bg-amber-500/20 text-amber-400',
        destructive: 'bg-red-500/20 text-red-400',
        outline: 'border border-border-2 text-text-muted',
      },
    },
    defaultVariants: { variant: 'default' },
  }
)
