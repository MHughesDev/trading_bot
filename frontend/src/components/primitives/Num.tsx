import { useEffect, useRef, useState, type ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { usePrefs } from '@/store/prefs'
import {
  DASH,
  dirArrow,
  dirClass,
  money as fmtMoney,
  pct as fmtPct,
  price as fmtPrice,
  qty as fmtQty,
  signed as fmtSigned,
  signedMoney as fmtSignedMoney,
  signedPct as fmtSignedPct,
  toNum,
  type Numish,
} from '@/lib/format'

/* =============================================================================
   Number display primitives.
   Every changing number in the product goes through one of these.

   Enforced here:
     - --font-numeric + tabular-nums (spec N1)
     - formatting only via lib/format.ts (spec §5.3)
     - direction never carried by colour alone (spec §1.3.3 / A4)
     - tick flash is background-only, position never animates (spec M1/M2)
     - stale renders at 60% with an age badge, never hidden or zeroed (N8)
   ============================================================================= */

interface BaseNumProps {
  className?: string
  /** Flash the cell background when the value changes (spec §5.1). */
  flash?: boolean
  /** Render at 60% opacity — the quote is older than its freshness budget. */
  stale?: boolean
  title?: string
}

function useTickFlash(value: unknown, requested: boolean) {
  // The user can switch the flash off entirely (spec §2.6 M2).
  const allowed = usePrefs((s) => s.tickFlash)
  const enabled = requested && allowed
  const prev = useRef(value)
  const [dir, setDir] = useState<'up' | 'down' | null>(null)

  useEffect(() => {
    if (!enabled) {
      prev.current = value
      return
    }
    const a = toNum(prev.current as Numish)
    const b = toNum(value as Numish)
    prev.current = value
    if (a === null || b === null || a === b) return
    setDir(b > a ? 'up' : 'down')
    const t = window.setTimeout(() => setDir(null), 240)
    return () => window.clearTimeout(t)
  }, [value, enabled])

  return dir
}

/** A raw tabular number. Use a specific variant below wherever one fits. */
export function Num({
  children,
  className,
  flash,
  stale,
  ...rest
}: BaseNumProps & { children: ReactNode }) {
  const dir = useTickFlash(children, !!flash)
  return (
    <span
      className={cn('num', stale && 'stale', dir === 'up' && 'flash-up', dir === 'down' && 'flash-down', className)}
      {...rest}
    >
      {children}
    </span>
  )
}

/** A price for a specific instrument — precision comes from instrument metadata. */
export function Price({
  instrument,
  value,
  className,
  flash,
  stale,
}: BaseNumProps & { instrument: string | undefined; value: Numish }) {
  const dir = useTickFlash(value, !!flash)
  return (
    <span
      className={cn('num', stale && 'stale', dir === 'up' && 'flash-up', dir === 'down' && 'flash-down', className)}
    >
      {fmtPrice(instrument, value)}
    </span>
  )
}

/** A size / quantity for a specific instrument. */
export function Qty({
  instrument,
  value,
  className,
  stale,
}: BaseNumProps & { instrument: string | undefined; value: Numish }) {
  return <span className={cn('num', stale && 'stale', className)}>{fmtQty(instrument, value)}</span>
}

/** A money figure at full precision — execution-path safe. */
export function Money({
  value,
  ccy = 'USD',
  dp = 2,
  className,
  stale,
}: BaseNumProps & { value: Numish; ccy?: string; dp?: number }) {
  return <span className={cn('num', stale && 'stale', className)}>{fmtMoney(value, ccy, dp)}</span>
}

/**
 * A P&L figure. Always carries an explicit sign in addition to colour (§2.1.5),
 * and zero renders neutral with no sign.
 */
export function Pnl({
  value,
  ccy = 'USD',
  dp = 2,
  bare = false,
  className,
  flash,
  stale,
}: BaseNumProps & { value: Numish; ccy?: string; dp?: number; bare?: boolean }) {
  const dir = useTickFlash(value, !!flash)
  const text = bare ? fmtSigned(value, dp) : fmtSignedMoney(value, ccy, dp)
  return (
    <span
      className={cn(
        'num',
        dirClass(value),
        stale && 'stale',
        dir === 'up' && 'flash-up',
        dir === 'down' && 'flash-down',
        className,
      )}
    >
      {text}
    </span>
  )
}

/** A percentage. `signed` adds the sign; otherwise an arrow glyph carries direction. */
export function Pct({
  value,
  dp = 2,
  signed = true,
  directional = true,
  arrow = false,
  className,
  stale,
}: BaseNumProps & {
  value: Numish
  dp?: number
  /** Prefix + / U+2212. */
  signed?: boolean
  /** Paint with the direction token. */
  directional?: boolean
  /** Show a ▲ / ▼ glyph — required when no sign is shown (§2.1.5). */
  arrow?: boolean
}) {
  const alwaysGlyph = usePrefs((s) => s.alwaysShowDirectionGlyph)
  const n = toNum(value)
  if (n === null) return <span className={cn('num mut', className)}>{DASH}</span>
  const text = signed ? fmtSignedPct(value, dp) : fmtPct(value, dp)
  const showGlyph = (arrow || alwaysGlyph) && n !== 0
  return (
    <span className={cn('num', directional && dirClass(value), stale && 'stale', className)}>
      {showGlyph && <span aria-hidden>{dirArrow(value)} </span>}
      {text}
    </span>
  )
}

/** A micro-label. Consumes all four --label-* tokens (spec §2.2.4). */
export function Label({ className, children, ...rest }: { className?: string; children: ReactNode } & Record<string, unknown>) {
  return (
    <span className={cn('lbl', className)} {...rest}>
      {children}
    </span>
  )
}

/* Spec §3.12 — stat tile. The value MUST be the largest thing in the tile. */
export function StatTile({
  label,
  value,
  meta,
  tone,
  className,
}: {
  label: ReactNode
  value: ReactNode
  meta?: ReactNode
  tone?: 'pos' | 'neg' | 'warn' | 'accent'
  className?: string
}) {
  return (
    <div className={cn('stat', className)}>
      <span className="k">{label}</span>
      <span className={cn('v', tone)}>{value}</span>
      {meta !== undefined && meta !== null && <span className="m">{meta}</span>}
    </div>
  )
}
