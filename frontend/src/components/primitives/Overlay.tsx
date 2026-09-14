import { useEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Check, ChevronDown } from 'lucide-react'
import { cn } from '@/lib/utils'

/* =============================================================================
   Tooltip (§3.16) · Popover / menu (§3.17) · Select (§3.7)
   Lightweight, portal-based, keyboard-complete. No interactive tooltips.
   ============================================================================= */

type Side = 'top' | 'bottom' | 'left' | 'right'

function anchorRect(el: HTMLElement | null) {
  return el?.getBoundingClientRect() ?? null
}

function place(rect: DOMRect | null, side: Side, offset = 8) {
  if (!rect) return { top: 0, left: 0 }
  switch (side) {
    case 'top': return { top: rect.top - offset, left: rect.left + rect.width / 2, transform: 'translate(-50%,-100%)' }
    case 'bottom': return { top: rect.bottom + offset, left: rect.left + rect.width / 2, transform: 'translate(-50%,0)' }
    case 'left': return { top: rect.top + rect.height / 2, left: rect.left - offset, transform: 'translate(-100%,-50%)' }
    default: return { top: rect.top + rect.height / 2, left: rect.right + offset, transform: 'translate(0,-50%)' }
  }
}

/** 300ms open delay, 0 close. Never contains interactive content. */
export function Tooltip({
  content,
  side = 'top',
  children,
  disabled,
}: {
  content: ReactNode
  side?: Side
  children: ReactNode
  disabled?: boolean
}) {
  const ref = useRef<HTMLSpanElement>(null)
  const [shown, setShown] = useState(false)
  const timer = useRef<number>(0)

  useEffect(() => () => window.clearTimeout(timer.current), [])

  if (disabled || content === null || content === undefined) return <>{children}</>

  const open = () => {
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => setShown(true), 300)
  }
  const close = () => {
    window.clearTimeout(timer.current)
    setShown(false)
  }

  const pos = shown ? place(anchorRect(ref.current), side) : null

  return (
    <>
      <span
        ref={ref}
        style={{ display: 'contents' }}
        onMouseEnter={open}
        onMouseLeave={close}
        onFocus={open}
        onBlur={close}
      >
        {children}
      </span>
      {shown &&
        pos &&
        createPortal(
          <div role="tooltip" className="tooltip" style={{ position: 'fixed', ...pos }}>
            {content}
          </div>,
          document.body,
        )}
    </>
  )
}

