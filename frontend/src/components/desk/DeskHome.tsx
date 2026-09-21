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
import { conversationsApi } from '@/api/agent'
import { deskApi, type Charter, type Mode, type Workflow } from '@/api/desk'
import { AwayDigestPanel, readLastVisit, writeLastVisit } from './AwayDigest'
import { Composer, type ComposerSubmission } from './Composer'
import { killSwitchBadge, monitorRow, type MonitorRow } from './Operations'
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
 * `live / paper` is **not** shown: `list_strategies` sends `strategy_id`,
 * `state` and `scale_pct` and nothing else, so the environment of a row is not
 * something this page knows. §2.4 names it; inventing it from a naming
 * convention is how a paper strategy comes to be read as live.
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
  const monitor = useQuery({ queryKey: ['desk', 'monitor'], queryFn: () => deskApi.monitor() })
  const kill = useQuery({ queryKey: ['desk', 'kill-switch'], queryFn: () => deskApi.killSwitch() })

  const running = useMemo(
    // `running` is on `Conversation`; the cast that used to be here was
    // defensive access over a shape the compiler already knows.
    () => (conversations.data?.conversations ?? []).filter((c) => c.running),
    [conversations.data],
  )
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

      <Panel>
        <PanelHeader title="Running now" />
        <PanelBody>
          {conversations.isPending ? (
            <PanelLoading lines={2} />
          ) : conversations.isError ? (
            <ErrorState
              message="The conversation list could not be read"
              detail="Whether anything is running is unknown, not none."
              onRetry={() => void conversations.refetch()}
            />
          ) : running.length === 0 ? (
            <EmptyState message="Nothing is running" detail="Start something below, or open a recent conversation." />
          ) : (
            // `Conversation` is a typed interface, so read its fields. The cast to
            // `Record<string, unknown>` that used to be here was defensive access
            // over a shape the compiler already knows — and TypeScript refused
            // it, which is why this file did not typecheck.
            running.map((c) => {
              const id = c.conversation_id
              return (
                <div key={id} className="desk-card">
                  <div className="desk-card-hd">
                    <Link to={`/desk/c/${id}`}>{c.title ?? id}</Link>
                    <Badge tone="accent">working</Badge>
                    {/* §2.2 asks for the tier badge on this row. Only shown
                        when the profile list names this model: a tier guessed
                        from a model string would be a capability claim, and
                        `unmeasured` on a profile we could not find would read
                        as a measurement that failed rather than a lookup that
                        did. */}
                    {(() => {
                      const p = profileList.find((x) => x.id === c.model)
                      return p ? (
                        <TierBadge tier={p.tier} measured={p.measured} detail={c.model} />
                      ) : (
                        <span className="sub">{c.model}</span>
                      )
                    })()}
                  </div>
                  <div className="desk-actions">
                    <Button size="xs" variant="secondary" onClick={() => void deskApi.cancel(id)}>
                      Stop
                    </Button>
                  </div>
                </div>
              )
            })
          )}
        </PanelBody>
      </Panel>

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
