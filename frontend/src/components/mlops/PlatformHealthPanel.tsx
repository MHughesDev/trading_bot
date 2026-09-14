import { useQuery } from '@tanstack/react-query'
import { motion, useReducedMotion } from 'framer-motion'
import { AlertTriangle, CircleHelp, CircleOff, CircleSlash, CircleCheck } from 'lucide-react'
import { cn } from '@/lib/utils'
import {
  platformHealthApi,
  valueLabel,
  type Signal,
  type SignalState,
} from '@/api/platformHealth'

/**
 * The platform's self-monitoring panel (SPEC §16.2, ADR-P5-01, AT-69).
 *
 * §16.2 calls this "the layer nobody builds", and the reason it is worth
 * building is the reason it is usually skipped: it is the only surface whose job
 * is to say whether the rest of the platform's judgment is working. A dashboard
 * that shows a confident 0 % for a model that was never trained is worse than no
 * dashboard, because it converts an absence of evidence into a number people
 * read as evidence.
 *
 * So this component renders **five** states and never collapses them to two. A
 * signal with no value shows a word, not a bar: `not fitted`, `n/a`,
 * `unavailable`. The detail line is always present, and in those three states it
 * is the whole answer.
 */

/** Foreground colour per state, from the semantic token layer. */
const TONE_FG: Record<SignalState, string> = {
  ok: 'text-[var(--fg-pos)]',
  alarm: 'text-[var(--fg-neg)]',
  not_fitted: 'text-[var(--fg-tertiary)]',
  not_applicable: 'text-[var(--fg-tertiary)]',
  unavailable: 'text-[var(--fg-warn)]',
}

const STATE_CONFIG: Record<
  SignalState,
  { tone: string; label: string; Icon: React.ElementType }
> = {
  ok: { tone: 'pos', label: 'OK', Icon: CircleCheck },
  alarm: { tone: 'neg', label: 'Alarm', Icon: AlertTriangle },
  // Deliberately `neutral`, not `warn`: an unfitted model is not a problem with
  // the platform, it is a thing that has not happened yet. Colouring it as a
  // fault trains readers to ignore the colour.
  not_fitted: { tone: 'neutral', label: 'Not fitted', Icon: CircleHelp },
  not_applicable: { tone: 'neutral', label: 'N/A', Icon: CircleSlash },
  // `warn`, not `neg`: the platform could not compute this, which is a fault in
  // the monitoring rather than a finding about the thing monitored. Reading it
  // as an alarm would make a broken query indistinguishable from a real breach.
  unavailable: { tone: 'warn', label: 'Unavailable', Icon: CircleOff },
}

function SignalRow({ signal }: { signal: Signal }) {
  const shouldReduce = useReducedMotion()
  const cfg = STATE_CONFIG[signal.state]
  const Icon = cfg.Icon

  return (
    <motion.li
      layout={!shouldReduce}
      className="flex items-start gap-3 border-b border-[var(--line-subtle)] py-3 last:border-b-0"
      data-signal={signal.id}
      data-state={signal.state}
    >
      <Icon
        className={cn('mt-0.5 h-4 w-4 shrink-0', TONE_FG[signal.state])}
        aria-hidden
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between gap-3">
          <span className="truncate text-sm font-medium">{signal.title}</span>
          <span
            className={cn(
              'shrink-0 font-mono text-sm tabular-nums',
              signal.state === 'alarm' && 'text-[var(--fg-neg)]',
              (signal.state === 'not_fitted' || signal.state === 'not_applicable') &&
                'text-[var(--fg-secondary)] italic',
              signal.state === 'unavailable' && 'text-[var(--fg-warn)] italic',
            )}
            title={
              signal.threshold !== null ? `threshold ${signal.threshold}` : undefined
            }
          >
            {valueLabel(signal)}
          </span>
        </div>
        {/* Always shown. For the three valueless states it *is* the answer. */}
        <p className="mt-0.5 text-xs text-[var(--fg-secondary)]">{signal.detail}</p>
        <p className="mt-0.5 font-mono text-[10px] text-[var(--fg-tertiary)]">
          {signal.spec_ref}
          {signal.severity === 'p1' ? ' · P1' : ' · P2'}
        </p>
      </div>
      <span className={cn('badge shrink-0', cfg.tone)}>{cfg.label}</span>
    </motion.li>
  )
}

export function PlatformHealthPanel({ className }: { className?: string }) {
  const { data, isLoading, isError, error } = useQuery({
    queryKey: ['platform', 'health'],
    queryFn: () => platformHealthApi.get().then((r) => r.data),
    // The backing job runs hourly; a minute of staleness on a self-monitoring
    // panel is not a problem, and polling harder would put a query on the ledger
    // every time somebody looks at the page.
    staleTime: 60_000,
    refetchInterval: 300_000,
  })

  if (isLoading) {
    return (
      <section className={cn('panel p-4', className)}>
        <h2 className="text-sm font-semibold">Platform health</h2>
        <div className="mt-3 space-y-2" aria-busy>
          {Array.from({ length: 6 }).map((_, i) => (
            <div key={i} className="h-10 animate-pulse rounded bg-[var(--bg-surface-raised)]" />
          ))}
        </div>
      </section>
    )
  }

  if (isError || !data) {
    // The panel that reports on everything else has to report on itself. Showing
    // nothing here would be indistinguishable from a healthy platform.
    return (
      <section className={cn('panel p-4', className)}>
        <h2 className="text-sm font-semibold">Platform health</h2>
        <p className="mt-2 text-sm text-[var(--fg-warn)]">
          The health report could not be loaded, so nothing on this panel is known —
          not even that the platform is fine.
        </p>
        <p className="mt-1 font-mono text-xs text-[var(--fg-secondary)]">
          {error instanceof Error ? error.message : 'unknown error'}
        </p>
      </section>
    )
  }

  const alarms = data.signals.filter((s) => s.state === 'alarm')
  const unavailable = data.signals.filter((s) => s.state === 'unavailable')
  const unfitted = data.signals.filter(
    (s) => s.state === 'not_fitted' || s.state === 'not_applicable',
  )

  return (
    <section className={cn('panel p-4', className)}>
      <header className="flex items-baseline justify-between gap-3">
        <h2 className="text-sm font-semibold">Platform health</h2>
        <span className="font-mono text-xs text-[var(--fg-secondary)]">
          {data.window_days}d window · {new Date(data.generated_at).toLocaleString()}
        </span>
      </header>

      {/* The summary counts the three groups separately on purpose. "14 of 16
          healthy" would fold an unfitted model and a broken query into the same
          number as a real pass. */}
      <p className="mt-1 text-xs text-[var(--fg-secondary)]">
        {alarms.length} in alarm · {unavailable.length} unavailable · {unfitted.length} with
        nothing fitted yet · {data.signals.length} signals
      </p>

      <ul className="mt-2">
        {data.signals.map((s) => (
          <SignalRow key={s.id} signal={s} />
        ))}
      </ul>
    </section>
  )
}
