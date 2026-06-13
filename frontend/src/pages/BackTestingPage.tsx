import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Plus, FlaskConical, Loader2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { backtestsApi, type BacktestSnapshot } from '@/api/backtests'
import { isActive } from '@/components/backtest/status'
import { BacktestTile } from '@/components/backtest/BacktestTile'
import { CreateBacktestDialog } from '@/components/backtest/CreateBacktestDialog'
import { BacktestDetailsDialog } from '@/components/backtest/BacktestDetailsDialog'

export function BackTestingPage() {
  const [createOpen, setCreateOpen] = useState(false)
  const [selected, setSelected] = useState<BacktestSnapshot | null>(null)

  const { data, isLoading } = useQuery({
    queryKey: ['backtests'],
    queryFn: () => backtestsApi.list().then((r) => r.data.backtests),
    // Poll faster while any run is active so progress and phase stay live.
    refetchInterval: (q) =>
      (q.state.data ?? []).some((r) => isActive(r.status)) ? 1500 : 8000,
  })

  const runs = useMemo(() => data ?? [], [data])
  const runningCount = useMemo(
    () => runs.filter((r) => isActive(r.status)).length,
    [runs],
  )

  return (
    <div className="mx-auto w-full max-w-6xl px-6 py-6">
      {/* Header */}
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="flex items-center gap-2 text-2xl font-semibold text-text">
            <FlaskConical className="h-6 w-6 text-blue-400" />
            Back Testing
          </h1>
          <p className="mt-1 text-sm text-text-muted">
            Simulate strategies against historical data via the market
            simulator engine.
            {runningCount > 0 && (
              <span className="ml-1 text-blue-400">
                {runningCount} running.
              </span>
            )}
          </p>
        </div>
        <Button onClick={() => setCreateOpen(true)}>
          <Plus className="h-4 w-4" />
          New backtest
        </Button>
      </div>

      {/* Body */}
      {isLoading ? (
        <div className="flex items-center justify-center py-20 text-text-dim">
          <Loader2 className="h-6 w-6 animate-spin" />
        </div>
      ) : runs.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-3 rounded-lg border border-dashed border-border py-20 text-center">
          <FlaskConical className="h-10 w-10 text-text-dim" />
          <div className="text-text-muted">No backtests yet.</div>
          <Button variant="outline" onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4" />
            Run your first backtest
          </Button>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
          {runs.map((run) => (
            <BacktestTile key={run.id} run={run} onOpen={setSelected} />
          ))}
        </div>
      )}

      <CreateBacktestDialog open={createOpen} onOpenChange={setCreateOpen} />
      <BacktestDetailsDialog
        runId={selected?.id ?? null}
        fallback={selected}
        open={!!selected}
        onOpenChange={(o) => !o && setSelected(null)}
      />
    </div>
  )
}
