// Agent run list — status-badged tiles, adaptive polling.

import { useQuery } from '@tanstack/react-query'
import { Bot, Loader2 } from 'lucide-react'
import { cn } from '@/lib/utils'
import { agentApi, AGENT_ACTIVE_STATUSES, type AgentRun } from '@/api/agent'

const STATUS_STYLES: Record<AgentRun['status'], string> = {
  queued: 'bg-surface text-text-muted border-border',
  running: 'bg-blue-500/10 text-blue-400 border-blue-500/20',
  waiting_backtest: 'bg-amber-500/10 text-amber-400 border-amber-500/20',
  completed: 'bg-green-500/10 text-green-400 border-green-500/20',
  failed: 'bg-red-500/10 text-red-400 border-red-500/20',
  cancelled: 'bg-surface text-text-dim border-border',
}

const STATUS_LABELS: Record<AgentRun['status'], string> = {
  queued: 'Queued',
  running: 'Running',
  waiting_backtest: 'Backtesting',
  completed: 'Completed',
  failed: 'Failed',
  cancelled: 'Cancelled',
}

export function RunList({
  selected,
  onSelect,
}: {
  selected: string | null
  onSelect: (runId: string) => void
}) {
  const { data } = useQuery({
    queryKey: ['agent-runs'],
    queryFn: () => agentApi.list({ limit: 25 }).then((r) => r.data),
    refetchInterval: (q) => {
      const anyActive = q.state.data?.runs.some((r) =>
        AGENT_ACTIVE_STATUSES.includes(r.status),
      )
      return anyActive ? 3000 : 10000
    },
  })

  if (!data) {
    return (
      <div className="flex items-center gap-2 text-sm text-text-muted p-4">
        <Loader2 className="h-4 w-4 animate-spin" /> Loading runs…
      </div>
    )
  }
  if (data.runs.length === 0) {
    return (
      <div className="flex flex-col items-center gap-2 text-center text-sm text-text-dim p-6">
        <Bot className="h-6 w-6" />
        No agent runs yet — configure a provider and start one.
      </div>
    )
  }

  return (
    <div className="space-y-2">
      {data.runs.map((run) => {
        const active = AGENT_ACTIVE_STATUSES.includes(run.status)
        return (
          <button
            key={run.run_id}
            onClick={() => onSelect(run.run_id)}
            className={cn(
              'w-full rounded-xl border p-3 text-left transition-colors',
              selected === run.run_id
                ? 'border-accent bg-surface'
                : 'border-border bg-surface-2 hover:bg-surface',
            )}
          >
            <div className="flex items-center justify-between gap-2">
              <span
                className={cn(
                  'inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-medium',
                  STATUS_STYLES[run.status],
                )}
              >
                {active && <Loader2 className="h-3 w-3 animate-spin" />}
                {STATUS_LABELS[run.status]}
              </span>
              <span className="text-[11px] text-text-dim">
                {new Date(run.created_at).toLocaleString()}
              </span>
            </div>
            <p className="mt-1.5 text-sm text-text line-clamp-2">{run.goal}</p>
            <div className="mt-1 flex items-center gap-3 text-[11px] text-text-muted">
              <span>{run.model}</span>
              <span>
                iter {run.iterations}/{run.max_iterations}
              </span>
              <span>{((run.tokens_in + run.tokens_out) / 1000).toFixed(1)}k tok</span>
            </div>
          </button>
        )
      })}
    </div>
  )
}
