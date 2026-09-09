// Live timeline for one agent run: assistant text, collapsible tool calls and
// results, backtest progress, final summary. Cursor-polls the transcript
// (after_seq) at 2s while the run is active — the BackTestingPage pattern.

import { useEffect, useRef, useState } from 'react'
import { Link } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  Bot,
  ChevronDown,
  ChevronRight,
  CircleStop,
  FlaskConical,
  Loader2,
  Terminal,
  Trophy,
} from 'lucide-react'
import { cn } from '@/lib/utils'
import {
  agentApi,
  AGENT_ACTIVE_STATUSES,
  type AgentMessage,
  type AgentRun,
} from '@/api/agent'

function Collapsible({
  header,
  children,
  tone = 'default',
}: {
  header: React.ReactNode
  children: React.ReactNode
  tone?: 'default' | 'error'
}) {
  const [open, setOpen] = useState(false)
  return (
    <div
      className={cn(
        'rounded-lg border',
        tone === 'error' ? 'border-red-500/20 bg-red-500/5' : 'border-border bg-surface-2',
      )}
    >
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-1.5 px-3 py-1.5 text-xs text-text-muted"
      >
        {open ? (
          <ChevronDown className="h-3 w-3 shrink-0" />
        ) : (
          <ChevronRight className="h-3 w-3 shrink-0" />
        )}
        {header}
      </button>
      {open && (
        <pre className="border-t border-border/50 px-3 py-2 text-[11px] text-text-muted overflow-x-auto max-h-80 overflow-y-auto whitespace-pre-wrap break-all">
          {children}
        </pre>
      )}
    </div>
  )
}

// The sealed product of a parameter sweep (FEAT-003 §7): a distribution, a
// surface description and the selection rule's single carry-forward — never a
// ranked list of samples.
interface SweepReport {
  study_ids: string[]
  trials_consumed: number
  n_sampled: number
  n_feasible: number
  n_failed_runs: number
  carried_forward: Record<string, unknown> | null
  neighbourhood_plateau: boolean | null
  distribution: { metric: string; median: number; worst_5pct: number; spread: number; iqr: [number, number] }
  surface: { text: string }
  violation_counts: Record<string, number>
  objective: { primary: string }
}

function SweepCard({ report, status }: { report: SweepReport; status: string }) {
  const d = report.distribution
  const violations = Object.entries(report.violation_counts ?? {})
  return (
    <div className="rounded-lg border border-sky-500/25 bg-sky-500/5 px-3 py-2.5 space-y-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
        <span className="flex items-center gap-1.5 font-semibold text-sky-400">
          <FlaskConical className="h-3.5 w-3.5" />
          Sweep {status || 'completed'}
        </span>
        <span className="text-text-muted">
          {report.trials_consumed} trials · {report.n_feasible}/{report.n_sampled} feasible
          {report.n_failed_runs > 0 ? ` · ${report.n_failed_runs} failed` : ''}
        </span>
        <span className="text-text-muted">
          {d.metric} median {d.median.toFixed(3)} · worst-5% {d.worst_5pct.toFixed(3)}
        </span>
        {report.neighbourhood_plateau != null && (
          <span
            className={cn(
              'rounded px-1.5 py-0.5 text-[10px] font-medium',
              report.neighbourhood_plateau
                ? 'bg-green-500/15 text-green-400'
                : 'bg-amber-500/15 text-amber-400',
            )}
          >
            {report.neighbourhood_plateau ? 'plateau' : 'spike — fragile'}
          </span>
        )}
      </div>
      {report.carried_forward && (
        <div className="text-xs">
          <span className="text-text-dim">carried forward (stable centroid): </span>
          <span className="font-mono text-text">
            {Object.entries(report.carried_forward)
              .map(([k, v]) => `${k}=${String(v)}`)
              .join('  ')}
          </span>
        </div>
      )}
      <pre className="whitespace-pre-wrap text-[11px] text-text-muted">{report.surface?.text}</pre>
      {violations.length > 0 && (
        <div className="text-[11px] text-text-dim">
          rejected: {violations.map(([k, n]) => `${k} ×${n}`).join('; ')}
        </div>
      )}
    </div>
  )
}

