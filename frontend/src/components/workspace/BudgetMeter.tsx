// The budget pane (COMP-006 §3, UI-07).
//
// Dollars, tokens, cache hit rate and dollars per verdict. The last one is the only
// figure on this panel that says anything about value rather than consumption: a
// session that spent $40 and produced a verdict is cheaper than four that spent $12
// each and produced none.
//
// Cache hit rate is here for a specific reason. D-03 says model capability is never
// traded for tokens — the only honest way to spend less is to waste less — so the
// number a reader needs is the one that separates the work from the overhead.

import { useQuery } from '@tanstack/react-query'
import { researchApi } from '@/api/research'

interface Props {
  projectId?: string
  sessionId?: string
  /** How often to refresh, in ms. */
  intervalMs?: number
}

function money(n: number): string {
  return n >= 100 ? `$${n.toFixed(0)}` : `$${n.toFixed(2)}`
}

function compact(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return String(n)
}

function Row({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="flex items-baseline justify-between gap-2 py-1">
      <span className="text-[11px] text-text-dim" title={hint}>
        {label}
      </span>
      <span className="font-mono text-xs text-text">{value}</span>
    </div>
  )
}

export function BudgetMeter({ projectId, sessionId, intervalMs = 10_000 }: Props) {
  const { data: usage, isError } = useQuery({
    queryKey: ['research', 'usage', projectId ?? null, sessionId ?? null],
    queryFn: async () =>
      (await researchApi.usage({ project_id: projectId, session_id: sessionId }))
        .data,
    refetchInterval: intervalMs,
  })

  if (!usage) {
    return (
      <div className="rounded-lg border border-border bg-surface p-3 text-xs text-text-dim">
        Budget: loading…
      </div>
    )
  }

  return (
    <div className="rounded-lg border border-border bg-surface p-3">
      <div className="mb-1 flex items-center justify-between">
        <h3 className="text-[11px] font-semibold uppercase tracking-wide text-text-dim">
          Budget
        </h3>
        {isError && (
          <span className="text-[10px] text-warning" title="the last refresh failed">
            stale
          </span>
        )}
      </div>
      <Row label="Spent" value={money(usage.cost_usd)} />
      <Row
        label="Per verdict"
        value={
          usage.dollars_per_verdict === null
            ? 'no verdict yet'
            : money(usage.dollars_per_verdict)
        }
        hint="Dollars divided by the number of sessions that filed a report. A session that spends more and answers is cheaper than several that spend less and do not."
      />
      <div className="my-1.5 h-px bg-border" />
      <Row label="Prompt tokens" value={compact(usage.input_tokens)} />
      <Row label="Output tokens" value={compact(usage.output_tokens)} />
      <Row
        label="Cache hit"
        value={`${(usage.cache_hit_rate * 100).toFixed(0)}%`}
        hint="Share of prompt tokens served from cache rather than re-billed at full price."
      />
      <Row label="Verdicts" value={String(usage.verdicts)} />
    </div>
  )
}
