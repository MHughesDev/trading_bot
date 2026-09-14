import { useQuery } from '@tanstack/react-query'
import { Check, X, CircleDashed } from 'lucide-react'
import { api as client } from '@/lib/api'
import { cn } from '@/lib/utils'

/**
 * The sixteen-gate board (SPEC §12.3; plan 5.5).
 *
 * Two things this renders that a pass/fail list would not, and both are the
 * point:
 *
 * **The trial count beside every number.** A Sharpe of 1.8 after four looks and
 * a Sharpe of 1.8 after four thousand are different claims, and §12.2's whole
 * argument is that the second one is mostly the search. So `N_eff` and the trial
 * count sit next to the statistic on every row, not in a footnote.
 *
 * **How many gates actually ran.** A stack with twelve recorded verdicts is not
 * a stack that failed four — it is a stack where four gates had no evidence to
 * run against. Padding to sixteen would make those two look identical, so the
 * header says "12 of 16 recorded" and the missing ones are shown as such.
 */

interface GateVerdict {
  gate_no: number
  gate_name: string
  passed: boolean
  statistic: number | null
  threshold: number | null
  detail: string
  profile_id: string
  n_eff: number | null
  trial_count_at_eval: number | null
  decided_at: string
}

interface GateStack {
  subject: string
  recorded: number
  of: number
  passed: number
  complete: boolean
  blocked_at: number | null
  profile_id: string | null
  verdicts: GateVerdict[]
}

/** §12.3's sixteen, in order. Names only — the thresholds are the profile's. */
const GATE_TITLES: Record<number, string> = {
  1: 'Pre-registration',
  2: 'Leakage suite',
  3: 'Cost sensitivity',
  4: 'Capacity / ADV',
  5: 'CPCV p05',
  6: 'Walk-forward across regimes',
  7: 'Probability of backtest overfitting',
  8: 'Deflated Sharpe',
  9: 'Minimum length',
  10: 'Factor attribution',
  11: 'Regime coverage',
  12: 'Perturbation / concentration',
  13: 'Stationary bootstrap',
  14: 'Romano–Wolf vs the family',
  15: 'Paper / shadow',
  16: 'Capital ramp',
}

function fmt(v: number | null): string {
  if (v === null || !Number.isFinite(v)) return '—'
  return Math.abs(v) >= 1000 ? v.toFixed(0) : v.toFixed(3)
}

function VerdictRow({ n, verdict }: { n: number; verdict?: GateVerdict }) {
  const title = GATE_TITLES[n] ?? `Gate ${n}`

  if (!verdict) {
    return (
      <li
        className="flex items-start gap-3 border-b border-[var(--line-subtle)] py-2 last:border-b-0 opacity-60"
        data-gate={n}
        data-state="not_recorded"
      >
        <CircleDashed className="mt-0.5 h-4 w-4 shrink-0 text-[var(--fg-tertiary)]" aria-hidden />
        <div className="min-w-0 flex-1">
          <div className="text-sm">
            <span className="font-mono text-[var(--fg-tertiary)]">{n}</span> {title}
          </div>
          {/* Not a failure. Nothing ran, and saying so is the whole reason this
              row exists rather than being omitted. */}
          <p className="text-xs italic text-[var(--fg-tertiary)]">
            not recorded — no verdict has been written for this gate
          </p>
        </div>
      </li>
    )
  }

  return (
    <li
      className="flex items-start gap-3 border-b border-[var(--line-subtle)] py-2 last:border-b-0"
      data-gate={n}
      data-state={verdict.passed ? 'passed' : 'failed'}
    >
      {verdict.passed ? (
        <Check className="mt-0.5 h-4 w-4 shrink-0 text-[var(--fg-pos)]" aria-hidden />
      ) : (
        <X className="mt-0.5 h-4 w-4 shrink-0 text-[var(--fg-neg)]" aria-hidden />
      )}
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between gap-3 text-sm">
          <span className="truncate">
            <span className="font-mono text-[var(--fg-tertiary)]">{verdict.gate_no}</span>{' '}
            {title}
          </span>
          <span className="shrink-0 font-mono tabular-nums">
            {fmt(verdict.statistic)}
            {verdict.threshold !== null && (
              <span className="text-[var(--fg-tertiary)]"> / {fmt(verdict.threshold)}</span>
            )}
          </span>
        </div>
        <p className="mt-0.5 text-xs text-[var(--fg-secondary)]">{verdict.detail}</p>
        {/* The counter beside the number (§12.2). Without it the statistic above
            is unreadable: it does not say how much searching produced it. */}
        <p className="mt-0.5 font-mono text-[10px] text-[var(--fg-tertiary)]">
          {verdict.profile_id}
          {verdict.n_eff !== null && ` · N_eff ${verdict.n_eff.toFixed(1)}`}
          {verdict.trial_count_at_eval !== null &&
            ` · ${verdict.trial_count_at_eval} trials at evaluation`}
        </p>
      </div>
    </li>
  )
}

export function GateStackBoard({
  subject,
  className,
}: {
  subject: string
  className?: string
}) {
  const { data, isLoading, isError } = useQuery({
    queryKey: ['platform', 'gates', subject],
    queryFn: () =>
      client.get<GateStack>(`/api/platform/gates/${encodeURIComponent(subject)}`).then((r) => r.data),
    enabled: !!subject,
    staleTime: 30_000,
  })

  if (isLoading) {
    return (
      <section className={cn('panel p-4', className)} aria-busy>
        <h2 className="text-sm font-semibold">Gate stack</h2>
        <div className="mt-3 h-40 animate-pulse rounded bg-[var(--bg-surface-raised)]" />
      </section>
    )
  }

  if (isError || !data) {
    return (
      <section className={cn('panel p-4', className)}>
        <h2 className="text-sm font-semibold">Gate stack</h2>
        <p className="mt-2 text-sm text-[var(--fg-warn)]">
          The gate stack could not be loaded. Nothing here is known — including whether
          this candidate passed.
        </p>
      </section>
    )
  }

  const byGate = new Map(data.verdicts.map((v) => [v.gate_no, v]))

  return (
    <section className={cn('panel p-4', className)}>
      <header className="flex flex-wrap items-baseline justify-between gap-2">
        <h2 className="text-sm font-semibold">Gate stack</h2>
        <span className="font-mono text-xs text-[var(--fg-secondary)]">
          {data.passed} passed · {data.recorded} of {data.of} recorded
          {data.profile_id && ` · ${data.profile_id}`}
        </span>
      </header>

      {!data.complete && (
        <p className="mt-1 text-xs text-[var(--fg-warn)]">
          {data.of - data.recorded} gate{data.of - data.recorded === 1 ? '' : 's'} have no
          recorded verdict. An unevaluated gate is not a passed one, and this stack
          authorises nothing until all {data.of} have run.
        </p>
      )}
      {data.complete && data.blocked_at !== null && (
        <p className="mt-1 text-xs text-[var(--fg-neg)]">
          Blocked at gate {data.blocked_at}.
        </p>
      )}

      <ul className="mt-2">
        {Array.from({ length: data.of }, (_, i) => i + 1).map((n) => (
          <VerdictRow key={n} n={n} verdict={byGate.get(n)} />
        ))}
      </ul>
    </section>
  )
}
