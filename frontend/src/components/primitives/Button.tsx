import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from 'react'
import { cn } from '@/lib/utils'

/* Spec §3.1 — variants: primary · secondary (default) · ghost · danger · buy · sell
   Sizes: xs · sm · md (default) · lg · xl (submit only)
   All eight states live in app.css: default, hover, active, focus-visible,
   selected, disabled, loading, error. */

export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger' | 'buy' | 'sell'
export type ButtonSize = 'xs' | 'sm' | 'md' | 'lg' | 'xl'

export interface ButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'type'> {
  variant?: ButtonVariant
  size?: ButtonSize
  /** Spinner replaces the leading icon; the label stays so width never collapses. */
  loading?: boolean
  /** Selected / toggled-on state. */
  active?: boolean
  block?: boolean
  icon?: ReactNode
  trailing?: ReactNode
  type?: 'button' | 'submit' | 'reset'
}

const VARIANT: Record<ButtonVariant, string> = {
  primary: 'primary',
  secondary: '',
  ghost: 'ghost',
  danger: 'danger',
  buy: 'buy',
  sell: 'sell',
}

const SIZE: Record<ButtonSize, string> = {
  xs: 'xs',
  sm: 'sm',
  md: '',
  lg: 'lg',
  xl: 'xl',
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  {
    variant = 'secondary',
    size = 'md',
    loading = false,
    active = false,
    block = false,
    icon,
    trailing,
    className,
    children,
    disabled,
    type = 'button',
    ...rest
  },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      className={cn('btn', VARIANT[variant], SIZE[size], block && 'block', active && 'on', className)}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      aria-pressed={active || undefined}
      {...rest}
    >
      {loading ? <span className="spinner" aria-hidden /> : icon}
      {children}
      {trailing}
    </button>
  )
})

/* Spec §3.2 — icon button. Always aria-label + tooltip at the call site. */
export interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'type'> {
  label: string
  active?: boolean
  bare?: boolean
  size?: 'sm' | 'md'
  type?: 'button' | 'submit' | 'reset'
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, active, bare, size = 'sm', className, children, type = 'button', ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      aria-label={label}
      title={label}
      aria-pressed={active || undefined}
      className={cn('iconbtn', size === 'md' && 'md', bare && 'bare', active && 'on', className)}
      {...rest}
    >
      {children}
    </button>
  )
})