function MessageRow({ msg }: { msg: AgentMessage }) {
  const c = msg.content as Record<string, unknown>
  switch (msg.kind) {
    case 'assistant': {
      const text = (c.content as string) ?? ''
      const calls = (c.tool_calls as Array<{ name: string }>) ?? []
      if (!text && calls.length === 0) return null
      return (
        <div className="space-y-1.5">
          {text && (
            <div className="rounded-lg bg-surface-2 border border-border px-3 py-2 text-sm text-text whitespace-pre-wrap">
              {text}
            </div>
          )}
        </div>
      )
    }
    case 'tool_call':
      return (
        <Collapsible
          header={
            <span className="flex items-center gap-1.5 font-mono">
              <Terminal className="h-3 w-3" />
              {(c.name as string) ?? 'tool'}
            </span>
          }
        >
          {JSON.stringify(c.arguments ?? {}, null, 2)}
        </Collapsible>
      )
    case 'tool_result': {
      const isError = Boolean(c.is_error)
      const content = (c.content ?? {}) as Record<string, unknown>
      const sweepReport =
        c.name === 'run_sweep' && !isError && content.report
          ? (content.report as SweepReport)
          : null
      return (
        <div className="space-y-1.5">
          {sweepReport && <SweepCard report={sweepReport} status={String(content.status ?? '')} />}
          <Collapsible
            tone={isError ? 'error' : 'default'}
            header={
              <span className={cn('font-mono', isError && 'text-red-400')}>
                ↳ {(c.name as string) ?? 'result'} {isError ? '(error)' : ''}
              </span>
            }
          >
            {JSON.stringify(c.content ?? {}, null, 2)}
          </Collapsible>
        </div>
      )
    }
    case 'status': {
      if (c.phase === 'sweeping') {
        const done = Number(c.done ?? 0)
        const planned = Math.max(1, Number(c.planned ?? 1))
        const pct = Math.min(100, (done / planned) * 100)
        return (
          <div className="rounded-lg border border-sky-500/20 bg-sky-500/5 px-3 py-2">
            <div className="flex items-center gap-2 text-xs text-sky-400">
              <FlaskConical className="h-3.5 w-3.5" />
              Sweep {String(c.sweep_status ?? 'running')} — {done}/{planned} runs
              {c.note ? <span className="text-text-dim">· {String(c.note)}</span> : null}
            </div>
            <div className="mt-1.5 h-1.5 rounded-full bg-surface overflow-hidden">
              <div
                className="h-full rounded-full bg-sky-400 transition-all"
                style={{ width: `${Math.max(2, pct)}%` }}
              />
            </div>
          </div>
        )
      }
      if (c.phase === 'waiting_backtest') {
        const progress = Number(c.progress ?? 0)
        return (
          <div className="rounded-lg border border-amber-500/20 bg-amber-500/5 px-3 py-2">
            <div className="flex items-center gap-2 text-xs text-amber-400">
              <FlaskConical className="h-3.5 w-3.5" />
              Backtest {String(c.backtest_status ?? 'running')} — {progress.toFixed(0)}%
            </div>
            <div className="mt-1.5 h-1.5 rounded-full bg-surface overflow-hidden">
              <div
                className="h-full rounded-full bg-amber-400 transition-all"
                style={{ width: `${Math.min(100, Math.max(2, progress))}%` }}
              />
            </div>
          </div>
        )
      }
      return (
        <div className="text-[11px] text-text-dim px-1">
          {String(c.phase ?? 'status')} {c.model ? `· ${String(c.model)}` : ''}
        </div>
      )
    }
    case 'error':
      return (
        <div className="rounded-lg border border-red-500/20 bg-red-500/10 px-3 py-2 text-xs text-red-400">
          {(c.error as string) ?? 'error'}
        </div>
      )
    case 'final':
      return (
        <div className="rounded-lg border border-green-500/25 bg-green-500/5 px-3 py-2.5">
          <div className="flex items-center gap-1.5 text-xs font-semibold text-green-400 mb-1.5">
            <Trophy className="h-3.5 w-3.5" />
            Final result
          </div>
          <div className="text-sm text-text whitespace-pre-wrap">
            {(c.content as string) ?? ''}
          </div>
        </div>
      )
    default:
      return null
  }
}

