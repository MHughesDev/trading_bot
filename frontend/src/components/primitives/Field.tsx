import {
  forwardRef,
  useId,
  type InputHTMLAttributes,
  type ReactNode,
  type TextareaHTMLAttributes,
} from 'react'
import { AlertCircle, Search } from 'lucide-react'
import { cn } from '@/lib/utils'

/* Spec §3.5 — input / number field.
   Number fields: accept pasted separators, step with arrows (x10 with Shift),
   clamp on blur, never reformat mid-typing. */

interface FieldShellProps {
  label?: ReactNode
  /** Right-aligned hint inside the label row. */
  labelAside?: ReactNode
  /** Inline, field-adjacent error message (§3.13 MUST). */
  error?: string | null
  hint?: ReactNode
  className?: string
  children: ReactNode
  htmlFor?: string
}

export function Field({ label, labelAside, error, hint, className, children, htmlFor }: FieldShellProps) {
  return (
    <div className={cn('field', className)}>
      {(label !== undefined || labelAside !== undefined) && (
        <label className="lbl" htmlFor={htmlFor}>
          <span>{label}</span>
          {labelAside}
        </label>
      )}
      {children}
      {error ? (
        <span className="field-err" role="alert">
          <AlertCircle size={11} aria-hidden />
          {error}
        </span>
      ) : hint ? (
        <span className="field-hint">{hint}</span>
      ) : null}
    </div>
  )
}

export interface InputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'size'> {
  label?: ReactNode
  labelAside?: ReactNode
  error?: string | null
  hint?: ReactNode
  /** Right-aligned unit suffix, e.g. `USD`, `BTC`, `bars`. */
  unit?: ReactNode
  /** Leading adornment (icon). */
  lead?: ReactNode
  /** Tabular numerals — set automatically by NumberField. */
  numeric?: boolean
  small?: boolean
  wrapperClassName?: string
}

export const Input = forwardRef<HTMLInputElement, InputProps>(function Input(
  { label, labelAside, error, hint, unit, lead, numeric, small, className, wrapperClassName, id, ...rest },
  ref,
) {
  const autoId = useId()
  const inputId = id ?? autoId
  const body = (
    <div className={cn('input', small && 'sm', error && 'err', rest.disabled && 'disabled', wrapperClassName)}>
      {lead && <span className="lead">{lead}</span>}
      <input
        ref={ref}
        id={inputId}
        aria-invalid={error ? true : undefined}
        className={cn(numeric && 'n', className)}
        {...rest}
      />
      {unit && <span className="unit">{unit}</span>}
    </div>
  )
  if (label === undefined && labelAside === undefined && !error && !hint) return body
  return (
    <Field label={label} labelAside={labelAside} error={error} hint={hint} htmlFor={inputId}>
      {body}
    </Field>
  )
})

export interface NumberFieldProps extends Omit<InputProps, 'onChange' | 'value' | 'type'> {
  value: string
  onValueChange: (next: string) => void
  /** Smallest increment. Shift multiplies by 10. */
  step?: number
  min?: number
  max?: number
  /** Fixed decimals applied on blur — comes from instrument metadata. */
  dp?: number
}

export const NumberField = forwardRef<HTMLInputElement, NumberFieldProps>(function NumberField(
  { value, onValueChange, step = 1, min, max, dp, ...rest },
  ref,
) {
  function clamp(n: number): number {
    let out = n
    if (min !== undefined) out = Math.max(min, out)
    if (max !== undefined) out = Math.min(max, out)
    return out
  }

  return (
    <Input
      ref={ref}
      numeric
      inputMode="decimal"
      autoComplete="off"
      value={value}
      // Accept pasted separators; never reformat while the user is typing.
      onChange={(e) => onValueChange(e.target.value.replace(/[,\s_]/g, ''))}
      onKeyDown={(e) => {
        if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return
        e.preventDefault()
        const current = Number(value || 0)
        if (!Number.isFinite(current)) return
        const mult = e.shiftKey ? 10 : 1
        const next = clamp(current + (e.key === 'ArrowUp' ? step : -step) * mult)
        onValueChange(dp === undefined ? String(next) : next.toFixed(dp))
      }}
      onBlur={(e) => {
        const n = Number(value)
        if (value !== '' && Number.isFinite(n)) {
          const c = clamp(n)
          onValueChange(dp === undefined ? String(c) : c.toFixed(dp))
        }
        rest.onBlur?.(e)
      }}
      {...rest}
    />
  )
})

export interface TextAreaProps extends TextareaHTMLAttributes<HTMLTextAreaElement> {
  label?: ReactNode
  error?: string | null
  hint?: ReactNode
}

export const TextArea = forwardRef<HTMLTextAreaElement, TextAreaProps>(function TextArea(
  { label, error, hint, className, id, rows = 4, ...rest },
  ref,
) {
  const autoId = useId()
  const areaId = id ?? autoId
  const body = <textarea ref={ref} id={areaId} rows={rows} className={cn('ta', className)} {...rest} />
  if (label === undefined && !error && !hint) return body
  return (
    <Field label={label} error={error} hint={hint} htmlFor={areaId}>
      {body}
    </Field>
  )
})

export function SearchInput({
  value,
  onValueChange,
  placeholder = 'Search',
  className,
  autoFocus,
  onKeyDown,
  'aria-label': ariaLabel,
}: {
  value: string
  onValueChange: (next: string) => void
  placeholder?: string
  className?: string
  autoFocus?: boolean
  onKeyDown?: (e: React.KeyboardEvent<HTMLInputElement>) => void
  'aria-label'?: string
}) {
  return (
    <div className={cn('searchbox', className)}>
      <Search size={13} aria-hidden />
      <input
        type="search"
        value={value}
        autoFocus={autoFocus}
        aria-label={ariaLabel ?? placeholder}
        placeholder={placeholder}
        onChange={(e) => onValueChange(e.target.value)}
        onKeyDown={onKeyDown}
      />
    </div>
  )
}

/* Spec §3.8 — toggle / switch.
   A toggle that changes money behaviour needs a text label and must not be the
   only confirmation. */
export function Switch({
  checked,
  onCheckedChange,
  label,
  disabled,
  className,
}: {
  checked: boolean
  onCheckedChange: (next: boolean) => void
  label: string
  disabled?: boolean
  className?: string
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onCheckedChange(!checked)}
      className={cn('switch', className)}
    />
  )
}

/** A labelled switch row, the shape Settings uses throughout. */
export function SwitchRow({
  label,
  description,
  checked,
  onCheckedChange,
  disabled,
}: {
  label: string
  description?: ReactNode
  checked: boolean
  onCheckedChange: (next: boolean) => void
  disabled?: boolean
}) {
  return (
    <div className="between" style={{ padding: 'var(--s-2) 0' }}>
      <div style={{ minWidth: 0 }}>
        <div style={{ fontSize: 'var(--t-13)', fontWeight: 'var(--w-medium)' }}>{label}</div>
        {description && (
          <div className="mut" style={{ fontSize: 'var(--t-12)', marginTop: 2, lineHeight: 1.45 }}>
            {description}
          </div>
        )}
      </div>
      <Switch checked={checked} onCheckedChange={onCheckedChange} label={label} disabled={disabled} />
    </div>
  )
}
