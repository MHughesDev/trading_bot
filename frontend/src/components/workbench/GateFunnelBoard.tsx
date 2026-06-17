// Gate-funnel board (D-8). Renders Gates 0→4 in order; each gate is LOCKED
// (non-interactive, dimmed) until the prior gate has a passing verdict. Mirrors
// the staged funnel: integrity → single-path → robustness → significance → vault.
// The "advance" action drives Gates 0→3; the vault row links to the one-shot
// vault panel rather than acting here.
import { Lock, Check, X, Circle, Loader2 } from 'lucide-react'
import type { FunnelView, GateStatus, GateView } from '@/api/experiments'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'

const GATE_LABELS: Record<string, { n: number; title: string; blurb: string }> = {
  integrity: { n: 0, title: 'Integrity', blurb: 'close-stamp leak scan + cost floor' },
  single_path: { n: 1, title: 'Single-path', blurb: 'one honest walk-forward, median > 0' },
  robustness: { n: 2, title: 'Robustness', blurb: 'CPCV + synthetic + neighborhood shape' },
  significance: { n: 3, title: 'Significance', blurb: 'permutation p + DSR/PBO corroborators' },
  vault: { n: 4, title: 'Vault', blurb: 'one-shot holdout evaluation' },
}

export function GateFunnelBoard({
  funnel,
  onAdvance,
  advancing,
  canAdvance,
}: {
  funnel: FunnelView
  onAdvance: () => void
  advancing: boolean
  canAdvance: boolean
}) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle className="text-sm">Gate funnel</CardTitle>
        <Button size="sm" onClick={onAdvance} disabled={advancing || !canAdvance}>
          {advancing && <Loader2 className="h-3 w-3 animate-spin" />}
          Advance funnel
        </Button>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        {!canAdvance && (
          <p className="text-xs text-text-dim">
            Choose a significance null before running the funnel (INV-3).
          </p>
        )}
        {funnel.gates.map((g) => (
          <GateRow key={g.gate} gate={g} />
        ))}
      </CardContent>
    </Card>
  )
}

function GateRow({ gate }: { gate: GateView }) {
  const meta = GATE_LABELS[gate.gate]
  const locked = gate.status === 'locked'
  return (
    <div
      className={`flex items-start gap-3 rounded-md border p-3 ${
        locked ? 'border-border bg-surface-2/40 opacity-60' : 'border-border-2 bg-surface-2'
      }`}
      aria-disabled={locked}
    >
      <StatusIcon status={gate.status} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2 text-sm font-medium text-text">
          <span className="text-text-dim">Gate {meta.n}</span>
          {meta.title}
          <StatusBadge status={gate.status} />
        </div>
        <div className="text-xs text-text-dim">{gate.summary ?? meta.blurb}</div>
        {gate.evidence.length > 0 && (
          <div className="mt-1 truncate font-mono text-[10px] text-text-dim" title={gate.evidence.join(', ')}>
            evidence: {gate.evidence.join(', ')}
          </div>
        )}
      </div>
    </div>
  )
}

function StatusIcon({ status }: { status: GateStatus }) {
  const cls = 'h-4 w-4 mt-0.5 shrink-0'
  switch (status) {
    case 'passed':
      return <Check className={`${cls} text-pnl-up`} />
    case 'failed':
      return <X className={`${cls} text-pnl-down`} />
    case 'locked':
      return <Lock className={`${cls} text-text-dim`} />
    default:
      return <Circle className={`${cls} text-accent`} />
  }
}

function StatusBadge({ status }: { status: GateStatus }) {
  const map: Record<GateStatus, string> = {
    passed: 'text-pnl-up',
    failed: 'text-pnl-down',
    locked: 'text-text-dim',
    ready: 'text-accent',
  }
  return <span className={`text-[10px] uppercase tracking-wide ${map[status]}`}>{status}</span>
}
