// The Monitor page's measurement half (COMP-007 §3, §6; Set S S-7.10).
//
// `Monitor()` in `Operations.tsx` is the controls: the switch and the
// per-strategy scale, halt and resume. This is what the monitor *knows* — the
// live event stream, what each feed's newest bar is, what each venue's
// reconciliation said, what is deployed but never judged, the platform defects
// somebody has to resolve, and when the fail-closed drills last passed.
//
// Everything here reads through `Read<T>` + `ReadGate`, so a route that did not
// answer is never drawn as the empty list a healthy platform returns. The
// stream is a read of the shared state `MonitorNotifier` writes; this component
// opens no connection of its own.
import { useState } from 'react'
import { skipToken, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Modal } from '@/components/primitives/Modal'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState } from '@/components/primitives/States'
import { Input } from '@/components/primitives/Field'
import { ReadGate } from '@/components/ops/ReadGate'
import { monitorOpsApi, refusalWords, type PlatformDefect } from '@/api/ops'
import { dateTime } from '@/lib/format'
import {
  MONITOR_LIVE_KEY,
  ageWords,
  eventSeverity,
  feedVerdict,
  kindWords,
  streamWords,
  timeframeWords,
  venueVerdict,
  type MonitorLiveState,
} from '@/lib/ops/monitor'

const OPS = ['monitor', 'ops'] as const

// ── The live stream ──────────────────────────────────────────────────────────