export function RunTimeline({ runId }: { runId: string }) {
  const qc = useQueryClient()
  const [messages, setMessages] = useState<AgentMessage[]>([])
  const lastSeqRef = useRef(0)
  const bottomRef = useRef<HTMLDivElement>(null)

  // Reset the accumulated transcript when switching runs.
  useEffect(() => {
    setMessages([])
    lastSeqRef.current = 0
  }, [runId])

  const { data: run } = useQuery({
    queryKey: ['agent-run', runId],
    queryFn: () => agentApi.get(runId).then((r) => r.data),
    refetchInterval: (q) =>
      q.state.data && AGENT_ACTIVE_STATUSES.includes(q.state.data.status) ? 2000 : false,
  })
  const active = run ? AGENT_ACTIVE_STATUSES.includes(run.status) : true

  useQuery({
    queryKey: ['agent-run-messages', runId],
    queryFn: async () => {
      const res = await agentApi.messages(runId, {
        after_seq: lastSeqRef.current,
        limit: 200,
      })
      if (res.data.messages.length > 0) {
        lastSeqRef.current = res.data.last_seq
        setMessages((prev) => [...prev, ...res.data.messages])
      }
      return res.data
    },
    refetchInterval: active ? 2000 : false,
  })

  const cancelMutation = useMutation({
    mutationFn: () => agentApi.cancel(runId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['agent-run', runId] })
      qc.invalidateQueries({ queryKey: ['agent-runs'] })
    },
  })

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages.length])

  const budgetPct = run
    ? Math.min(100, (run.iterations / Math.max(1, run.max_iterations)) * 100)
    : 0

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <div className="flex items-center justify-between gap-3 border-b border-border pb-3 mb-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <Bot className="h-4 w-4 text-text-muted shrink-0" />
            <span className="text-sm font-medium text-text truncate">{run?.goal ?? '…'}</span>
          </div>
          {run && (
            <div className="mt-1 flex items-center gap-3 text-[11px] text-text-muted">
              <span>
                {run.provider} · {run.model}
              </span>
              <span>
                iterations {run.iterations}/{run.max_iterations}
              </span>
              <span>
                {((run.tokens_in + run.tokens_out) / 1000).toFixed(1)}k tokens
              </span>
            </div>
          )}
          {run && (
            <div className="mt-1.5 h-1 w-48 rounded-full bg-surface-2 overflow-hidden">
              <div
                className="h-full rounded-full bg-accent transition-all"
                style={{ width: `${budgetPct}%` }}
              />
            </div>
          )}
        </div>
        {active && (
          <button
            disabled={cancelMutation.isPending}
            onClick={() => cancelMutation.mutate()}
            className="flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm text-red-400 hover:bg-red-400/10 border border-red-400/30 transition-colors disabled:opacity-40 shrink-0"
          >
            {cancelMutation.isPending ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <CircleStop className="h-3.5 w-3.5" />
            )}
            Cancel
          </button>
        )}
      </div>

      {/* Timeline */}
      <div className="flex-1 overflow-y-auto space-y-2 pr-1">
        {messages.length === 0 && (
          <div className="flex items-center gap-2 text-sm text-text-muted p-2">
            <Loader2 className="h-4 w-4 animate-spin" />
            Waiting for the agent…
          </div>
        )}
        {messages.map((m) => (
          <MessageRow key={m.seq} msg={m} />
        ))}

        {run?.status === 'failed' && run.error && (
          <div className="rounded-lg border border-red-500/20 bg-red-500/10 px-3 py-2 text-xs text-red-400">
            Run failed: {run.error}
          </div>
        )}
        {run?.status === 'completed' && (
          <div className="flex gap-2 pt-1">
            {run.final_strategy_id && (
              <Link
                to="/strategy"
                className="rounded-lg border border-border bg-surface-2 px-3 py-1.5 text-xs text-text hover:bg-surface transition-colors"
              >
                Strategy: {run.final_strategy_id}
              </Link>
            )}
            {run.best_backtest_id && (
              <Link
                to="/backtesting"
                className="rounded-lg border border-border bg-surface-2 px-3 py-1.5 text-xs text-text hover:bg-surface transition-colors"
              >
                View backtest
              </Link>
            )}
          </div>
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  )
}
