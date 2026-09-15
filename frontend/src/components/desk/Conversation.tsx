// The conversation — the desk's main working screen (AGENT-012 §3).
//
// Header · card stream · controls row · composer, with the right panel beside
// it. Everything a person needs to work with a run is on this screen and nothing
// here requires reading a transcript.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { StaleBadge } from '@/components/primitives/States'
import type { DeskCard, WorkingState } from '@/api/desk'
import { deskApi } from '@/api/desk'
import { conversationsApi } from '@/api/agent'
import { mergeCards, toCards, type WireEvent } from '@/lib/desk/events'
import { useDeskStream } from '@/hooks/useDeskStream'
import { CardStream } from './CardStream'
import { Composer, type ComposerHandle, type ComposerSubmission } from './Composer'
import { ControlsRow, useDeskKeymap } from './ControlsRow'
import {
  AgendaTab,
  CharterTab,
  CostTab,
  EvidenceTab,
  MemoryTab,
  RightPanel,
  StateTab,
  ToolsTab,
  pendingApprovals,
  type PanelTab,
} from './RightPanel'
import { StageStrip, TierBadge, type IdResolver } from './Shared'

/**
 * The set of ids the platform can actually open.
 *
 * Built from what tools *returned* — the working state's evidence and findings —
 * never from what a card *mentions*. That distinction is the whole point: an id
 * the model wrote into a claim but no tool ever produced is a fabrication, and
 * the chip renders struck-through. Resolving a card's ids from the card itself
 * would make every id resolve and the check would assert nothing.
 */
export function makeResolver(state: WorkingState | null, cards: DeskCard[]): IdResolver {
  const known = new Map<string, { label: string; kind: string }>()
  for (const e of state?.evidence ?? []) known.set(e.id, { label: `from ${e.tool}, step ${e.step}`, kind: 'evidence' })
  for (const f of state?.findings ?? []) known.set(f.id, { label: f.headline, kind: 'finding' })
  for (const c of cards) {
    if (c.kind !== 'tool') continue
    const produced = (c.detail?.produced_ids ?? c.detail?.ids) as unknown
    if (Array.isArray(produced)) {
      for (const id of produced) {
        if (typeof id === 'string') known.set(id, { label: `returned by ${c.headline}`, kind: 'tool result' })
      }
    }
    const handle = c.detail?.handle
    if (typeof handle === 'string') known.set(handle, { label: `full result of ${c.headline}`, kind: 'handle' })
  }
  return (id) => known.get(id) ?? null
}

