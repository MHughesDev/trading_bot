// INV-3 significance card: renders the p-value, its null (kind +
// preserves/destroys), AND the trial-count-at-eval as one inseparable unit — or
// an explicit "not yet significance-tested" empty state. There is deliberately
// no prop or branch that renders a bare p-value: the component takes the whole
// `SignificanceView | null`, and a missing null or trial count is unrepresentable
// because they travel together in that one object.
import { AlertTriangle, FlaskConical } from 'lucide-react'
import type { SignificanceView } from '@/api/experiments'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'

export function SignificanceCard({ significance }: { significance: SignificanceView | null }) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle className="flex items-center gap-2 text-sm">
          <FlaskConical className="h-4 w-4 text-accent" />
          Significance (INV-3)
        </CardTitle>
      </CardHeader>
      <CardContent>
        {significance ? (
          <SignificanceBody s={significance} />
        ) : (
          <div className="rounded-md border border-dashed border-border-2 p-4 text-sm text-text-dim">
            Not yet significance-tested. A p-value only exists once Gate&nbsp;3 runs the
            primary permutation test against the chosen null — and it is rendered with its
            null and trial count, never alone.
          </div>
        )}
      </CardContent>
    </Card>
  )
}

function SignificanceBody({ s }: { s: SignificanceView }) {
  const significant = s.p_value <= 0.05
  return (
    <div className="flex flex-col gap-4">
      {/* The inseparable triple: p ⊕ null ⊕ trial count. */}
      <div className="grid grid-cols-3 gap-3">
        <Stat label="corrected p-value" value={s.p_value.toFixed(4)} accent={significant} />
        <Stat label="vs null" value={s.null_kind} mono />
        <Stat label="trials at eval" value={s.trial_count_at_eval.toLocaleString()} />
      </div>
      <div className="text-xs text-text-dim">
        raw p = {s.raw_p_value.toFixed(4)} · selection-bias-corrected to{' '}
        {s.p_value.toFixed(4)} over {s.trial_count_at_eval.toLocaleString()} trials ·{' '}
        <span className="font-mono">{s.null_id}</span>
      </div>

      {/* The null's stated hypothesis — preserves/destroys (D-7). */}
      <div className="grid grid-cols-2 gap-3">
        <Hypothesis title="preserves" items={s.preserves} tone="muted" />
        <Hypothesis title="destroys" items={s.destroys} tone="muted" />
      </div>

      {/* Corroborators — never co-equal votes; disagreement is an investigate flag. */}
      <div className="flex items-center gap-2 border-t border-border pt-3">
        <Badge variant="outline">DSR {s.deflated_sharpe.toFixed(3)}</Badge>
        <Badge variant="outline">PBO {s.pbo.toFixed(3)}</Badge>
        {s.corroborators_agree ? (
          <span className="text-xs text-text-dim">corroborators agree</span>
        ) : (
          <Badge variant="warning" className="gap-1">
            <AlertTriangle className="h-3 w-3" />
            investigate — corroborators disagree
          </Badge>
        )}
      </div>
    </div>
  )
}

function Stat({
  label,
  value,
  accent,
  mono,
}: {
  label: string
  value: string
  accent?: boolean
  mono?: boolean
}) {
  return (
    <div className="rounded-md bg-surface-2 p-3">
      <div className="text-[10px] uppercase tracking-wide text-text-dim">{label}</div>
      <div
        className={`mt-1 text-lg font-semibold ${mono ? 'font-mono text-sm' : ''} ${
          accent ? 'text-pnl-up' : 'text-text'
        }`}
      >
        {value}
      </div>
    </div>
  )
}

function Hypothesis({
  title,
  items,
  tone,
}: {
  title: string
  items: string[]
  tone: 'muted'
}) {
  return (
    <div>
      <div className="text-[10px] uppercase tracking-wide text-text-dim">{title}</div>
      <ul className={`mt-1 space-y-0.5 text-xs ${tone === 'muted' ? 'text-text-muted' : ''}`}>
        {items.map((it) => (
          <li key={it}>· {it}</li>
        ))}
      </ul>
    </div>
  )
}