/** Popover / dropdown menu. Focus returns to the trigger, Esc closes. */
export function Popover({
  trigger,
  children,
  align = 'end',
  side = 'bottom',
  width,
  ariaLabel,
}: {
  trigger: (props: { onClick: () => void; ref: React.Ref<HTMLButtonElement>; 'aria-expanded': boolean }) => ReactNode
  children: (close: () => void) => ReactNode
  align?: 'start' | 'end'
  side?: 'bottom' | 'top'
  width?: number
  ariaLabel: string
}) {
  const triggerRef = useRef<HTMLButtonElement>(null)
  const panelRef = useRef<HTMLDivElement>(null)
  const [open, setOpen] = useState(false)

  useEffect(() => {
    if (!open) return
    function onDown(e: MouseEvent) {
      if (panelRef.current?.contains(e.target as Node)) return
      if (triggerRef.current?.contains(e.target as Node)) return
      setOpen(false)
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') {
        setOpen(false)
        triggerRef.current?.focus()
      }
    }
    document.addEventListener('mousedown', onDown)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', onDown)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  const rect = open ? anchorRect(triggerRef.current) : null
  const style: React.CSSProperties = rect
    ? {
        position: 'fixed',
        top: side === 'bottom' ? rect.bottom + 6 : undefined,
        bottom: side === 'top' ? window.innerHeight - rect.top + 6 : undefined,
        left: align === 'start' ? rect.left : undefined,
        right: align === 'end' ? window.innerWidth - rect.right : undefined,
        width,
        maxHeight: '60vh',
        overflowY: 'auto',
      }
    : {}

  return (
    <>
      {trigger({ onClick: () => setOpen((o) => !o), ref: triggerRef, 'aria-expanded': open })}
      {open &&
        createPortal(
          <div ref={panelRef} className="popover" role="menu" aria-label={ariaLabel} style={style}>
            {children(() => {
              setOpen(false)
              triggerRef.current?.focus()
            })}
          </div>,
          document.body,
        )}
    </>
  )
}

export function MenuItem({
  onClick,
  children,
  danger,
  selected,
  icon,
  disabled,
}: {
  onClick?: () => void
  children: ReactNode
  danger?: boolean
  selected?: boolean
  icon?: ReactNode
  disabled?: boolean
}) {
  return (
    <button
      type="button"
      role="menuitem"
      disabled={disabled}
      className={cn('menu-item', danger && 'danger', selected && 'on')}
      onClick={onClick}
    >
      {icon}
      {children}
      {selected && <Check size={13} style={{ marginLeft: 'auto' }} aria-hidden />}
    </button>
  )
}

export function MenuSeparator() {
  return <div className="menu-sep" role="separator" />
}

export function MenuLabel({ children }: { children: ReactNode }) {
  return (
    <div className="lbl" style={{ padding: '6px 8px 4px' }}>
      {children}
    </div>
  )
}

/* Spec §3.7 — select. Trigger matches Input, full keyboard, role=listbox. */
export interface SelectOption<T extends string> {
  value: T
  label: ReactNode
  /** Text used for type-ahead and the trigger when label is a node. */
  text?: string
  disabled?: boolean
  group?: string
}

export function Select<T extends string>({
  value,
  onChange,
  options,
  ariaLabel,
  placeholder = 'Select…',
  small,
  disabled,
  className,
  width,
}: {
  value: T | null
  onChange: (next: T) => void
  options: SelectOption<T>[]
  ariaLabel: string
  placeholder?: string
  small?: boolean
  disabled?: boolean
  className?: string
  width?: number
}) {
  const current = options.find((o) => o.value === value)
  const groups = options.reduce<Record<string, SelectOption<T>[]>>((acc, o) => {
    const g = o.group ?? ''
    ;(acc[g] ??= []).push(o)
    return acc
  }, {})

  return (
    <Popover
      ariaLabel={ariaLabel}
      align="start"
      width={width}
      trigger={({ onClick, ref, 'aria-expanded': expanded }) => (
        <button
          ref={ref}
          type="button"
          role="combobox"
          aria-haspopup="listbox"
          aria-expanded={expanded}
          aria-label={ariaLabel}
          disabled={disabled}
          onClick={onClick}
          className={cn('input', small && 'sm', disabled && 'disabled', className)}
          style={{ width: width ?? '100%', justifyContent: 'space-between', cursor: 'pointer', textAlign: 'left' }}
        >
          <span className={cn('truncate-1', !current && 'mut')} style={{ fontSize: small ? 'var(--t-12)' : 'var(--t-13)' }}>
            {current ? (current.text ?? current.label) : placeholder}
          </span>
          <ChevronDown size={13} className="mut" aria-hidden />
        </button>
      )}
    >
      {(close) => (
        <div role="listbox" aria-label={ariaLabel}>
          {Object.entries(groups).map(([g, opts]) => (
            <div key={g || '_'}>
              {g && <MenuLabel>{g}</MenuLabel>}
              {opts.map((o) => (
                <button
                  key={o.value}
                  type="button"
                  role="option"
                  aria-selected={o.value === value}
                  disabled={o.disabled}
                  className={cn('menu-item', o.value === value && 'on')}
                  onClick={() => {
                    onChange(o.value)
                    close()
                  }}
                >
                  <span className="truncate-1">{o.label}</span>
                  {o.value === value && <Check size={13} style={{ marginLeft: 'auto' }} aria-hidden />}
                </button>
              ))}
            </div>
          ))}
        </div>
      )}
    </Popover>
  )
}
