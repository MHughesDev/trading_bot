// INV-2 distribution viewer. Shows median / IQR / worst-5% / spread and the
// empirical histogram for a sealed Study. There is deliberately NO "best member",
// "sort by metric", or "open peak run" control anywhere in this component —
// `member_run_ids` are listed in insertion order for provenance only, and the
// carry-forward config (if any) is labelled with its pre-declared SelectionRule.
// The worst-5% line is rendered at least as prominently as the median.
import type { StudyView } from '@/api/experiments'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'

const SELECTION_LABELS: Record<string, string> = {
  none: 'none',
  median_stable_centroid: 'median-stable centroid',
  worst_case_robust: 'worst-case robust',
}

export function DistributionViewer({ study }: { study: StudyView }) {
  const d = study.distribution
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center justify-between gap-2 text-sm">
          <span className="flex items-center gap-2">
            {study.study_id}
            <Badge variant="inactive">{study.kind}</Badge>
            <Badge variant="outline">{study.metric}</Badge>
          </span>
          <span className="text-xs font-normal text-text-dim">
            +{study.trial_delta} trials · sealed
          </span>
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <p className="text-xs italic text-text-dim">“{study.question}”</p>

        {/* worst-5% is first and styled at least as prominently as the median. */}
        <div className="grid grid-cols-4 gap-3">
          <DistStat label="worst-5%" value={d.worst_5pct} emphatic />
          <DistStat label="median" value={d.median} emphatic />
          <DistStat label="IQR" value={`[${fmt(d.iqr[0])}, ${fmt(d.iqr[1])}]`} />
          <DistStat label="spread" value={d.spread} />
        </div>

        <Histogram values={d.dist} median={d.median} worst5={d.worst_5pct} />

        <div className="flex flex-wrap items-center gap-2 border-t border-border pt-3 text-xs text-text-dim">
          <span>
            verdict:{' '}
            <Badge variant={study.verdict.positive_median ? 'active' : 'inactive'}>
              {study.verdict.positive_median ? 'positive median' : 'non-positive median'}
            </Badge>
          </span>
          {study.verdict.plateau !== null && (
            <Badge variant={study.verdict.plateau ? 'active' : 'warning'}>
              {study.verdict.plateau ? 'plateau' : 'spike'}
            </Badge>
          )}
        </div>

        {/* Carry-forward is the ONLY config promotion, and it is the pre-declared
            selection rule's output — never a metric-ranked pick. */}
        <div className="text-xs text-text-dim">
          carry-forward rule:{' '}
          <span className="font-mono text-text-muted">
            {SELECTION_LABELS[study.selection_rule] ?? study.selection_rule}
          </span>
          {study.carried_forward
            ? ' · one config carried forward by this rule'
            : ' · nothing carried forward'}
        </div>

        {/* Provenance only: insertion order, no ranking, no open-peak control. */}
        <details className="text-xs text-text-dim">
          <summary className="cursor-pointer select-none">
            {study.members.length} member run ids (provenance, insertion order)
          </summary>
          <ol className="mt-2 max-h-32 list-decimal space-y-0.5 overflow-y-auto pl-5 font-mono">
            {study.members.map((m, i) => (
              <li key={`${m}-${i}`} className="truncate" title={m}>
                {m}
              </li>
            ))}
          </ol>
        </details>
      </CardContent>
    </Card>
  )
}

function DistStat({
  label,
  value,
  emphatic,
}: {
  label: string
  value: number | string
  emphatic?: boolean
}) {
  return (
    <div className={`rounded-md p-3 ${emphatic ? 'bg-surface-2' : ''}`}>
      <div className="text-[10px] uppercase tracking-wide text-text-dim">{label}</div>
      <div className={`mt-1 font-semibold ${emphatic ? 'text-lg text-text' : 'text-sm text-text-muted'}`}>
        {typeof value === 'number' ? fmt(value) : value}
      </div>
    </div>
  )
}

// A simple equal-width histogram. It surfaces the shape of the distribution; it
// exposes no interaction to pick or open an individual member.
function Histogram({ values, median, worst5 }: { values: number[]; median: number; worst5: number }) {
  if (values.length === 0) {
    return <div className="text-xs text-text-dim">no surviving members</div>
  }
  const bins = 16
  const min = Math.min(...values)
  const max = Math.max(...values)
  const range = max - min || 1
  const counts = new Array(bins).fill(0)
  for (const v of values) {
    const idx = Math.min(bins - 1, Math.floor(((v - min) / range) * bins))
    counts[idx] += 1
  }
  const peak = Math.max(...counts) || 1
  const pos = (v: number) => `${(((v - min) / range) * 100).toFixed(1)}%`
  return (
    <div className="relative h-28">
      <div className="flex h-24 items-end gap-0.5">
        {counts.map((c, i) => (
          <div
            key={i}
            className="flex-1 rounded-t bg-accent/40"
            style={{ height: `${(c / peak) * 100}%` }}
          />
        ))}
      </div>
      {/* worst-5% marker — visually weighted equal to the median. */}
      <Marker left={pos(worst5)} color="bg-pnl-down" label="worst-5%" />
      <Marker left={pos(median)} color="bg-text-muted" label="median" />
    </div>
  )
}

function Marker({ left, color, label }: { left: string; color: string; label: string }) {
  return (
    <div className="absolute top-0 h-24" style={{ left }}>
      <div className={`h-24 w-px ${color}`} />
      <div className="mt-0.5 -translate-x-1/2 text-[9px] text-text-dim">{label}</div>
    </div>
  )
}

function fmt(v: number): string {
  return v.toFixed(4)
}
