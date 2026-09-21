// Desk home (AGENT-012 §2 as amended; task S-6.14).
//
// The operator hour: what is waiting on you, what is running, what changed while
// you were away, what has been concluded, what the book looks like, which tiers
// are measured — and one place to start.
//
// Meridian §3.22 applies to every panel here: an empty "Needs you" says what the
// last decision was, never a bare "No data".
import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link, useNavigate } from 'react-router-dom'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { ConfirmDialog } from '@/components/primitives/Modal'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import { conversationsApi, type Conversation } from '@/api/agent'
import {
  deskApi,
  type Charter,
  type DeskRunningRun,
  type Mode,
  type NotRecordedNote,
  type Workflow,
} from '@/api/desk'
import { AwayDigestPanel, readLastVisit, writeLastVisit } from './AwayDigest'
import { Composer, type ComposerSubmission } from './Composer'
import { environmentBadge, killSwitchBadge, monitorRow, type MonitorRow } from './Operations'
import { TierBadge, ago, tierTiles } from './Shared'

/**
 * The last decision anybody made on this desk.
 *
 * `/api/approvals?state=all` returns every approval newest-first with its
 * `answer` and `answered_at`, so the answered ones are the decision log the
 * empty state needs. Meridian §3.22 asks an empty "Needs you" to say what was
 * last decided rather than "No data", and the panel used to pass a hardcoded
 * `null` and then print "No decision has been recorded on this desk yet" — an
 * assertion about the desk's whole history made without looking at anything.
 *
 * Returns `null` only when the rows genuinely contain no answered approval.
 */
export function lastDecision(rows: Array<Record<string, unknown>>): string | null {
  const answered = rows
    .filter((r) => typeof r.answer === 'string' && (r.answer as string).length > 0)
    .sort((a, b) => String(b.answered_at ?? '').localeCompare(String(a.answered_at ?? '')))
  const top = answered[0]
  if (!top) return null
  const who = typeof top.answered_by === 'string' && top.answered_by ? ` by ${top.answered_by}` : ''
  return `${String(top.kind ?? 'an approval')} → ${String(top.answer)}${who}`
}

export function LastDecision({
  text,
  known,
}: {
  text: string | null
  /** False while the decision log is loading or failed to load. */
  known: boolean
}) {
  return (
    <EmptyState
      message="Nothing is waiting on you"
      detail={
        !known
          ? 'The decision log could not be read, so the desk cannot say what was last decided.'
          : text
            ? `Last decision: ${text}`
            : 'No approval on this desk has been answered yet.'
      }
    />
  )
}

/**
 * A count the run recorded, or the words for one it did not.
 *
 * Migration 0062 made every telemetry column nullable so a run that crashed
 * before filling them is still a run, and `null` there is *not recorded*. A
 * `?? 0` would say the run used no context, spent no trials and took no step —
 * three measurements nobody made.
 */
export function runCount(value: number | null, name: string): string {
  return value === null || value === undefined
    ? `${name} not recorded`
    : `${value.toLocaleString()} ${name}`
}

/**
 * "Running now" (AGENT-012 §2.2), from `GET /api/desk/running`.
 *
 * The panel used to read `running` off the conversation list, which answers one
 * of §2.2's questions — is any turn active in this conversation — and none of
 * the others. Every other column §2.2 asks for is a fact about a **run**, and a
 * conversation with three finished turns and one working one has one tier and
 * one token count that matter. So the rows are runs, joined here to the
 * conversation list by `conversation_id` for the link and the stop control.
 *
 * Two things this panel deliberately does not do:
 *
 * * **No cache-hit percentage.** Both token counts are shown and the ratio is
 *   the reader's to take. `prompt_tokens` is nullable, and a percentage computed
 *   over a missing denominator is an invented number rather than a measured one.
 * * **No hiding of `not_recorded`.** That array is the route naming the columns
 *   §2.2 asks for that nothing in this deployment records — per-run dollars, and
 *   the working-state step. Dropping it turns a stated absence into a silent
 *   one, which is how a blank cost column comes to be read as free.
 */
