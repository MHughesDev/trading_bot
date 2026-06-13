import { useQuery } from '@tanstack/react-query'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@/components/ui/dialog'
import { Badge } from '@/components/ui/badge'
import { backtestsApi, type BacktestSnapshot } from '@/api/backtests'
import { statusPresentation, isActive } from './status'

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md bg-surface-2 px-3 py-2">
      <div className="text-[10px] uppercase tracking-wide text-text-dim">
        {label}
      </div>
      <div className="text-sm font-semibold text-text">{value}</div>
    </div>
  )
}

function StatGrid({ entries }: { entries: Array<[string, string]> }) {
  if (entries.length === 0)
    return <div className="text-sm text-text-dim">No data.</div>
  return (
    <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
      {entries.map(([k, v]) => (
        <Stat key={k} label={k} value={v} />
      ))}
    </div>
  )
}

export function BacktestDetailsDialog({
  runId,
  fallback,
  open,
  onOpenChange,
}: {
  runId: string | null
  fallback: BacktestSnapshot | null
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  // Poll the single run while the dialog is open and the run is active.
  const { data } = useQuery({
    queryKey: ['backtest', runId],
    queryFn: () => backtestsApi.get(runId as string).then((r) => r.data),
    enabled: open && !!runId,
    initialData: fallback ?? undefined,
    refetchInterval: (q) =>
      q.state.data && isActive(q.state.data.status) ? 1500 : false,
  })

  const run = data ?? fallback
  if (!run) return null
  const present = statusPresentation(run.status)
  const result = run.result

  const pnlEntries: Array<[string, string]> = result?.stats_pnls
    ? Object.entries(result.stats_pnls).flatMap(([ccy, stats]) =>
        Object.entries(stats).map(
          ([k, v]) => [`${k} (${ccy})`, Number(v).toFixed(4)] as [string, string],
        ),
      )
    : []
  const returnEntries: Array<[string, string]> = result?.stats_returns
    ? Object.entries(result.stats_returns).map(
        ([k, v]) => [k, Number(v).toFixed(4)] as [string, string],
      )
    : []
  const generalEntries: Array<[string, string]> = result?.stats_general
    ? Object.entries(result.stats_general).map(
        ([k, v]) => [k, Number(v).toFixed(4)] as [string, string],
      )
    : []

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {run.name}
            <Badge variant={present.variant}>{present.label}</Badge>
          </DialogTitle>
          <DialogDescription>
            {run.strategy_slug} · {run.instrument_id} · {run.venue_id} ·{' '}
            {run.timeframe}
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-5">
          {/* Configuration */}
          <section>
            <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-text-dim">
              Configuration
            </h4>
            <StatGrid
              entries={[
                ['Asset class', run.asset_class],
                ['Window start', new Date(run.start).toLocaleString()],
                ['Window end', new Date(run.end).toLocaleString()],
                ['Initial balance', `${run.initial_balance} ${run.quote_currency}`],
                ['Auto-collect', run.auto_collect ? 'On' : 'Off'],
              ]}
            />
          </section>

          {/* Data coverage */}
          {run.coverage && (
            <section>
              <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-text-dim">
                Data coverage
              </h4>
              <StatGrid
                entries={[
                  ['Expected bars', run.coverage.expected_bars.toLocaleString()],
                  ['Present bars', run.coverage.present_bars.toLocaleString()],
                  ['Collected bars', run.coverage.collected_bars.toLocaleString()],
                  ['Gaps found', String(run.coverage.missing_ranges.length)],
                ]}
              />
            </section>
          )}

          {/* Failure detail */}
          {run.status === 'failed' && run.error && (
            <section className="rounded-md border border-red-500/30 bg-red-500/10 p-3 text-sm text-red-300">
              <div className="font-medium">
                Failed during {run.failed_phase ?? 'processing'}
              </div>
              <div className="mt-1 break-words">{run.error}</div>
            </section>
          )}

          {/* Results */}
          {result && (
            <>
              <section>
                <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-text-dim">
                  Run summary
                </h4>
                <StatGrid
                  entries={[
                    ['Orders', String(result.total_orders ?? 0)],
                    ['Positions', String(result.total_positions ?? 0)],
                    ['Events', String(result.total_events ?? 0)],
                    ['Iterations', String(result.iterations ?? 0)],
                    [
                      'Elapsed',
                      result.elapsed_time_secs
                        ? `${result.elapsed_time_secs.toFixed(2)}s`
                        : '—',
                    ],
                  ]}
                />
              </section>

              {pnlEntries.length > 0 && (
                <section>
                  <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-text-dim">
                    PnL statistics
                  </h4>
                  <StatGrid entries={pnlEntries} />
                </section>
              )}

              {returnEntries.length > 0 && (
                <section>
                  <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-text-dim">
                    Return statistics
                  </h4>
                  <StatGrid entries={returnEntries} />
                </section>
              )}

              {generalEntries.length > 0 && (
                <section>
                  <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-text-dim">
                    General statistics
                  </h4>
                  <StatGrid entries={generalEntries} />
                </section>
              )}
            </>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}
