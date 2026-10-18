// Execution quality per strategy and instrument (FB-RX-003).
//
// What the order journal says about how orders actually executed: fills, the
// fraction of the ordered quantity that filled, and how long the first fill
// took — beside each venue's reconciliation verdict from the monitor.
//
// Slippage versus the fill model is shown for what it is: a statistic with no
// producer today, named once above the table with the server's own reason. It
// is never a column of zeros. Every cell that has no value says "not measured".
import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState } from '@/components/primitives/States'
import { Segmented } from '@/components/primitives/Segmented'
import { ReadGate } from '@/components/ops/ReadGate'
import { execQualityApi, monitorOpsApi } from '@/api/ops'
import { dateTime } from '@/lib/format'
import { venueVerdict } from '@/lib/ops/monitor'
import {
  fmtBps,
  fmtMs,
  fmtRatio,
  measuredText,
  reasonOf,
  slippageReason,
  totalFills,
  unproducedStatistics,
} from '@/lib/ops/execQuality'

type Window = '7' | '30' | '90'

function VenuePanel() {
  const q = useQuery({ queryKey: ['monitor', 'ops', 'venues'], queryFn: () => monitorOpsApi.venues() })
  return (
    <Panel>
      <PanelHeader title="Venue health" />
      <PanelBody>
        <ReadGate query={q} route="/api/monitor/venues" what="the venue verdicts">
          {(d) =>
            d.venues.length === 0 ? (
              <EmptyState message="No venue has been reconciled" detail="There is no check on record." />
            ) : (
              <table className="tbl" data-testid="exec-venues">
                <thead>
                  <tr>
                    <th style={{ textAlign: 'left' }}>venue</th>
                    <th>reconciliation</th>
                    <th>checked</th>
                  </tr>
                </thead>
                <tbody>
                  {d.venues.map((v) => {
                    const verdict = venueVerdict(v.verdict)
                    return (
                      <tr key={v.venue}>
                        <td style={{ textAlign: 'left' }}>{v.venue}</td>
                        <td>
                          <Badge tone={verdict.tone} title={verdict.meaning}>
                            {verdict.word}
                          </Badge>
                        </td>
                        <td>{dateTime(v.checked_at)}</td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            )
          }
        </ReadGate>
      </PanelBody>
    </Panel>
  )
}

export function ExecutionQualityPage() {
  const [days, setDays] = useState<Window>('30')
  const q = useQuery({
    queryKey: ['execution', 'quality', days],
    queryFn: () => execQualityApi.get(Number(days)),
    refetchInterval: 60_000,
  })
  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Execution quality</h1>
        <div className="spacer" />
        <Segmented
          ariaLabel="Window"
          value={days}
          onChange={setDays}
          options={[
            { value: '7', label: '7 days' },
            { value: '30', label: '30 days' },
            { value: '90', label: '90 days' },
          ]}
        />
      </div>
      <div className="page-body" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
        <Panel>
          <PanelHeader title="By strategy and instrument" />
          <PanelBody>
            <ReadGate query={q} route="/api/execution/quality" what="execution quality">
              {(d) => {
                if (d.rows.length === 0 || totalFills(d.rows) === 0) {
                  return (
                    <EmptyState
                      message="No fills recorded yet"
                      detail={`Nothing in the last ${d.window_days} days has filled, so there is no quality to report. That is not the same as good quality.`}
                    />
                  )
                }
                const missing = unproducedStatistics(d.rows)
                const why = slippageReason(d.rows)
                return (
                  <>
                    {missing.length > 0 && (
                      <p className="sub" data-testid="exec-unproduced">
                        Not measured for any row: {missing.join(', ')}. {why}.
                      </p>
                    )}
                    <table className="tbl" data-testid="exec-quality-table">
                      <thead>
                        <tr>
                          <th style={{ textAlign: 'left' }}>strategy</th>
                          <th>instrument</th>
                          <th>orders</th>
                          <th>fills</th>
                          <th>fill ratio</th>
                          <th>slippage p50 / p95</th>
                          <th>first fill p50 / p95</th>
                        </tr>
                      </thead>
                      <tbody>
                        {d.rows.map((r) => (
                          <tr key={`${r.strategy_id}/${r.instrument_id}`}>
                            <td style={{ textAlign: 'left' }}>{r.strategy_id}</td>
                            <td>{r.instrument_id}</td>
                            <td>{r.orders}</td>
                            <td>{r.fill_count}</td>
                            <td title={reasonOf(r.fill_ratio)}>{measuredText(r.fill_ratio, fmtRatio)}</td>
                            <td title={reasonOf(r.slippage_bps_p50)}>
                              {measuredText(r.slippage_bps_p50, fmtBps)} / {measuredText(r.slippage_bps_p95, fmtBps)}
                            </td>
                            <td title={reasonOf(r.submit_to_fill_ms_p50)}>
                              {measuredText(r.submit_to_fill_ms_p50, fmtMs)} /{' '}
                              {measuredText(r.submit_to_fill_ms_p95, fmtMs)}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </>
                )
              }}
            </ReadGate>
          </PanelBody>
        </Panel>
        <VenuePanel />
      </div>
    </>
  )
}
