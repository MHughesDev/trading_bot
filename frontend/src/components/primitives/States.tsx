import type { ReactNode } from 'react'
import { AlertTriangle, Inbox, RotateCw, SearchX, WifiOff } from 'lucide-react'
import { cn } from '@/lib/utils'
import { Button } from './Button'

/* Spec §3.22 — every screen ships its empty, loading, partial, error and
   stale states. A component with an undesigned state is not complete. */

export function EmptyState({
  icon,
  message,
  detail,
  action,
  className,
}: {
  icon?: ReactNode
  message: ReactNode
  detail?: ReactNode
  action?: ReactNode
  className?: string
}) {
  return (
    <div className={cn('empty', className)}>
      <span className="ico">{icon ?? <Inbox size={20} aria-hidden />}</span>
      <span className="msg">{message}</span>
      {detail && <span className="sub">{detail}</span>}
      {action}
    </div>
  )
}

/** Filtered to nothing — names the query and offers to clear it (§3.22). */
export function NoResults({
  query,
  onClear,
  className,
}: {
  query: string
  onClear: () => void
  className?: string
}) {
  return (
    <EmptyState
      className={className}
      icon={<SearchX size={20} aria-hidden />}
      message={
        <>
          No results for <em>{query}</em>
        </>
      }
      action={
        <Button size="sm" onClick={onClear}>
          Clear filters
        </Button>
      }
    />
  )
}

/** Panel-scoped failure. Never a toast for a panel-scoped error (§3.22). */
export function ErrorState({
  message,
  detail,
  onRetry,
  className,
}: {
  message?: ReactNode
  detail?: ReactNode
  onRetry?: () => void
  className?: string
}) {
  return (
    <div className={cn('inline-err', className)} role="alert">
      <span className="ico">
        <AlertTriangle size={20} aria-hidden />
      </span>
      <span className="msg">{message ?? 'This panel could not load.'}</span>
      {detail && <span className="msg mut">{detail}</span>}
      {onRetry && (
        <Button size="sm" icon={<RotateCw size={13} aria-hidden />} onClick={onRetry}>
          Retry
        </Button>
      )}
    </div>
  )
}

export function Skeleton({
  width,
  height,
  className,
  style,
}: {
  width?: number | string
  height?: number | string
  className?: string
  style?: React.CSSProperties
}) {
  return <span className={cn('skel', className)} style={{ width, height, display: 'block', ...style }} />
}

/** Skeleton list matching the final layout's shape and height. */
export function SkeletonRows({ rows = 5, height }: { rows?: number; height?: number }) {
  return (
    <div>
      {Array.from({ length: rows }).map((_, i) => (
        <div key={i} className="skel-row">
          <Skeleton width="34%" />
          <Skeleton width="18%" />
          <Skeleton width="22%" />
        </div>
      ))}
    </div>
  )
}

/** Loading state for a whole panel body — skeletons, never a spinner (§3.22). */
export function PanelLoading({ lines = 4 }: { lines?: number }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)', padding: 'var(--s-1) 0' }}>
      {Array.from({ length: lines }).map((_, i) => (
        <Skeleton key={i} width={`${92 - i * 11}%`} height={12} />
      ))}
    </div>
  )
}

/** Age badge that accompanies a stale figure (§5.2). */
export function StaleBadge({ age }: { age: string }) {
  return (
    <span className="badge warn" title={`Last update ${age} ago`}>
      <AlertTriangle size={9} aria-hidden />
      {age} old
    </span>
  )
}

export type ConnectionState = 'connected' | 'degraded' | 'reconnecting' | 'disconnected'

/** Persistent banner shown while the market-data feed is not healthy (§5.2). */
export function DisconnectedBanner({
  state,
  detail,
  onRetry,
}: {
  state: ConnectionState
  detail?: ReactNode
  onRetry?: () => void
}) {
  if (state === 'connected') return null
  const tone = state === 'disconnected' ? 'neg' : 'warn'
  const text =
    state === 'disconnected'
      ? 'Market data disconnected. Figures show their last known value and are marked stale.'
      : state === 'reconnecting'
        ? 'Reconnecting to market data…'
        : 'Market data delayed. Figures may lag the venue.'
  return (
    <div className={cn('banner', tone)} role="status" aria-live="polite">
      <WifiOff size={13} aria-hidden />
      <span>{text}</span>
      {detail}
      <span className="spacer" />
      {onRetry && (
        <Button size="xs" variant="ghost" onClick={onRetry}>
          Reconnect now
        </Button>
      )}
    </div>
  )
}