function LiveStreamPanel() {
  // Read-only: `MonitorNotifier` (in the shell) owns the connection. `skipToken`
  // makes this a subscription to the cache entry with no fetch of its own.
  const live = useQuery<MonitorLiveState>({ queryKey: MONITOR_LIVE_KEY, queryFn: skipToken })
  const s = live.data

  return (
    <Panel>
      <PanelHeader
        title="Live events"
        badges={
          s ? (
            <Badge
              tone={s.stream.state === 'live' ? 'pos' : s.stream.state === 'stale' ? 'warn' : 'neutral'}
              data-testid="monitor-stream-state"
            >
              {s.stream.state}
            </Badge>
          ) : (
            <Badge tone="warn" data-testid="monitor-stream-state">
              not subscribed
            </Badge>
          )
        }
      />
      <PanelBody>
        {!s ? (
          <EmptyState
            message="This page is not subscribed to /api/monitor/stream"
            detail="The shell's subscriber has not reported. Nothing below is a statement about whether the monitor has fired."
          />
        ) : (
          <>
            <p className="sub" data-testid="monitor-stream-words">
              /api/monitor/stream is{' '}
              {streamWords(s.stream.state, s.stream.everReceived, s.stream.staleForSecs, s.stream.error)}.
              {s.snapshot && (
                <>
                  {' '}
                  Lease {s.snapshot.lease} · orders{' '}
                  {s.snapshot.orders_permitted ? 'permitted' : 'not permitted'} ·{' '}
                  {s.snapshot.error
                    ? `switch unreadable (${s.snapshot.error})`
                    : s.snapshot.halted_scopes.length === 0
                      ? 'no scope halted'
                      : `halted: ${s.snapshot.halted_scopes.join(', ')}`}
                </>
              )}
            </p>
            {s.streamError && (
              <p className="sub" role="alert" data-testid="monitor-stream-error">
                The monitor's listener reported: {s.streamError}. Events may be missing until it recovers.
              </p>
            )}
            {s.events.length === 0 ? (
              <EmptyState
                message="No event has arrived on this connection"
                detail="Events appear here as they are written. This is not a history: /api/monitor/events holds that."
              />
            ) : (
              <table className="tbl" data-testid="monitor-live-events">
                <thead>
                  <tr>
                    <th style={{ textAlign: 'left' }}>when</th>
                    <th>scope</th>
                    <th>what</th>
                    <th>metric</th>
                    <th>value</th>
                    <th>limit</th>
                    <th>by</th>
                  </tr>
                </thead>
                <tbody>
                  {s.events.map((e) => {
                    const sev = eventSeverity(e.kind)
                    return (
                      <tr key={e.event_id}>
                        <td style={{ textAlign: 'left' }}>{dateTime(e.occurred_at)}</td>
                        <td>{e.scope}</td>
                        <td>
                          <Badge tone={sev === 'critical' ? 'neg' : sev === 'warning' ? 'warn' : 'neutral'}>
                            {kindWords(e.kind)}
                          </Badge>
                          {e.is_drill && <Badge tone="outline"> DRILL</Badge>}
                        </td>
                        <td>{e.metric ?? 'not recorded'}</td>
                        <td data-testid="event-value">{e.observed_value ?? 'not recorded'}</td>
                        <td data-testid="event-limit">{e.limit_value ?? 'not recorded'}</td>
                        <td>{e.actor}</td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            )}
          </>
        )}
      </PanelBody>
    </Panel>
  )
}

// ── Feeds ────────────────────────────────────────────────────────────────────

function FeedsPanel() {
  const q = useQuery({
    queryKey: [...OPS, 'feeds'],
    queryFn: () => monitorOpsApi.feeds(),
    refetchInterval: 15_000,
  })
  return (
    <Panel>
      <PanelHeader title="Feeds" />
      <PanelBody>
        <ReadGate query={q} route="/api/monitor/feeds" what="the feed ages">
          {(d) => (
            <>
              {d.unwatched_deployments.length > 0 && (
                <p className="sub" role="alert" data-testid="unwatched-deployments">
                  {d.unwatched_deployments.length} running deployment
                  {d.unwatched_deployments.length === 1 ? ' has' : 's have'} no feed gauge, so nothing
                  measures whether their data is stale: {d.unwatched_deployments.join(', ')}.
                </p>
              )}
              {d.feeds.length === 0 ? (
                <EmptyState
                  message="No feed is gauged"
                  detail="The monitor holds no feed rows. With deployments running, that is the monitor not measuring, not the data being fresh."
                />
              ) : (
                <table className="tbl" data-testid="feeds-table">
                  <thead>
                    <tr>
                      <th style={{ textAlign: 'left' }}>feed</th>
                      <th>venue</th>
                      <th>age of newest bar</th>
                      <th>verdict</th>
                      <th>read by</th>
                    </tr>
                  </thead>
                  <tbody>
                    {d.feeds.map((f) => {
                      const v = feedVerdict(f.verdict)
                      return (
                        <tr key={`${f.venue}/${f.symbol}/${f.timeframe_secs}`}>
                          <td style={{ textAlign: 'left' }}>
                            {f.symbol} · {timeframeWords(f.timeframe_secs)}
                          </td>
                          <td>{f.venue}</td>
                          <td title={`last bar ${dateTime(f.last_bar_at)}`}>{ageWords(f.age_secs)}</td>
                          <td>
                            <Badge tone={v.tone} title={v.meaning}>
                              {v.word}
                            </Badge>
                          </td>
                          <td>{f.strategies.length === 0 ? 'no strategy' : f.strategies.join(', ')}</td>
                        </tr>
                      )
                    })}
                  </tbody>
                </table>
              )}
            </>
          )}
        </ReadGate>
      </PanelBody>
    </Panel>
  )
}

// ── Venues ───────────────────────────────────────────────────────────────────

function VenuesPanel() {
  const q = useQuery({
    queryKey: [...OPS, 'venues'],
    queryFn: () => monitorOpsApi.venues(),
    refetchInterval: 30_000,
  })
  return (
    <Panel>
      <PanelHeader
        title="Venue reconciliation"
        badges={
          q.data?.state === 'ok' ? (
            <Badge tone={q.data.data.all_venues_reconciled ? 'pos' : 'warn'}>
              {q.data.data.all_venues_reconciled ? 'all reconciled' : 'not all reconciled'}
            </Badge>
          ) : undefined
        }
      />
      <PanelBody>
        <ReadGate query={q} route="/api/monitor/venues" what="the venue verdicts">
          {(d) =>
            d.venues.length === 0 ? (
              <EmptyState
                message="No venue has been reconciled"
                detail="Not the same as every venue agreeing: there is no check on record, and a resume is held to one."
              />
            ) : (
              <table className="tbl" data-testid="venues-table">
                <thead>
                  <tr>
                    <th style={{ textAlign: 'left' }}>venue</th>
                    <th>verdict</th>
                    <th>instruments</th>
                    <th>checked</th>
                    <th>detail</th>
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
                        <td>{v.instruments_checked}</td>
                        <td>{dateTime(v.checked_at)}</td>
                        <td>{v.detail ?? 'none recorded'}</td>
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

// ── What the monitor covers ──────────────────────────────────────────────────

function CoveragePanel() {
  const q = useQuery({
    queryKey: [...OPS, 'coverage'],
    queryFn: () => monitorOpsApi.coverage(),
    refetchInterval: 30_000,
  })
  return (
    <Panel>
      <PanelHeader title="What is judged" />
      <PanelBody>
        <ReadGate query={q} route="/api/monitor/coverage" what="the monitor's coverage">
          {(d) =>
            d.deployments.length === 0 ? (
              <EmptyState
                message="No deployment is running"
                detail="Nothing to cover. This is a fact about deployments, not about the monitor."
              />
            ) : (
              <table className="tbl" data-testid="coverage-table">
                <thead>
                  <tr>
                    <th style={{ textAlign: 'left' }}>strategy</th>
                    <th>instrument</th>
                    <th>judged</th>
                    <th>limit families declared</th>
                    <th>feed gauged</th>
                    <th>may leave paper</th>
                  </tr>
                </thead>
                <tbody>
                  {d.deployments.map((x) => (
                    <tr key={`${x.strategy_id}/${x.instrument_id}`}>
                      <td style={{ textAlign: 'left' }}>{x.strategy_id}</td>
                      <td>{x.instrument_id}</td>
                      <td>
                        {/* A deployment with nothing declared is measured and
                            never judged; the word says so. */}
                        <Badge tone={x.judged ? 'pos' : 'warn'}>{x.judged ? 'judged' : 'measured, never judged'}</Badge>
                      </td>
                      <td>{x.declared_families.length === 0 ? 'none' : x.declared_families.join(', ')}</td>
                      <td>
                        <Badge tone={x.feed_gauged ? 'pos' : 'warn'}>{x.feed_gauged ? 'yes' : 'no gauge'}</Badge>
                      </td>
                      <td title={x.promotion_gate.reason}>
                        {x.promotion_gate.allowed ? 'yes' : (x.promotion_gate.reason ?? 'no')}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )
          }
        </ReadGate>
      </PanelBody>
    </Panel>
  )
}

// ── Defects ──────────────────────────────────────────────────────────────────

function ResolveDefectDialog({
  defect,
  onClose,
}: {
  defect: PlatformDefect | null
  onClose: () => void
}) {
  const qc = useQueryClient()
  const [note, setNote] = useState('')
  const resolve = useMutation({
    mutationFn: async () => {
      if (!defect) throw new Error('no defect selected')
      return (await monitorOpsApi.resolveDefect(defect.defect_id, note.trim())).data
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: [...OPS, 'defects'] })
      onClose()
    },
  })
  return (
    <Modal
      open={defect !== null}
      onClose={onClose}
      title="Resolve a platform defect"
      description={defect ? `${defect.source} · filed by ${defect.filed_by}` : undefined}
      footer={
        <>
          <span className="spacer" />
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            disabled={note.trim().length === 0}
            loading={resolve.isPending}
            onClick={() => resolve.mutate()}
          >
            Resolve, as me
          </Button>
        </>
      }
    >
      <p className="sub">{defect?.summary}</p>
      <Input
        label="What you did about it"
        value={note}
        onChange={(e) => setNote(e.target.value)}
        placeholder="One sentence a later reader can act on"
        autoComplete="off"
      />
      {resolve.isError && (
        <p className="sub" role="alert" data-testid="resolve-defect-error">
          Refused: {refusalWords(resolve.error)}
        </p>
      )}
    </Modal>
  )
}

function DefectsPanel() {
  const [pending, setPending] = useState<PlatformDefect | null>(null)
  const q = useQuery({
    queryKey: [...OPS, 'defects'],
    queryFn: () => monitorOpsApi.defects(),
    refetchInterval: 30_000,
  })
  return (
    <Panel>
      <PanelHeader
        title="Platform defects"
        badges={
          q.data?.state === 'ok' ? (
            <Badge tone={q.data.data.defects.length > 0 ? 'warn' : 'pos'}>
              {q.data.data.defects.length} open
            </Badge>
          ) : undefined
        }
      />
      <PanelBody>
        <ReadGate query={q} route="/api/monitor/defects" what="the open defects">
          {(d) =>
            d.defects.length === 0 ? (
              <EmptyState
                message="No open defect"
                detail="The route answered with an empty list."
              />
            ) : (
              <table className="tbl" data-testid="defects-table">
                <thead>
                  <tr>
                    <th style={{ textAlign: 'left' }}>summary</th>
                    <th>source</th>
                    <th>filed</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {d.defects.map((x) => (
                    <tr key={x.defect_id}>
                      <td style={{ textAlign: 'left' }}>{x.summary}</td>
                      <td>{x.source}</td>
                      <td>
                        {dateTime(x.filed_at)} · {x.filed_by}
                      </td>
                      <td>
                        <Button size="xs" variant="ghost" onClick={() => setPending(x)}>
                          Resolve
                        </Button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )
          }
        </ReadGate>
        <p className="sub">
          Resolving takes a person and a sentence; an agent or service token is refused by the server.
        </p>
      </PanelBody>
      <ResolveDefectDialog key={pending?.defect_id ?? 'none'} defect={pending} onClose={() => setPending(null)} />
    </Panel>
  )
}

// ── Drills ───────────────────────────────────────────────────────────────────

function DrillsPanel() {
  const qc = useQueryClient()
  const q = useQuery({
    queryKey: [...OPS, 'drill-runs'],
    queryFn: () => monitorOpsApi.drillRuns(),
    refetchInterval: 60_000,
  })
  const run = useMutation({
    mutationFn: async () => (await monitorOpsApi.runDrills()).data,
    onSuccess: () => void qc.invalidateQueries({ queryKey: [...OPS, 'drill-runs'] }),
  })
  return (
    <Panel>
      <PanelHeader
        title="Fail-closed drills"
        badges={
          q.data?.state === 'ok' && q.data.data.suite_due ? <Badge tone="warn">suite due</Badge> : undefined
        }
        actions={
          <Button size="xs" variant="secondary" loading={run.isPending} onClick={() => run.mutate()}>
            Run the four now
          </Button>
        }
      />
      <PanelBody>
        <ReadGate query={q} route="/api/monitor/drill-runs" what="the drill history">
          {(d) => (
            <>
              <p className="sub">The suite is due every {d.interval_days} days.</p>
              {d.drills.length === 0 ? (
                <EmptyState
                  message="No drill has ever run"
                  detail="A fail-closed design nobody has rehearsed is a claim, not a property."
                />
              ) : (
                <table className="tbl" data-testid="drills-table">
                  <thead>
                    <tr>
                      <th style={{ textAlign: 'left' }}>drill</th>
                      <th>last run</th>
                      <th>result</th>
                    </tr>
                  </thead>
                  <tbody>
                    {d.drills.map((x) => (
                      <tr key={x.drill}>
                        <td style={{ textAlign: 'left' }}>{x.drill}</td>
                        <td>{x.last_run_at ? dateTime(x.last_run_at) : 'never'}</td>
                        <td>
                          <Badge tone={x.passed === null ? 'warn' : x.passed ? 'pos' : 'neg'}>
                            {x.passed === null ? 'not recorded' : x.passed ? 'passed' : 'FAILED'}
                          </Badge>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </>
          )}
        </ReadGate>
        {run.isError && (
          <p className="sub" role="alert" data-testid="drill-run-error">
            The drills did not run: {refusalWords(run.error)}
          </p>
        )}
        {run.data && (
          <p className="sub" data-testid="drill-run-result">
            {run.data.passed ? 'All four drills passed.' : 'At least one drill failed — see below.'}{' '}
            {run.data.drills
              .filter((x) => !x.passed)
              .map((x) => `${x.drill}: ${x.failure ?? 'no reason recorded'}`)
              .join('; ')}
          </p>
        )}
      </PanelBody>
    </Panel>
  )
}

export function MonitorLive() {
  return (
    <>
      <LiveStreamPanel />
      <FeedsPanel />
      <VenuesPanel />
      <CoveragePanel />
      <DefectsPanel />
      <DrillsPanel />
    </>
  )
}
