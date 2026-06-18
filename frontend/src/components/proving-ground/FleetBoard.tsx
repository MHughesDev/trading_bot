// The fleet board — a live strip of every study/experiment currently advancing,
// so you can run many at once and watch them in one place. Driven by the existing
// /ws/backtest-suite progress frames (one row per experiment, latest frame wins).
//
// The suite serializes study execution server-side today, so usually one row is
// "running" at a time; once parallel execution lands (Phase B-exec) this board
// fills out with no UI change — it already renders N concurrent rows.
import { Activity } from 'lucide-react'
import type { ExperimentView } from '@/api/experiments'
import type { SuiteProgress } from '@/hooks/useSuiteProgress'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'

function isRunning(p: SuiteProgress): boolean {
  return p.phase.endsWith('_running') || (p.progress < 100 && !p.phase.endsWith('_failed'))
}

function barClass(p: SuiteProgress): string {
  if (p.phase.endsWith('_failed')) return 'bg-pnl-down'
  if (isRunning(p)) return 'bg-accent'
  return 'bg-pnl-up'
}

export function FleetBoard({
  experiments,
  progress,
}: {
  experiments: ExperimentView[]
  progress: Record<string, SuiteProgress>
}) {
  // Join each progress frame to its experiment (frames key on the experiment id).
  const rows = experiments
    .map((exp) => ({ exp, p: progress[exp.id] ?? progress[exp.experiment_id] }))
    .filter((r): r is { exp: ExperimentView; p: SuiteProgress } => !!r.p)
    .sort((a, b) => Number(isRunning(b.p)) - Number(isRunning(a.p)))

  const runningCount = rows.filter((r) => isRunning(r.p)).length

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between py-3">
        <CardTitle className="flex items-center gap-2 text-sm">
          <Activity className={`h-4 w-4 ${runningCount > 0 ? 'text-accent' : 'text-text-dim'}`} />
          Fleet
        </CardTitle>
        <span className="text-xs text-text-dim">
          {runningCount > 0 ? `${runningCount} running` : 'idle'}
        </span>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        {rows.length === 0 ? (
          <p className="text-xs text-text-dim">
            No studies running. Start one below — each appears here live while it executes.
          </p>
        ) : (
          rows.map(({ exp, p }) => (
            <div key={exp.id} className="flex flex-col gap-1">
              <div className="flex items-center justify-between gap-2 text-xs">
                <span className="flex min-w-0 items-center gap-2">
                  <span className="truncate font-medium text-text">{exp.experiment_id}</span>
                  <Badge variant="outline" className="font-mono text-[10px]">{p.phase}</Badge>
                </span>
                <span className="shrink-0 tabular-nums text-text-dim">
                  {Math.round(p.progress)}% · {exp.trial_counter.toLocaleString()} trials
                </span>
              </div>
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-2">
                <div
                  className={`h-full rounded-full transition-all ${barClass(p)}`}
                  style={{ width: `${Math.min(100, Math.max(0, p.progress))}%` }}
                />
              </div>
              {p.detail && <span className="truncate text-[11px] text-text-dim">{p.detail}</span>}
            </div>
          ))
        )}
      </CardContent>
    </Card>
  )
}
