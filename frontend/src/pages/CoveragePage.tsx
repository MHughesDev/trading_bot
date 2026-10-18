// Data coverage: what the platform actually holds, thinnest series first
// (Set P P-0.2; DATA-005 §7; ROADMAP §1, §4 Stage 0 exit).
//
// This is the page to open before believing any claim about what can be
// evaluated. Each historical series is stated in words — how deep its longest
// continuous run is, and what that permits under `strict_v1` Gate 9 and
// `paper_v1` — and each forward-only series (funding, open interest, books:
// things that cannot be collected later) shows the age of its last row, because
// a forward series that stopped is losing history that does not come back.
//
// Desk-only on the server: a research project gets a 403 `desk_only`, and the
// page says that instead of rendering an empty census.
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { RotateCw } from 'lucide-react'
import { Badge } from '@/components/primitives/Badge'
import { IconButton } from '@/components/primitives/Button'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState } from '@/components/primitives/States'
import { ReadGate } from '@/components/ops/ReadGate'
import { dataCoverageApi } from '@/api/ops'
import { dateTime } from '@/lib/format'
import {
  STANDING_WORDS,
  depthFraction,
  depthStanding,
  depthWords,
  lastRowWords,
  stalestFirst,
  summarise,
  thinnestFirst,
} from '@/lib/ops/coverage'

const KEY = ['data', 'coverage'] as const

export function CoveragePage() {
  const qc = useQueryClient()
  const q = useQuery({
    queryKey: KEY,
    queryFn: () => dataCoverageApi.get(),
    refetchInterval: 5 * 60_000,
  })
  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Data coverage</h1>
        <div className="spacer" />
        <IconButton label="Refresh" onClick={() => void qc.invalidateQueries({ queryKey: KEY })}>
          <RotateCw size={14} aria-hidden />
        </IconButton>
      </div>
      <div className="page-body" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
        <ReadGate query={q} route="/api/data/coverage" what="the coverage census">
          {(d) => {
            const sum = summarise(d)
            const rows = thinnestFirst(d.series)
            return (
              <>
                <Panel>
                  <PanelHeader title="Census" />
                  <PanelBody>
                    {!d.census_taken ? (
                      <EmptyState
                        message="No census has been taken"
                        detail="The table is empty because nothing has measured the series yet, not because the platform holds nothing."
                      />
                    ) : (
                      <p className="sub" data-testid="census-summary">
                        {sum.total} series, as of {d.census_as_of ? dateTime(d.census_as_of) : 'an unrecorded time'}.{' '}
                        {sum.strict} meet the {d.floors.strict_v1_days}-day Gate 9 floor, {sum.paper} meet only the{' '}
                        {d.floors.paper_v1_days}-day paper floor, {sum.short} are below both and {sum.empty} hold
                        no rows. {sum.notAccruing} {sum.notAccruing === 1 ? 'is' : 'are'} not accruing: a stale
                        newest row means the shortfall does not close by waiting.
                      </p>
                    )}
                  </PanelBody>
                </Panel>

                {d.census_taken && (
                  <Panel>
                    <PanelHeader title="Historical series, thinnest first" />
                    <PanelBody flush>
                      <table className="tbl" data-testid="coverage-census">
                        <thead>
                          <tr>
                            <th style={{ textAlign: 'left' }}>series</th>
                            <th>depth</th>
                            <th>standing</th>
                            <th>missing</th>
                            <th>newest row</th>
                            <th style={{ textAlign: 'left' }}>what it permits</th>
                          </tr>
                        </thead>
                        <tbody>
                          {rows.map((s) => {
                            const standing = STANDING_WORDS[depthStanding(s)]
                            return (
                              <tr key={`${s.instrument}/${s.venue}/${s.timeframe}`}>
                                <td style={{ textAlign: 'left' }}>
                                  {s.instrument} · {s.timeframe}
                                  <div className="sub">
                                    {s.venue} · {s.series_kind}
                                  </div>
                                </td>
                                <td>
                                  <meter
                                    min={0}
                                    max={1}
                                    value={depthFraction(s, d.floors.strict_v1_days)}
                                    aria-label={`${depthWords(s)}, against the ${d.floors.strict_v1_days}-day floor`}
                                  />
                                  <div className="sub">{depthWords(s)}</div>
                                </td>
                                <td>
                                  <Badge tone={standing.tone}>{standing.word}</Badge>
                                </td>
                                <td>{(s.missing_bps / 100).toFixed(2)}%</td>
                                <td>
                                  {lastRowWords(s.last_row_age_secs)}
                                  {!s.accruing && (
                                    <div>
                                      <Badge tone="warn">not accruing</Badge>
                                    </div>
                                  )}
                                </td>
                                <td style={{ textAlign: 'left' }}>
                                  <div>{s.strict_v1.permits}</div>
                                  <div className="sub">{s.paper_v1.permits}</div>
                                </td>
                              </tr>
                            )
                          })}
                        </tbody>
                      </table>
                    </PanelBody>
                  </Panel>
                )}

                <Panel>
                  <PanelHeader title="Forward-only series" />
                  <PanelBody>
                    {!d.forward.measured ? (
                      <EmptyState
                        message="Forward series are not measured"
                        detail={`${d.forward.reason}. That is a missing measurement, not an empty list of series.`}
                      />
                    ) : d.forward.series.length === 0 ? (
                      <EmptyState
                        message="No forward series has written a heartbeat"
                        detail="Funding, open interest and book snapshots cannot be collected after the fact; none is being collected."
                      />
                    ) : (
                      <table className="tbl" data-testid="forward-series">
                        <thead>
                          <tr>
                            <th style={{ textAlign: 'left' }}>series</th>
                            <th>venue</th>
                            <th>last row</th>
                            <th>rows in the last beat</th>
                          </tr>
                        </thead>
                        <tbody>
                          {stalestFirst(d.forward.series).map((f) => (
                            <tr key={`${f.series}/${f.venue}/${f.instrument}`}>
                              <td style={{ textAlign: 'left' }}>
                                {f.series} · {f.instrument}
                              </td>
                              <td>{f.venue}</td>
                              <td title={dateTime(f.last_event_time)}>{lastRowWords(f.last_row_age_secs)}</td>
                              <td>{f.rows_written_last_beat ?? 'not recorded'}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    )}
                  </PanelBody>
                </Panel>
              </>
            )
          }}
        </ReadGate>
      </div>
    </>
  )
}
