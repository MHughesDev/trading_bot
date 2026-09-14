import type { HTMLAttributes, ReactNode } from 'react'
import { Link } from 'react-router-dom'
import { cn } from '@/lib/utils'
import { assetClassColor, seriesColor, type AssetClass } from '@/lib/format'

/* Spec §3.9 — badge and chip.
   A badge MUST contain a word, never colour alone. */

export type BadgeTone = 'pos' | 'neg' | 'warn' | 'info' | 'accent' | 'neutral' | 'outline'

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: BadgeTone
  /** Renders as a router link — used by the positions `Source` badge (§4.2). */
  to?: string
}

export function Badge({ tone = 'neutral', to, className, children, ...rest }: BadgeProps) {
  const cls = cn('badge', tone, className)
  if (to) {
    return (
      <Link to={to} className={cls}>
        {children}
      </Link>
    )
  }
  return (
    <span className={cls} {...rest}>
      {children}
    </span>
  )
}

/** A count that rides alongside a tab or title. */
export function CountBadge({ value, className }: { value: number; className?: string }) {
  return <span className={cn('badge count neutral', className)}>{value}</span>
}

/**
 * Asset-class identity in chrome is a NEUTRAL chip + label (§2.1.7).
 * Hue is reserved for data visualisation, where it is slot-assigned.
 */
export function Chip({ className, children, ...rest }: HTMLAttributes<HTMLSpanElement>) {
  return (
    <span className={cn('chip', className)} {...rest}>
      {children}
    </span>
  )
}

/** The small square that identifies a series in a legend or identity cell. */
export function Swatch({
  slot,
  assetClass,
  color,
  large,
  className,
}: {
  slot?: number
  assetClass?: AssetClass
  color?: string
  large?: boolean
  className?: string
}) {
  const background = color ?? (assetClass ? assetClassColor(assetClass) : seriesColor(slot ?? 1))
  return (
    <i
      aria-hidden
      className={cn('swatch', large && 'lg', className)}
      style={{ background }}
    />
  )
}

/** Status dot. Never the only carrier of state — always pair with a word. */
export function StatusDot({
  state,
  pulsing,
  className,
}: {
  state: 'live' | 'degraded' | 'down' | 'neutral'
  pulsing?: boolean
  className?: string
}) {
  return (
    <i
      aria-hidden
      className={cn('dot', state !== 'neutral' && state, pulsing && 'pulsing', className)}
    />
  )
}

/** A key/value line used inside wells and inspectors. */
export function KeyValue({
  k,
  v,
  tone,
  className,
}: {
  k: ReactNode
  v: ReactNode
  tone?: 'pos' | 'neg' | 'warn' | 'accent'
  className?: string
}) {
  return (
    <div className={cn('kv', className)}>
      <span className="k">{k}</span>
      <span className={cn('v', tone)}>{v}</span>
    </div>
  )
}
