import { useRef, type ReactNode } from 'react'
import { cn } from '@/lib/utils'

/* Spec §3.3 — segmented control. role=radiogroup, arrows move selection.
   The buy/sell variant tints from the direction families, never from accent. */

export interface SegmentedOption<T extends string> {
  value: T
  label: ReactNode
  /** Buy/sell tint for the `.bs` variant. */
  side?: 'buy' | 'sell'
  disabled?: boolean
  title?: string
}

export function Segmented<T extends string>({
  value,
  onChange,
  options,
  ariaLabel,
  variant = 'default',
  wide,
  className,
}: {
  value: T
  onChange: (next: T) => void
  options: SegmentedOption<T>[]
  ariaLabel: string
  variant?: 'default' | 'buysell'
  wide?: boolean
  className?: string
}) {
  const ref = useRef<HTMLDivElement>(null)

  function move(delta: number) {
    const enabled = options.filter((o) => !o.disabled)
    const i = enabled.findIndex((o) => o.value === value)
    if (i < 0) return
    const next = enabled[(i + delta + enabled.length) % enabled.length]
    onChange(next.value)
    const btns = ref.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')
    btns?.[(i + delta + enabled.length) % enabled.length]?.focus()
  }

  return (
    <div
      ref={ref}
      role="radiogroup"
      aria-label={ariaLabel}
      className={cn('seg', variant === 'buysell' && 'bs', wide && 'wide', className)}
      onKeyDown={(e) => {
        if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
          e.preventDefault()
          move(1)
        } else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
          e.preventDefault()
          move(-1)
        }
      }}
    >
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          disabled={o.disabled}
          title={o.title}
          tabIndex={o.value === value ? 0 : -1}
          className={cn(o.value === value && 'on', o.value === value && o.side)}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  )
}

/* Spec §3.4 — tabs. role=tablist, arrows + Home/End, aria-selected. */
export interface TabDef<T extends string> {
  value: T
  label: ReactNode
  /** Count badge riding alongside the label. */
  count?: number
  disabled?: boolean
}

export function Tabs<T extends string>({
  value,
  onChange,
  tabs,
  ariaLabel,
  right,
  className,
}: {
  value: T
  onChange: (next: T) => void
  tabs: TabDef<T>[]
  ariaLabel: string
  /** Right-aligned content in the tab bar (meta, filters). */
  right?: ReactNode
  className?: string
}) {
  const ref = useRef<HTMLDivElement>(null)

  function focusAt(i: number) {
    const btns = ref.current?.querySelectorAll<HTMLButtonElement>('button[role="tab"]:not(:disabled)')
    if (!btns?.length) return
    const idx = (i + btns.length) % btns.length
    btns[idx].focus()
    btns[idx].click()
  }

  const enabled = tabs.filter((t) => !t.disabled)

  return (
    <div ref={ref} className={cn('tabs', className)} role="tablist" aria-label={ariaLabel}>
      {tabs.map((t) => (
        <button
          key={t.value}
          type="button"
          role="tab"
          aria-selected={t.value === value}
          tabIndex={t.value === value ? 0 : -1}
          disabled={t.disabled}
          className={cn(t.value === value && 'on')}
          onClick={() => onChange(t.value)}
          onKeyDown={(e) => {
            const i = enabled.findIndex((x) => x.value === value)
            if (e.key === 'ArrowRight') { e.preventDefault(); focusAt(i + 1) }
            else if (e.key === 'ArrowLeft') { e.preventDefault(); focusAt(i - 1) }
            else if (e.key === 'Home') { e.preventDefault(); focusAt(0) }
            else if (e.key === 'End') { e.preventDefault(); focusAt(enabled.length - 1) }
          }}
        >
          {t.label}
          {t.count !== undefined && <span className="badge count neutral">{t.count}</span>}
        </button>
      ))}
      {right !== undefined && (
        <>
          <span className="spacer" />
          {right}
        </>
      )}
    </div>
  )
}

/* Spec §3.6 — percent-of-buying-power row. */
export const PCT_STEPS = [10, 25, 50, 75, 100] as const

export function PercentRow({
  value,
  onChange,
  steps = PCT_STEPS as unknown as number[],
  className,
}: {
  /** null when the user has typed a size directly. */
  value: number | null
  onChange: (pct: number) => void
  steps?: number[]
  className?: string
}) {
  return (
    <div className={cn('pctrow', className)} role="group" aria-label="Percent of buying power">
      {steps.map((s) => (
        <button
          key={s}
          type="button"
          aria-pressed={value === s}
          className={cn(value === s && 'on')}
          onClick={() => onChange(s)}
        >
          {s === 100 ? 'Max' : `${s}%`}
        </button>
      ))}
    </div>
  )
}
