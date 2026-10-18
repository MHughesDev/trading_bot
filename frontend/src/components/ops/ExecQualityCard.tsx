// Execution quality for one instrument, as a card under the terminal chart
// (FB-RX-003, acceptance 2).
//
// The same read as the Execution quality page, narrowed to the instrument on
// screen. Empty state is the acceptance's own sentence: "No fills recorded yet".
// A statistic with no value says "not measured"; it is never a zero.
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { EmptyState } from '@/components/primitives/States'
import { ReadGate } from '@/components/ops/ReadGate'
import { execQualityApi } from '@/api/ops'
import { fmtBps, fmtMs, fmtRatio, measuredText, reasonOf, totalFills } from '@/lib/ops/execQuality'

export function ExecQualityCard({ instrument }: { instrument: string }) {
  const q = useQuery({
    queryKey: ['execution', 'quality', '30'],
    queryFn: () => execQualityApi.get(30),
    refetchInterval: 60_000,
  })
  return (
    <div style={{ padding: 'var(--s-3) var(--s-4)', overflow: 'auto' }} data-testid="exec-quality-card">
      <ReadGate query={q} route="/api/execution/quality" what="execution quality">
        {(d) => {
          const rows = d.rows.filter((r) => r.instrument_id === instrument)
          if (rows.length === 0 || totalFills(rows) === 0) {
            return (
              <EmptyState
                message="No fills recorded yet"
                detail={`${instrument} has no fill in the last ${d.window_days} days, so there is no quality to report.`}
              />
            )
          }
          return (
            <>
              <table className="tbl dense">
                <thead>
                  <tr>
                    <th style={{ textAlign: 'left' }}>strategy</th>
                    <th>fills</th>
                    <th>fill ratio</th>
                    <th>slippage p50 / p95</th>
                    <th>first fill p50 / p95</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((r) => (
                    <tr key={r.strategy_id}>
                      <td style={{ textAlign: 'left' }}>{r.strategy_id}</td>
                      <td>{r.fill_count}</td>
                      <td title={reasonOf(r.fill_ratio)}>{measuredText(r.fill_ratio, fmtRatio)}</td>
                      <td title={reasonOf(r.slippage_bps_p50)}>
                        {measuredText(r.slippage_bps_p50, fmtBps)} / {measuredText(r.slippage_bps_p95, fmtBps)}
                      </td>
                      <td title={reasonOf(r.submit_to_fill_ms_p50)}>
                        {measuredText(r.submit_to_fill_ms_p50, fmtMs)} / {measuredText(r.submit_to_fill_ms_p95, fmtMs)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <p className="sub">
                Last {d.window_days} days. <Link to="/execution-quality">All instruments and venue health</Link>
              </p>
            </>
          )
        }}
      </ReadGate>
    </div>
  )
}