export function RunningNow({
  runs,
  notRecorded,
  loading,
  conversations,
  onRetry,
}: {
  /** `null` when `/api/desk/running` did not answer — unknown, not none. */
  runs: DeskRunningRun[] | null
  notRecorded: NotRecordedNote[]
  loading: boolean
  /** The conversation list this joins to; `null` when it did not answer either. */
  conversations: Conversation[] | null
  onRetry?: () => void
}) {
  const byId = new Map((conversations ?? []).map((c) => [c.conversation_id, c]))
  const alsoWorking = (conversations ?? []).filter((c) => c.running).length
  return (
    <Panel>
      <PanelHeader title="Running now" />
      <PanelBody>
        {loading ? (
          <PanelLoading lines={2} />
        ) : runs === null ? (
          <ErrorState
            message="The run list could not be read"
            detail={
              conversations === null
                ? '/api/desk/running is served, so this is the request failing rather than nothing running. What is working is unknown, not none.'
                : `/api/desk/running is served, so this is the request failing rather than nothing running. The conversation list says ${alsoWorking} conversation${alsoWorking === 1 ? ' is' : 's are'} working, which is all that is still known here.`
            }
            onRetry={onRetry}
          />
        ) : runs.length === 0 ? (
          <EmptyState message="Nothing is running" detail="Start something below, or open a recent conversation." />
        ) : (
          <div data-testid="running-runs">
            {runs.map((r) => {
              const convo = r.conversation_id ? byId.get(r.conversation_id) : undefined
              const title = r.title ?? convo?.title ?? r.run_id
              const step = r.last_transcript_entry
              return (
                <div key={r.run_id} className="desk-card" data-testid={`run-${r.run_id}`}>
                  <div className="desk-card-hd">
                    {r.conversation_id ? (
                      <Link to={`/desk/c/${r.conversation_id}`}>{title}</Link>
                    ) : (
                      // A run with no conversation cannot be opened, and a dead
                      // link that looks like a link is worse than plain text.
                      <span>{title}</span>
                    )}
                    <Badge tone="accent">{r.status}</Badge>
                    <Badge tone="outline">{r.mode}</Badge>
                    {/* A Chat turn is doing none of the seven workflows, and
                        `null` here is that — not a workflow nobody named. */}
                    <Badge tone="neutral">{r.workflow ?? 'no workflow — a chat turn'}</Badge>
                    {/* The tier the run was dispatched at, as recorded. Never
                        read off the model string, which would be a capability
                        claim rather than a record. */}
                    <Badge tone={r.tier === null ? 'warn' : 'info'}>
                      {r.tier === null ? 'tier not recorded' : r.tier}
                    </Badge>
                  </div>
                  <p className="sub">
                    stage {r.stage} ·{' '}
                    {step
                      ? `last transcript entry: ${step.kind ?? 'kind not recorded'}${
                          step.seq === null ? '' : ` #${step.seq}`
                        }${step.at ? `, ${ago(step.at)} ago` : ''}`
                      : 'no transcript entry yet'}
                  </p>
                  <p className="sub">
                    {r.model} · {r.profile_name ?? 'profile not recorded'} ·{' '}
                    {runCount(r.iterations, 'iterations')}
                  </p>
                  {/* Both counts, never their ratio: `prompt_tokens` is
                      nullable, and a hit rate over a missing denominator is a
                      number this panel would have invented. */}
                  <p className="sub" data-testid={`tokens-${r.run_id}`}>
                    {runCount(r.prompt_tokens, 'prompt tokens')} ·{' '}
                    {runCount(r.cache_read_tokens, 'cache read tokens')}
                  </p>
                  <p className="sub">
                    {/* 0 trials is a run that searched nothing; absent is a run
                        that did not record what it searched. */}
                    {runCount(r.trials_spent, 'trials spent')} ·{' '}
                    {r.outcome ?? 'no outcome yet'} ·{' '}
                    {r.started_at ? `started ${ago(r.started_at)} ago` : 'not started yet'}
                  </p>
                  {r.conversation_id && (
                    <div className="desk-actions">
                      <Button
                        size="xs"
                        variant="secondary"
                        onClick={() => void deskApi.cancel(r.conversation_id as string)}
                      >
                        Stop
                      </Button>
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        )}
        {runs !== null && notRecorded.length > 0 && (
          <div data-testid="running-not-recorded">
            <span className="lbl">not recorded</span>
            {notRecorded.map((n) => (
              <p className="sub" key={n.field}>
                <b>{n.field}</b>: {n.why}
              </p>
            ))}
          </div>
        )}
      </PanelBody>
    </Panel>
  )
}

/**
 * The book (AGENT-012 §2.4) — a compact mirror of `/desk/monitor`.
 *
 * §2.4 asks for three things: the control-limit status **per strategy**, the
 * kill-switch state, and one **Halt all** behind Meridian's irreversible-action
 * pattern. The panel had the second and shipped the other two as the sentence
 * "N strategies under control limits", which is a count, not a status — a
 * strategy the monitor had scaled to 10% and one running at full size were the
 * same character on this page.
 *
 * The worse half was the empty state: `/api/monitor/strategies` has been served
 * since COMP-007 landed, and a read that did not answer rendered "The monitor is
 * not built yet — COMP-007 lands with Set S phase 7". A false explanation is
 * worse than none, because it tells the operator there is nothing to look at.
 *
 * `live / paper` is shown now that `list_strategies` sends it — read from
 * `orders.execution_mode`, which is a recorded fact per order and not the
 * strategy's name read as an environment. The third state is the one that
 * matters on this panel: a strategy that has placed **no order** is served as
 * `null`, which is *unknown*, and rendering it as paper would be this page
 * answering a question nobody asked the database.
 */
export function TheBook({
  rows,
  loading,
  kill,
  killLoading,
}: {
  /** `null` when `/api/monitor/strategies` did not answer. */
  rows: MonitorRow[] | null
  loading: boolean
  kill: Parameters<typeof killSwitchBadge>[0]
  killLoading: boolean
}) {
  const [halting, setHalting] = useState(false)
  const [haltError, setHaltError] = useState(false)
  const badge = killSwitchBadge(kill)
  return (
    <Panel>
      <PanelHeader
        title="The book"
        badges={
          killLoading ? (
            <Badge tone="neutral">reading the kill switch…</Badge>
          ) : (
            <Badge tone={badge.tone} data-testid="book-kill-badge">
              kill switch: {badge.word}
            </Badge>
          )
        }
        actions={<Link to="/desk/monitor">Open the monitor</Link>}
      />
      <PanelBody>
        {loading ? (
          <PanelLoading lines={2} />
        ) : rows === null ? (
          <ErrorState
            message="The monitor did not answer"
            detail="/api/monitor/strategies is served (COMP-007), so this is the request failing rather than the phase being unbuilt. What is live is unknown, not none."
          />
        ) : rows.length === 0 ? (
          <EmptyState
            message="No strategy is under the monitor"
            detail="The endpoint answered with an empty list, which is a fact about the monitor rather than about this panel."
          />
        ) : (
          <ul style={{ listStyle: 'none', margin: 0, padding: 0 }} data-testid="book-rows">
            {rows.map((r) => (
              <li key={r.strategy_id} className="desk-card-hd" style={{ fontSize: 'var(--t-12)' }}>
                <span className="mono">{r.strategy_id}</span>
                {/* `state` is the control-limit status. Absent is its own word:
                    a row the monitor sent without one has not been cleared. */}
                <Badge tone={r.state === 'nominal' ? 'pos' : r.state === null ? 'warn' : 'neg'}>
                  {r.state ?? 'state not reported'}
                </Badge>
                {/* 0% is the monitor having shut it down; absent is nobody
                    having said. Never the same character. */}
                <span className="sub">
                  {r.scale_pct === null ? 'scale not reported' : `${r.scale_pct}% of full size`}
                </span>
                {/* Three readings, not two: live, paper, and a strategy that
                    has never traded — which is unknown and must not read as
                    the safe one. */}
                {(() => {
                  const env = environmentBadge(r.environment)
                  return (
                    <Badge tone={env.tone} data-testid={`book-env-${r.strategy_id}`}>
                      {env.word}
                    </Badge>
                  )
                })()}
              </li>
            ))}
          </ul>
        )}
        <div className="desk-actions">
          <Button size="xs" variant="danger" onClick={() => setHalting(true)}>
            Halt all
          </Button>
          <span className="sub">
            A reset needs a reason and the typed scope; it is on the monitor page.
          </span>
        </div>
        <ConfirmDialog
          open={halting}
          onCancel={() => setHalting(false)}
          title="Halt all trading?"
          consequence="Every strategy stops placing orders immediately, on every venue. Open positions are held, not closed. Only a reset — rate-limited, reasoned and typed — undoes it."
          typeToConfirm="HALT"
          confirmLabel="Halt all"
          onConfirm={() => {
            setHalting(false)
            // Not a bare `void`: a session without `permits_monitor_control`
            // gets a 403, and a halt that failed silently on this panel is an
            // operator who believes trading has stopped.
            deskApi
              .haltAll('halted from the desk home book panel')
              .catch(() => setHaltError(true))
          }}
        />
        {haltError && (
          <p className="sub" role="alert" data-testid="book-halt-error">
            The halt did not reach the monitor. Nothing was stopped — open the monitor and try there.
          </p>
        )}
      </PanelBody>
    </Panel>
  )
}

export function DeskHome() {
  const navigate = useNavigate()
  const [lastVisit] = useState(() => readLastVisit())
  const [model, setModel] = useState('')

  const conversations = useQuery({
    queryKey: ['desk', 'conversations'],
    queryFn: () => conversationsApi.list().then((r) => r.data),
    refetchInterval: 10_000,
  })
  const approvals = useQuery({
    queryKey: ['desk', 'approvals', 'pending'],
    queryFn: () => deskApi.approvals({ state: 'pending' }).then((r) => r.data),
    refetchInterval: 10_000,
  })
  const profiles = useQuery({
    queryKey: ['desk', 'profiles'],
    queryFn: () => conversationsApi.listProfiles().then((r) => r.data),
    staleTime: 60_000,
  })
  // The decision log behind the empty state. Separate from the pending query
  // because they answer different questions: one is what is waiting, the other
  // is what was already settled.
  const decisions = useQuery({
    queryKey: ['desk', 'approvals', 'all'],
    queryFn: () => deskApi.approvals({ state: 'all' }).then((r) => r.data),
    staleTime: 30_000,
  })
  const digest = useQuery({
    queryKey: ['desk', 'digest', lastVisit],
    queryFn: () => deskApi.awayDigest(new Date(lastVisit).toISOString()),
  })
  // §2.2's own route, per run. The conversation list answers "is a turn active
  // here" and none of the other columns.
  const runningNow = useQuery({
    queryKey: ['desk', 'running'],
    queryFn: () => deskApi.running(),
    refetchInterval: 10_000,
  })
  const monitor = useQuery({ queryKey: ['desk', 'monitor'], queryFn: () => deskApi.monitor() })
  const kill = useQuery({ queryKey: ['desk', 'kill-switch'], queryFn: () => deskApi.killSwitch() })

  const recent = useMemo(
    () => (conversations.data?.conversations ?? []).slice(0, 8),
    [conversations.data],
  )
  const pending = approvals.data?.approvals ?? []

  const profileList = tierTiles(profiles.data?.profiles)

  async function start(s: ComposerSubmission) {
    if (s.continuous && s.charter) {
      const created = await deskApi.createSession({
        mode: s.mode as Mode,
        continuous: true,
        charter: s.charter as Charter,
        workflow: s.workflow as Workflow,
        model: model || profileList[0]?.id,
      })
      if (created) navigate(`/desk/c/${created.conversation_id}`)
      return
    }
    const { data } = await conversationsApi.create()
    await conversationsApi.send(
      data.conversation_id,
      `mode=${s.mode}${s.mode === 'research' ? ` workflow=${s.workflow}` : ''}\n${s.question}`,
    )
    navigate(`/desk/c/${data.conversation_id}`)
  }

  return (
    <div className="desk-scroll" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
      <Panel>
        <PanelHeader title="Needs you" badges={<Badge tone={pending.length ? 'warn' : 'neutral'}>{pending.length}</Badge>} />
        <PanelBody>
          {/* Four states, because "nothing is waiting on you" is a claim and the
              other three are not. A failed inbox that renders as an empty one
              tells the operator to walk away from a run that is blocked on
              them — the one failure this panel exists to prevent. */}
          {approvals.isPending ? (
            <PanelLoading lines={2} />
          ) : approvals.isError ? (
            <ErrorState
              message="The approvals inbox could not be read"
              detail="This is not the same as nothing waiting on you. A run blocked on an approval stays blocked."
              onRetry={() => void approvals.refetch()}
            />
          ) : pending.length === 0 ? (
            <LastDecision
              text={lastDecision(decisions.data?.approvals ?? [])}
              known={!decisions.isPending && !decisions.isError}
            />
          ) : (
            <ul style={{ listStyle: 'none', margin: 0, padding: 0 }}>
              {pending.map((a) => {
                const row = a as Record<string, unknown>
                const id = String(row.approval_id ?? '')
                return (
                  <li key={id} className="desk-card">
                    <div className="desk-card-hd">
                      <Badge tone="outline">{String(row.kind ?? 'approval')}</Badge>
                      <span className="sub">waiting {ago(String(row.created_at ?? ''))}</span>
                    </div>
                    <p>{String((row.payload as Record<string, unknown>)?.summary ?? row.kind ?? '')}</p>
                    <div className="desk-actions">
                      <Button size="xs" onClick={() => void deskApi.answerApproval(id, 'approve')}>
                        Approve
                      </Button>
                      <Button size="xs" variant="secondary" onClick={() => void deskApi.answerApproval(id, 'deny')}>
                        Deny
                      </Button>
                    </div>
                  </li>
                )
              })}
            </ul>
          )}
        </PanelBody>
      </Panel>

      <AwayDigestPanel
        data={digest.data ?? null}
        lastVisit={lastVisit}
        loading={digest.isPending}
        onDismiss={() => writeLastVisit()}
      />

      <RunningNow
        runs={runningNow.data?.runs ?? null}
        notRecorded={runningNow.data?.not_recorded ?? []}
        loading={runningNow.isPending}
        conversations={conversations.data?.conversations ?? null}
        onRetry={() => void runningNow.refetch()}
      />

      <Panel>
        <PanelHeader title="Recent conclusions" />
        <PanelBody>
          {conversations.isPending ? (
            <PanelLoading lines={3} />
          ) : conversations.isError ? (
            <ErrorState
              message="The conversation list could not be read"
              detail="Nothing below is a claim that you have concluded nothing."
              onRetry={() => void conversations.refetch()}
            />
          ) : recent.length === 0 ? (
            <EmptyState message="No conversations yet" detail="The first thing you ask will appear here." />
          ) : (
            <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }}>
              {recent.map((c) => {
                const id = c.conversation_id
                return (
                  <li key={id} style={{ fontSize: 'var(--t-12)' }}>
                    <Link to={`/desk/c/${id}`}>{c.title ?? id}</Link>{' '}
                    <span className="sub">{ago(c.last_activity_at)} ago</span>
                  </li>
                )
              })}
            </ul>
          )}
        </PanelBody>
      </Panel>

      <TheBook
        rows={monitor.data ? monitor.data.strategies.map(monitorRow) : null}
        loading={monitor.isPending}
        kill={kill.data}
        killLoading={kill.isPending}
      />


      <Panel>
        <PanelHeader title="Tier status" />
        <PanelBody>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 'var(--s-3)' }}>
            {profileList.map((p) => (
              <div key={p.id} className="well" style={{ minWidth: 200 }}>
                <div className="desk-card-hd">
                  <strong style={{ fontSize: 'var(--t-12)' }}>{p.label}</strong>
                  <TierBadge tier={p.tier} measured={p.measured} />
                </div>
                <p className="sub">{p.measured ? 'calibrated' : 'no calibration row — treated as unmeasured'}</p>
              </div>
            ))}
            {/* Three states. "No profiles" is an alarming claim — nothing can
                run without one — and it was made on a request that may never
                have come back. */}
            {profileList.length === 0 &&
              (profiles.isPending ? (
                <PanelLoading lines={2} />
              ) : profiles.data === undefined ? (
                <ErrorState
                  message="The profile list could not be read"
                  detail="/api/agent/profiles is served, so this is the request failing. Which tiers are measured is unknown, not none."
                  onRetry={() => void profiles.refetch()}
                />
              ) : (
                <EmptyState message="No profiles" detail="No capability profile is configured." />
              ))}
          </div>
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader title="Start" />
        <PanelBody>
          <Composer
            running={false}
            profiles={profileList}
            model={model || profileList[0]?.id || ''}
            onModelChange={setModel}
            onSubmit={(s) => void start(s)}
            onOffer={(q) => void start({
              mode: 'research',
              continuous: false,
              workflow: 'investigate',
              question: q,
              charter: null,
              constraints: { instrument: '', timeframe: '', window: '', balance: '' },
              steer: false,
            })}
          />
          <div className="desk-actions">
            <span className="sub">Starters:</span>
            <Button size="xs" variant="ghost" onClick={() => void start({
              mode: 'chat', continuous: false, workflow: 'investigate',
              question: 'What instruments do we have clean 1m bars for?',
              charter: null, constraints: { instrument: '', timeframe: '', window: '', balance: '' }, steer: false,
            })}>
              Ask a question
            </Button>
            <Button size="xs" variant="ghost" onClick={() => void start({
              mode: 'research', continuous: false, workflow: 'falsify',
              question: 'Does the overnight gap on BTC carry an edge after costs?',
              charter: null, constraints: { instrument: '', timeframe: '', window: '', balance: '' }, steer: false,
            })}>
              Research an edge
            </Button>
            <Button size="xs" variant="ghost" onClick={() => navigate('/desk/memory')}>
              Browse what is already known
            </Button>
          </div>
        </PanelBody>
      </Panel>
    </div>
  )
}
