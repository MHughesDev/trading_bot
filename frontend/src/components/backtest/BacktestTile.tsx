import { useMutation, useQueryClient } from '@tanstack/react-query'
import {
  MoreVertical,
  Loader2,
  StopCircle,
  RotateCcw,
  Trash2,
  Eye,
  AlertTriangle,
} from 'lucide-react'
import { Card } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuLabel,
} from '@/components/ui/dropdown-menu'
import { backtestsApi, type BacktestSnapshot } from '@/api/backtests'
import { toast } from '@/hooks/useToast'
import { statusPresentation, isActive, resultHighlights } from './status'

function fmtDate(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  })
}

export function BacktestTile({
  run,
  onOpen,
}: {
  run: BacktestSnapshot
  onOpen: (run: BacktestSnapshot) => void
}) {
  const qc = useQueryClient()
  const present = statusPresentation(run.status)
  const active = isActive(run.status)
  const highlights = resultHighlights(run.result)

  const invalidate = () => qc.invalidateQueries({ queryKey: ['backtests'] })

  const stop = useMutation({
    mutationFn: () => backtestsApi.stop(run.id),
    onSuccess: () => {
      toast({ title: 'Stopping backtest…' })
      invalidate()
    },
    onError: () => toast({ title: 'Could not stop backtest', variant: 'error' }),
  })
  const rerun = useMutation({
    mutationFn: () => backtestsApi.rerun(run.id),
    onSuccess: () => {
      toast({ title: 'Re-running backtest', variant: 'success' })
      invalidate()
    },
    onError: () => toast({ title: 'Could not re-run', variant: 'error' }),
  })
  const remove = useMutation({
    mutationFn: () => backtestsApi.remove(run.id),
    onSuccess: () => {
      toast({ title: 'Backtest deleted' })
      invalidate()
    },
    onError: () => toast({ title: 'Could not delete', variant: 'error' }),
  })

  const pct = Math.round(run.progress)

  return (
    <Card className="flex flex-col gap-3 p-4">
      {/* Header: title + status + quick actions */}
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="truncate font-semibold text-text" title={run.name}>
            {run.name}
          </div>
          <div className="mt-0.5 truncate text-xs text-text-dim">
            {run.instrument_id} · {run.timeframe} · {run.asset_class}
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-1.5">
          <Badge variant={present.variant}>
            <span className="flex items-center gap-1">
              {present.busy && <Loader2 className="h-3 w-3 animate-spin" />}
              {present.label}
            </span>
          </Badge>

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon-sm" aria-label="Quick actions">
                <MoreVertical className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuLabel>Quick actions</DropdownMenuLabel>
              <DropdownMenuItem onSelect={() => onOpen(run)}>
                <Eye className="h-4 w-4" /> View details
              </DropdownMenuItem>
              {active && (
                <DropdownMenuItem
                  onSelect={() => stop.mutate()}
                  disabled={stop.isPending}
                >
                  <StopCircle className="h-4 w-4" /> Stop
                </DropdownMenuItem>
              )}
              <DropdownMenuItem
                onSelect={() => rerun.mutate()}
                disabled={rerun.isPending}
              >
                <RotateCcw className="h-4 w-4" /> Re-run
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                destructive
                disabled={active || remove.isPending}
                onSelect={() => remove.mutate()}
              >
                <Trash2 className="h-4 w-4" /> Delete
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      {/* Progress bar (in-flight) */}
      {active && (
        <div className="flex flex-col gap-1">
          <div className="flex justify-between text-xs text-text-dim">
            <span>{present.label}</span>
            <span>{pct}%</span>
          </div>
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-2">
            <div
              className="h-full rounded-full bg-blue-500 transition-all"
              style={{ width: `${pct}%` }}
            />
          </div>
          {run.status === 'collecting_data' && run.coverage && (
            <div className="text-xs text-amber-400">
              Backfilling {run.coverage.missing_ranges.length} gap
              {run.coverage.missing_ranges.length === 1 ? '' : 's'} —{' '}
              {run.coverage.collected_bars.toLocaleString()} bars collected
            </div>
          )}
        </div>
      )}

      {/* Failure surfacing */}
      {run.status === 'failed' && run.error && (
        <div className="flex items-start gap-2 rounded-md border border-red-500/30 bg-red-500/10 p-2 text-xs text-red-300">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <div>
            <div className="font-medium">
              Failed during {run.failed_phase ?? 'processing'}
            </div>
            <div className="mt-0.5 break-words opacity-90">{run.error}</div>
          </div>
        </div>
      )}

      {/* Result highlights (completed) */}
      {highlights.length > 0 && (
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
          {highlights.map((h) => (
            <div key={h.label} className="rounded-md bg-surface-2 px-2 py-1.5">
              <div className="text-[10px] uppercase tracking-wide text-text-dim">
                {h.label}
              </div>
              <div className="text-sm font-semibold text-text">{h.value}</div>
            </div>
          ))}
        </div>
      )}

      {/* Footer: window */}
      <div className="mt-auto flex items-center justify-between border-t border-border pt-2 text-xs text-text-dim">
        <span>
          {fmtDate(run.start)} → {fmtDate(run.end)}
        </span>
        <button
          className="text-blue-400 hover:underline"
          onClick={() => onOpen(run)}
        >
          Details
        </button>
      </div>
    </Card>
  )
}
