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
import { statusPresentation, isActive, phaseLabel } from './status'

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

/** Maps asset class to the data source used for backfill/history. */
function dataSource(assetClass: string): string {
  switch (assetClass) {
    case 'crypto_spot_cex': return 'Kraken'
    case 'crypto_spot_dex':
    case 'perpetual_swap': return 'Binance'
    case 'equity':
    case 'etf': return 'Alpaca'
    default: return assetClass
  }
}

/** Format a simulated-time span (in seconds) as "Xd Yh Zm". */
function formatSimSpan(secs: number): string {
  const d = Math.floor(secs / 86400)
  const h = Math.floor((secs % 86400) / 3600)
  const m = Math.floor((secs % 3600) / 60)
  const parts: string[] = []
  if (d > 0) parts.push(`${d}d`)
  if (h > 0) parts.push(`${h}h`)
  if (m > 0 || parts.length === 0) parts.push(`${m}m`)
  return parts.join(' ')
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
            {run.strategy_slug} · {run.instrument_id} · {dataSource(run.asset_class)} ·{' '}
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
              {run.coverage.missing_ranges.length > 0 && (
                <div className="mt-2 rounded-md border border-amber-500/20 bg-amber-500/5 px-3 py-2">
                  <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-amber-400/70">
                    Missing ranges
                  </div>
                  <ul className="space-y-0.5">
                    {run.coverage.missing_ranges.map((r, i) => (
                      <li key={i} className="text-xs text-amber-300/80 tabular-nums">
                        {new Date(r.from).toLocaleDateString()} –{' '}
                        {new Date(r.to).toLocaleDateString()}
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </section>
          )}

          {/* Failure detail */}
          {run.status === 'failed' && run.error && (
            <section className="rounded-md border border-red-500/30 bg-red-500/10 p-3 text-sm text-red-300">
              <div className="font-semibold">
                Failed during {phaseLabel(run.failed_phase)}
              </div>
              <div className="mt-2 break-words leading-relaxed">{run.error}</div>
              {run.coverage && (
                <div className="mt-2 text-xs text-red-400/70">
                  {run.coverage.present_bars.toLocaleString()} of{' '}
                  {run.coverage.expected_bars.toLocaleString()} bars were present at failure.
                </div>
              )}
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
                    ['Orders filled', String(result.total_orders ?? 0)],
                    ['Positions', String(result.total_positions ?? 0)],
                    ['Events', String(result.total_events ?? 0)],
                    ['Bars processed', String(result.iterations ?? 0)],
                    [
                      'Sim span',
                      result.elapsed_time_secs
                        ? formatSimSpan(result.elapsed_time_secs)
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