export function Conversation({
  conversationId,
  onOpenRun,
  onOpenDossier,
}: {
  conversationId: string
  onOpenRun: (runId: string) => void
  onOpenDossier: (ticket: string) => void
}) {
  const [cards, setCards] = useState<DeskCard[]>([])
  const [events, setEvents] = useState<WireEvent[]>([])
  const [tab, setTab] = useState<PanelTab>('state')
  const [detailsAll, setDetailsAll] = useState(false)
  const [paused, setPaused] = useState(false)
  const [sideOpen, setSideOpen] = useState(true)
  const [model, setModel] = useState('')
  const composer = useRef<ComposerHandle>(null)

  const convo = useQuery({
    queryKey: ['desk', 'conversation', conversationId],
    queryFn: () => conversationsApi.get(conversationId).then((r) => r.data),
    refetchInterval: 10_000,
  })
  const profiles = useQuery({
    queryKey: ['desk', 'profiles'],
    queryFn: () => conversationsApi.listProfiles().then((r) => r.data),
    staleTime: 60_000,
  })

  const latestTurn = convo.data?.turns?.[convo.data.turns.length - 1] ?? null
  const runId = latestTurn?.run_id ?? null
  const running = convo.data?.conversation.running === true

  // The live stream, with the polling fallback behind it. `useDeskStream`
  // reports `closed` on a 404, which is what today's backend answers for a
  // conversation run — so the desk degrades to the endpoint that does exist
  // rather than showing an empty screen.
  const onEvent = useCallback((e: WireEvent) => {
    setEvents((prev) => [...prev, e])
  }, [])
  const stream = useDeskStream({
    url: runId ? `/api/agent/sessions/${runId}/events` : null,
    onEvent,
  })

  const polled = useQuery({
    queryKey: ['desk', 'messages', runId],
    queryFn: () => deskApi.messages(runId as string).then((r) => r.data),
    enabled: !!runId && stream.state !== 'live',
    refetchInterval: running ? 1_500 : false,
  })

  useEffect(() => {
    const wire: WireEvent[] = [
      ...(polled.data?.messages ?? []),
      ...events,
    ]
    if (wire.length === 0) return
    setCards((prev) => mergeCards(prev, toCards(wire)))
  }, [polled.data, events])

  const state = useQuery({
    queryKey: ['desk', 'state', runId],
    queryFn: () => deskApi.state(runId as string),
    enabled: !!runId,
    refetchInterval: running ? 4_000 : false,
  })

  const workingState = state.data ?? null
  const resolve = useMemo(() => makeResolver(workingState, cards), [workingState, cards])
  const approvals = useMemo(() => pendingApprovals(cards), [cards])
  const continuous = false // A session's `continuous` flag arrives with S-11.13.

  const steer = useCallback(
    (text: string) => {
      if (!runId) return
      void deskApi.steer(runId, { kind: 'steer', text })
    },
    [runId],
  )

  const controls = {
    paused,
    busy: running,
    disconnected: stream.state === 'stale' || stream.state === 'closed',
    hasPendingApproval: approvals.length > 0,
    detailsOpen: detailsAll,
    onPause: () => {
      setPaused(true)
      if (runId) void deskApi.pause(runId)
    },
    onResume: () => {
      setPaused(false)
      if (runId) void deskApi.resume(runId)
    },
    onStop: () => void deskApi.cancel(conversationId),
    onSteer: () => composer.current?.focus(),
    onApprove: () => {
      const a = approvals[0]
      const id = (a?.detail?.approval_id as string) ?? null
      if (id) void deskApi.answerApproval(id, 'approve')
    },
    onDeny: () => {
      const a = approvals[0]
      const id = (a?.detail?.approval_id as string) ?? null
      if (id) void deskApi.answerApproval(id, 'deny')
    },
    onEscalate: () => {
      if (runId) void deskApi.escalate(runId, 'frontier')
    },
    onBudget: () => setTab('cost'),
    onToggleDetails: () => setDetailsAll((d) => !d),
    onReplay: () => runId && onOpenRun(runId),
    onFocusComposer: () => composer.current?.focus(),
    onShortcuts: () =>
      window.alert(
        'space pause/resume · esc stop · a approve · x deny · s steer · d details · r replay · / composer · ? this list',
      ),
  }
  useDeskKeymap(controls)

  function submit(s: ComposerSubmission) {
    if (s.steer) {
      steer(s.question)
      return
    }
    const preamble = [
      s.constraints.instrument && `instrument=${s.constraints.instrument}`,
      s.constraints.timeframe && `timeframe=${s.constraints.timeframe}`,
      s.constraints.window && `window=${s.constraints.window}`,
      s.constraints.balance && `balance=${s.constraints.balance}`,
      `mode=${s.mode}`,
      s.mode === 'research' && `workflow=${s.workflow}`,
      s.continuous && 'continuous=on',
    ]
      .filter(Boolean)
      .join(' ')
    if (s.continuous && s.charter) {
      void deskApi.createSession({ mode: s.mode, continuous: true, charter: s.charter, workflow: s.workflow })
      return
    }
    void conversationsApi.send(conversationId, `${preamble}\n${s.question}`)
  }

  const profileList = (profiles.data?.profiles ?? []).map((p) => ({
    id: p.model_id,
    label: p.model_id,
    tier: p.tier,
    measured: p.multi_step_evidence !== null || !p.is_local,
    frontier: p.tier === 'frontier',
  }))
  const chosen = profileList.find((p) => p.id === model) ?? profileList[0]

  return (
    <>
      <div className="desk-hd">
        <span className="title">{convo.data?.conversation.title ?? 'Conversation'}</span>
        {chosen && <TierBadge tier={chosen.tier} measured={chosen.measured} detail={chosen.label} />}
        {workingState && (
          <>
            <Badge tone="outline">{workingState.mode}</Badge>
            <Badge tone="outline">{workingState.workflow}</Badge>
            <Badge tone={workingState.ring === 'research' ? 'neutral' : 'warn'}>ring: {workingState.ring}</Badge>
            <StageStrip stage={workingState.stage} />
          </>
        )}
        <Badge tone={running ? 'accent' : 'neutral'}>
          {paused ? 'paused' : running ? 'working' : 'idle'}
        </Badge>
        {stream.state === 'stale' && <StaleBadge age={`${stream.staleForSecs ?? 0}s`} />}
        {workingState && (
          <span className="sub">
            trials {workingState.budget.trials_spent} · n_eff{' '}
            {workingState.budget.n_eff === null ? '(not computed)' : workingState.budget.n_eff.toFixed(1)}
          </span>
        )}
        <Button size="xs" variant="ghost" onClick={() => setSideOpen((o) => !o)}>
          {sideOpen ? 'Hide panel' : 'Show panel'}
        </Button>
      </div>

      <div className="desk-scroll">
        <CardStream
          conversationId={conversationId}
          cards={cards}
          resolve={resolve}
          detailsAll={detailsAll}
          actions={{
            onSteer: steer,
            onOpenDossier,
            onApprove: (id) => void deskApi.answerApproval(id, 'approve'),
            onDeny: (id) => void deskApi.answerApproval(id, 'deny'),
            onAnswer: (_id, text) => runId && void deskApi.steer(runId, { kind: 'answer', text }),
            onStartResearch: (q) => void conversationsApi.send(conversationId, `mode=research\n${q}`),
            onOpenId: () => setTab('evidence'),
          }}
        />
      </div>

      <div className="desk-dock">
        <ControlsRow {...controls} />
        <Composer
          ref={composer}
          running={running}
          profiles={profileList}
          model={chosen?.id ?? ''}
          onModelChange={setModel}
          onSubmit={submit}
          onSlash={(cmd, rest) => {
            if (cmd === '/steer') steer(rest)
            else if (cmd === '/pause') controls.onPause()
            else if (cmd === '/budget') setTab('cost')
            else if (cmd === '/why') steer('why did you do that?')
          }}
          onOffer={(q) => {
            // Until S-10.2 classifies server-side, the offer is rendered locally
            // as a card so the behaviour is real rather than promised.
            setCards((prev) =>
              mergeCards(prev, [
                {
                  seq: Date.now(),
                  step: 0,
                  kind: 'offer',
                  at: new Date().toISOString(),
                  headline: q,
                  detail: { workflow: 'investigate' },
                },
              ]),
            )
          }}
        />
      </div>

      <aside className="desk-side" hidden={!sideOpen}>
        <RightPanel tab={tab} onTab={setTab} continuous={continuous}>
          {tab === 'state' && (
            <StateTab
              state={workingState}
              resolve={resolve}
              paused={paused}
              pendingApproval={approvals.length > 0}
              onOpenId={() => setTab('evidence')}
            />
          )}
          {tab === 'evidence' && (
            <EvidenceTab
              resolve={resolve}
              entries={(workingState?.evidence ?? []).map((e) => ({
                id: e.id,
                type: e.id.split('_')[0] ?? 'id',
                summary: `produced by ${e.tool} at step ${e.step}`,
              }))}
            />
          )}
          {tab === 'memory' && <MemoryTab items={null} />}
          {tab === 'tools' && (
            <ToolsTab
              mode={workingState?.mode ?? 'chat'}
              exposed={null}
              catalogue={null}
              searchedFor={[]}
            />
          )}
          {tab === 'cost' && <CostTab calls={[]} bytesPerTokenCalibrated={null} bytesPerTokenEstimate={null} />}
          {tab === 'agenda' && <AgendaTab items={null} />}
          {tab === 'charter' && <CharterTab charter={null} amendments={[]} />}
        </RightPanel>
      </aside>
    </>
  )
}
