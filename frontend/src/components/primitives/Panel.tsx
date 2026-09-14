import type { ReactNode, HTMLAttributes } from 'react'
import { cn } from '@/lib/utils'

/* Spec §3.10 / §2.3.5 — panel anatomy.
   header 38px, hairline bottom · body scrolls · footer fixed.
   Tables and charts use `flush` and bleed to the panel edge. */

interface PanelProps extends HTMLAttributes<HTMLDivElement> {
  /** Panel sitting above another panel. */
  raised?: boolean
  /** Focused pane in a multi-pane layout (spec §4.2). */
  focused?: boolean
}

export function Panel({ raised, focused, className, ...rest }: PanelProps) {
  return (
    <div
      className={cn('panel', raised && 'raised', focused && 'focused', className)}
      {...rest}
    />
  )
}

interface PanelHeaderProps extends Omit<HTMLAttributes<HTMLDivElement>, 'title'> {
  title?: ReactNode
  /** Rendered right-aligned in the header. */
  actions?: ReactNode
  /** Rendered immediately after the title. */
  badges?: ReactNode
  tall?: boolean
}

export function PanelHeader({
  title,
  actions,
  badges,
  tall,
  className,
  children,
  ...rest
}: PanelHeaderProps) {
  return (
    <div className={cn('panel-hd', tall && 'tall', className)} {...rest}>
      {title !== undefined && <div className="panel-title truncate-1">{title}</div>}
      {badges}
      {children}
      {actions !== undefined && (
        <>
          <div className="spacer" />
          {actions}
        </>
      )}
    </div>
  )
}

interface PanelBodyProps extends HTMLAttributes<HTMLDivElement> {
  /** Zero padding — required for tables and charts (§2.3.5). */
  flush?: boolean
  /** Body does not scroll; an inner element owns the scroll. */
  clip?: boolean
}

export function PanelBody({ flush, clip, className, ...rest }: PanelBodyProps) {
  return <div className={cn('panel-bd', flush && 'flush', clip && 'clip', className)} {...rest} />
}

export function PanelFooter({ className, ...rest }: HTMLAttributes<HTMLDivElement>) {
  return <div className={cn('panel-ft', className)} {...rest} />
}
