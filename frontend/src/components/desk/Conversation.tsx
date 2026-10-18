// The conversation — the desk's main working screen (AGENT-012 §3).
//
// Header · card stream · controls row · composer, with the right panel beside
// it. Everything a person needs to work with a run is on this screen and nothing
// here requires reading a transcript.
import { useCallback, useMemo, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { invalidateApprovals } from '@/lib/desk/approvals'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { StaleBadge } from '@/components/primitives/States'
import type { DeskCard, SessionDigestRef, Stage } from '@/api/desk'
import {
  charterView,
  deskApi,
  sessionAmendments,
  sessionDigestTarget,
  sessionForConversation,
} from '@/api/desk'
import { conversationsApi } from '@/api/agent'
import { costCalls, latestExposure, mergeCards, toCards, toolSearches, type WireEvent } from '@/lib/desk/events'
import { useDeskStream } from '@/hooks/useDeskStream'
import { useDeskAnnouncer } from '@/hooks/useDeskAnnouncer'
import { CardStream } from './CardStream'
import { Composer, type ComposerHandle, type ComposerSubmission } from './Composer'
import { ControlsRow, type PauseState } from './ControlsRow'
import { streamEvidence, evidenceEntries } from '@/lib/desk/evidence'
import { daySpendFromDigests, sessionRefusal, slashAction } from '@/lib/desk/sessions'
import { AgendaTab, CharterTab, CostTab, EvidenceTab, MemoryTab, RightPanel, StateTab, ToolsTab, type PanelTab, type RecordedLifecycle, type StateAbsence } from './RightPanel'
import { DigestCards } from './DigestCard'
import { DayBudgetBar, StageStrip, TierBadge } from './Shared'
import { initialTab, staleAge, makeResolver } from './Conversation.helpers'
import { isIrreversibleEnvelope } from '@/components/desk/Cards.helpers'
import { useDeskKeymap } from '@/components/desk/useDeskKeymap'
import { pendingApprovals } from '@/components/desk/RightPanel.helpers'
import { tierTiles } from '@/components/desk/Shared.helpers'
import { PROVIDERS_QUERY_KEY, llmKeysApi } from '@/api/llm'
import { ConnectProviderCta } from '@/components/providers/ConnectProviderCta'
import { conversationEngine } from '@/components/providers/engines'
import { PANEL_BECOMES_DRAWER_PX, viewportIsAtMost } from '@/lib/desk/viewport'

export function Conversation({
  conversationId,
  onOpenRun,
  onOpenDossier,
  onOpenConversation,
  onOpenPath,
}: {
  conversationId: string
  onOpenRun: (runId: string) => void
  onOpenDossier: (ticket: string) => void
  /** A child conversation of a continuous session's agenda item (§2b). */
  onOpenConversation?: (conversationId: string) => void
  /**
   * A desk path a digest line names — the memory browser for a finding, the
   * proposals queue for a proposal (§2.4). Passed in rather than taken from
   * `useNavigate` so this screen still renders outside a router in a test.
   */
  onOpenPath?: (to: string) => void
}) {
  /** Cards this screen renders itself, which are on no wire. See `onOffer`. */
  const [localCards, setLocalCards] = useState<DeskCard[]>([])
  const [events, setEvents] = useState<WireEvent[]>([])
  // `?tab=charter` is how desk home's **Amend** on a stalled session lands on
  // the charter rather than on the State tab. Read once, from the address the
  // desk was opened at; anything not a tab is ignored.
  const [tab, setTab] = useState<PanelTab>(() => initialTab())
  const [detailsAll, setDetailsAll] = useState(false)
  /**
   * Open beside the conversation, closed over it.
   *
   * Above 1200px this aside is the third grid column and there is no reason to
   * start it closed. Below 1200px `desk.css` makes it `position: fixed;
   * right: 0; width: min(380px, 92vw)` — at 375px that is 345 of 375 pixels laid
   * over the conversation, the controls row and the composer. Defaulting to open
   * there meant the first thing the desk asked a phone operator to do was
   * dismiss it, which is S-6.22's 375px clause. The read is unavailable in jsdom,
   * where `viewportIsAtMost` answers `false` and the panel starts open as it
   * always did.
   */
  const [sideOpen, setSideOpen] = useState(() => !viewportIsAtMost(PANEL_BECOMES_DRAWER_PX))
  const [model, setModel] = useState('')
  const composer = useRef<ComposerHandle>(null)
  const qc = useQueryClient()
  // An answer here leaves every list of pending approvals at once (S-6.15).
  const answered = useCallback(() => invalidateApprovals(qc), [qc])

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
  // Whether this conversation's provider still has this user's key behind it
  // (PROVIDERS track). Shared with every other reader of the provider list.
  const providerStatus = useQuery({
    queryKey: PROVIDERS_QUERY_KEY,
    queryFn: () => llmKeysApi.status(),
    staleTime: 30_000,
  })

  const latestTurn = convo.data?.turns?.[convo.data.turns.length - 1] ?? null
  const runId = latestTurn?.run_id ?? null
  // Three readings, because `=== true` gave a conversation that could not be
  // read the same badge as one sitting idle. `idle` is the calm word on a header
  // whose other words are `working` and `paused`, and a run nobody can see the
  // state of is not calm.
  const convoKnown = convo.data !== undefined && convo.data !== null
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

  // The whole stream, in one place. The card list, the Cost tab's per-call rows
  // and the Tools tab's exposure note are all folds of exactly this array, so
  // none of the three can disagree with the other two about what happened.
  const wire = useMemo<WireEvent[]>(
    () => [...(polled.data?.messages ?? []), ...events],
    [polled.data, events],
  )

  // Folded during render rather than accumulated by an effect. `wire` is already
  // the whole stream -- `events` only ever grows and the poll returns the lot --
  // so `cards` was a second copy of it that lagged by a render and could, in
  // principle, disagree with the Cost and Tools tabs the comment above promises
  // it cannot. The one thing that was *not* in `wire` is the locally rendered
  // offer card, so that keeps its own state and is merged on top.
  const cards = useMemo(() => mergeCards(toCards(wire), localCards), [wire, localCards])

  const state = useQuery({
    queryKey: ['desk', 'state', runId],
    queryFn: () => deskApi.state(runId as string),
    enabled: !!runId,
    refetchInterval: running ? 4_000 : false,
  })

  // The run row, read for one reason: it is where the pause's server-side truth
  // lives (ADR-D-27). Polled while the run is live rather than read once,
  // because the deadline expires on the server's clock and a pause that has
  // already lapsed into a stop must stop offering Resume. The previous version
  // of this control kept a local `paused` flag instead, which is exactly the
  // defect that got it deleted.
  const run = useQuery({
    queryKey: ['desk', 'run', runId],
    queryFn: () => deskApi.run(runId as string).then((r) => r.data),
    enabled: !!runId,
    refetchInterval: running ? 4_000 : false,
  })

  // The run's memory brief, read only while the Memory tab is open.
  const brief = useQuery({
    queryKey: ['desk', 'brief', runId],
    queryFn: () => deskApi.brief(runId as string),
    enabled: !!runId && tab === 'memory',
  })

  // The run's hypotheses with the numbers that decided each verdict, read while
  // the State tab is open (S-10.15). From the ledger, so it answers whether or
  // not the working-state snapshot route is mounted.
  const hypotheses = useQuery({
    queryKey: ['desk', 'hypotheses', runId],
    queryFn: () => deskApi.hypotheses(runId as string),
    enabled: !!runId && tab === 'state',
  })

  // `/api/catalogue` has been served since S-6.19 and nothing in the desk's own
  // conversation ever called it, so the Tools tab rendered "not built yet" over
  // a built endpoint. The catalogue is a registry, not a run, so it is cached
  // rather than polled.
  const catalogue = useQuery({
    queryKey: ['desk', 'catalogue'],
    queryFn: () => deskApi.catalogue(),
    staleTime: 5 * 60_000,
  })

  // ── The session this conversation is, if it is one (§2b) ──────────────────
  //
  // Two calls, because the list is the only place a conversation id maps to a
  // session id and the charter is only on the detail. Both routes are served and
  // neither had a caller: the panel hard-coded `continuous = false`, so the
  // Agenda and Charter tabs could not be opened at all, on a session whose whole
  // point is that board.
  const sessions = useQuery({
    queryKey: ['desk', 'sessions'],
    queryFn: () => deskApi.sessions(),
    staleTime: 30_000,
  })
  const sessionRow = useMemo(
    () => sessionForConversation(sessions.data?.sessions, conversationId),
    [sessions.data, conversationId],
  )
  const sessionId = sessionRow?.session_id ?? null
  const session = useQuery({
    queryKey: ['desk', 'session', sessionId],
    queryFn: () => deskApi.session(sessionId as string),
    enabled: !!sessionId,
    refetchInterval: running ? 10_000 : false,
  })
  const continuous = session.data?.continuous === true
  const agenda = useQuery({
    queryKey: ['desk', 'agenda', sessionId],
    queryFn: () => deskApi.agenda(sessionId as string),
    enabled: !!sessionId && continuous,
    refetchInterval: running ? 5_000 : false,
  })
  // The digests, which are what makes coming back to a week-old session cheap
  // (AGENT-014 §2.4). They render as cards in the journal — this conversation —
  // and only here: a child conversation has its own dossier, not the session's
  // review.
  const digests = useQuery({
    queryKey: ['desk', 'digests', sessionId],
    queryFn: () => deskApi.digests(sessionId as string),
    enabled: !!sessionId && continuous,
    refetchInterval: running ? 30_000 : false,
  })

  // The session's own event log, pushed (AGENT-014 §10; S-11.14's "SSE delivers
  // agenda transitions to an open journal within one event"). Each frame is a
  // session event — a claim, a veto, a reorder, an amendment — and the journal
  // re-reads the three things an event can change rather than folding the log
  // a second time in the browser. The polls below stay as the fallback.
  const refetchAgenda = agenda.refetch
  const refetchSession = session.refetch
  const refetchDigests = digests.refetch
  const onSessionEvent = useCallback(() => {
    void refetchAgenda()
    void refetchSession()
    void refetchDigests()
  }, [refetchAgenda, refetchSession, refetchDigests])
  useDeskStream({
    url: sessionId && continuous ? `/api/research/sessions/${sessionId}/events` : null,
    onEvent: onSessionEvent,
  })

  const openDigestRef = useCallback(
    (ref: SessionDigestRef) => {
      const target = sessionDigestTarget(ref)
      if (target.kind === 'run') onOpenRun(target.runId)
      else if (target.kind === 'panel') setTab(target.tab)
      else if (target.kind === 'route') onOpenPath?.(target.to)
      // `none` never reaches here: the card renders those lines as text with
      // the reason, rather than as a control that eats the click.
    },
    [onOpenRun, onOpenPath],
  )

  const workingState = state.data?.served ? state.data.data : null
  // Why there is no working state, which the panel has to say and could not.
  //
  // Three cases collapsed into one `null` before: no run at all, a route that
  // answered, and a route that is not mounted. `/api/agent/runs/{id}/state` is
  // the third — an AGENT-012 §17 ✗ row — so the State and Evidence tabs printed
  // "no working state yet" and "this run has produced no ids yet" over every
  // conversation on this platform, both of which read as facts about the run.
  const stateAbsence: StateAbsence | undefined = !runId
    ? 'no_run'
    : state.data && !state.data.served
      ? 'not_served'
      : undefined
  const resolve = useMemo(() => makeResolver(workingState, cards), [workingState, cards])
  // Every id the run's tools returned, with the stored result it opens.
  const evidence = useMemo(() => streamEvidence(cards), [cards])
  // The id a chip elsewhere on the desk asked to open. A chip that only switched
  // tabs left the reader hunting for the id it was clicked on.
  const [focusedId, setFocusedId] = useState<string | null>(null)
  const openEvidence = useCallback((id: string) => {
    setFocusedId(id)
    setTab('evidence')
  }, [])
  const approvals = useMemo(() => pendingApprovals(cards), [cards])

  // §15's live regions, and the browser notification behind them (S-6.22).
  //
  // `running` is passed three-valued. `convo.data?.conversation.running ===
  // true` is `false` for a conversation that has not loaded and for one the
  // platform refused, and feeding that `false` in would announce **run
  // finished** every time the desk lost sight of a run — the same defect as the
  // header's old `paused` badge, said out loud.
  useDeskAnnouncer({
    conversationId,
    cards,
    running: convoKnown ? running : null,
    title: convo.data?.conversation.title ?? null,
  })

  // The two notes that were already on this conversation's stream and that the
  // right panel was passed a literal `[]` and `null` instead. `calls` is `null`
  // with no run, because "nobody asked" and "no call was made" are different
  // sentences and the Cost tab prints a different one for each.
  const calls = useMemo(() => (runId ? costCalls(wire) : null), [wire, runId])
  const exposure = useMemo(() => latestExposure(wire), [wire])
  // The third of them. `search_tools` is answered by the harness itself and
  // recorded as a `core_tool` note, so a run's searches have been on this same
  // array since the desk was written — the Tools tab was handed `[]`.
  const searches = useMemo(() => toolSearches(wire), [wire])

  // The stage of each running child, for the agenda board's running cards.
  // `/api/desk/running` is the one served read that carries a run's stage; it
  // is joined by conversation id because that is the key an agenda item has.
  const running_ = useQuery({
    queryKey: ['desk', 'running'],
    queryFn: () => deskApi.running(),
    // Read while this conversation has a live run (its own row carries mode,
    // workflow, ring and stage) and while the agenda board is open.
    enabled: (!!runId && running) || (continuous && tab === 'agenda'),
    refetchInterval: running ? 10_000 : false,
  })
  const childStages = useMemo(() => {
    const out: Record<string, Stage> = {}
    for (const r of running_.data?.runs ?? []) {
      if (r.conversation_id && r.stage) out[r.conversation_id] = r.stage as Stage
    }
    return out
  }, [running_.data])

  // This run's own row on the running list: the lifecycle columns the header
  // and State tab show when the working state is not served (S-10.15).
  const recorded = useMemo<RecordedLifecycle | null>(() => {
    const r = running_.data?.runs.find((x) => x.run_id === runId)
    return r ? { mode: r.mode, workflow: r.workflow, ring: r.ring ?? null, stage: r.stage as Stage } : null
  }, [running_.data, runId])

  /** What the last slash command or session action said, when it said no. */
  const [slashNote, setSlashNote] = useState<string | null>(null)

  const steer = useCallback(
    (text: string) => {
      if (!runId) return
      void deskApi.steer(runId, { kind: 'steer', text })
    },
    [runId],
  )

  function runSlash(cmd: string, rest: string) {
    setSlashNote(null)
    const act = slashAction(cmd, rest, { continuous, hasRun: !!runId })
    if (!act) return
    const failed = (e: unknown) => setSlashNote(`${cmd}: ${sessionRefusal(e)}`)
    switch (act.kind) {
      case 'steer':
        steer(act.text)
        return
      case 'tab':
        setTab(act.tab)
        return
      case 'refuse':
        setSlashNote(act.why)
        return
      case 'session':
        if (!sessionId) return setSlashNote(`${cmd}: the session behind this journal could not be read.`)
        deskApi
          .sessionAction(sessionId, act.action)
          .then(() => void session.refetch())
          .catch(failed)
        return
      case 'run':
        if (!runId) return
        ;(act.action === 'pause' ? deskApi.pauseRun(runId) : deskApi.resumeRun(runId))
          .then(() => void run.refetch())
          .catch(failed)
        return
      case 'agenda_add':
        if (!sessionId) return setSlashNote(`${cmd}: the session behind this journal could not be read.`)
        deskApi
          .addAgendaItem(sessionId, { question: act.question })
          .then(() => {
            void agenda.refetch()
            setTab('agenda')
          })
          .catch(failed)
        return
    }
  }

  // Whether the approval at the front of the queue is one the platform will
  // refuse without a written reason. The keyboard cannot supply one, so `a` and
  // the row's Approve must stand down and point at the card instead of firing a
  // request that comes back 400 into a `void`.
  const frontApprovalNeedsReason = useMemo(
    () => isIrreversibleEnvelope((approvals[0]?.detail?.envelope as string) ?? null),
    [approvals],
  )

  // Three states, from the server, with `null` kept apart from `false`. A run
  // this replica is not driving reports `paused: null`, and collapsing that to
  // "not paused" would draw a Pause button whose request answers 409 — the
  // control that was deleted, rebuilt one layer down.
  const pause: PauseState = useMemo(() => {
    const row = run.data
    if (!row || row.paused === null || row.paused === undefined) return { kind: 'unobserved' }
    return row.paused ? { kind: 'paused', until: row.paused_until } : { kind: 'running' }
  }, [run.data])

  const controls = {
    busy: running,
    disconnected: stream.state === 'stale' || stream.state === 'closed',
    hasPendingApproval: approvals.length > 0,
    approvalNeedsReason: frontApprovalNeedsReason,
    detailsOpen: detailsAll,
    pause,
    onStop: () => void deskApi.cancel(conversationId),
    // Both refetch rather than setting anything locally: the server owns this
    // state and the reply is a confirmation, not the source.
    onPause: () => {
      if (!runId) return
      void deskApi.pauseRun(runId).finally(() => void run.refetch())
    },
    onResume: () => {
      if (!runId) return
      void deskApi.resumeRun(runId).finally(() => void run.refetch())
    },
    onSteer: () => composer.current?.focus(),
    onApprove: () => {
      const a = approvals[0]
      const id = (a?.detail?.approval_id as string) ?? null
      // No note from here, by construction: the row only offers this path for
      // approvals that do not need one.
      if (id && !frontApprovalNeedsReason) void deskApi.answerApproval(id, 'approve').then(answered)
    },
    onDeny: () => {
      const a = approvals[0]
      const id = (a?.detail?.approval_id as string) ?? null
      if (id) void deskApi.answerApproval(id, 'deny').then(answered)
    },
    onBudget: () => setTab('cost'),
    onToggleDetails: () => setDetailsAll((d) => !d),
    onReplay: () => runId && onOpenRun(runId),
    onFocusComposer: () => composer.current?.focus(),
    onShortcuts: () =>
      window.alert(
        'esc stop · space pause/resume · a approve · x deny · s steer · d details · r replay · / composer · ? this list',
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
      // A new session is a new journal, so the desk goes there — and a refusal
      // (a local profile, `replenish` before S-11.11) is printed, not dropped
      // into a `void` where it read as a session that started.
      setSlashNote(null)
      // Opened on this conversation's provider and model. Without a
      // `conversation_id` the server opens the journal on its default provider,
      // whatever key this user connected (PROVIDERS track).
      const opened = engine
        ? conversationsApi
            .create({ provider: engine.choice.provider, model: engine.choice.model })
            .then((r) => r.data.conversation_id)
        : Promise.resolve(undefined)
      opened
        .then((journal) => {
          const body = {
            mode: s.mode,
            continuous: true,
            charter: s.charter as NonNullable<typeof s.charter>,
            workflow: s.workflow,
            model: engine?.choice.model ?? chosen?.id,
            ...(journal ? { conversation_id: journal } : {}),
          }
          return deskApi.createSession(body)
        })
        .then((created) => {
          if (created) onOpenConversation?.(created.conversation_id)
          else setSlashNote('The session route did not answer; no session was started.')
        })
        .catch((e: unknown) => setSlashNote(`No session was started: ${sessionRefusal(e)}`))
      return
    }
    void conversationsApi.send(conversationId, `${preamble}\n${s.question}`)
  }

  // `tierTiles`, shared with desk home, shared in turn with the Models page's
  // `profileIsMeasured`. This was the **third** copy of
  // `p.multi_step_evidence !== null || !p.is_local`: S-6.20 removed it from the
  // Models page and the two surfaces that draw the same badge kept it. This one
  // is the badge in the conversation header — the one beside a running turn,
  // and the one the composer's "unmeasured" warning is supposed to agree with.
  const profileList = tierTiles(profiles.data?.profiles)
  // The engine this conversation was opened on. Its provider and model are
  // fixed for its life, so the header badge and the composer show it rather
  // than whichever profile happens to be first in the list.
  const engine = conversationEngine(convo.data?.conversation, providerStatus.data, profiles.data?.profiles)
  const chosen = engine?.choice ?? profileList.find((p) => p.id === model) ?? profileList[0]

  return (
    <>
      <div className="desk-hd">
        <span className="title">{convo.data?.conversation.title ?? 'Conversation'}</span>
        {chosen && <TierBadge tier={chosen.tier} measured={chosen.measured} detail={chosen.label} />}
        {!workingState && recorded && (
          <>
            <Badge tone="outline">{recorded.mode}</Badge>
            {recorded.workflow && <Badge tone="outline">{recorded.workflow}</Badge>}
            <Badge tone={recorded.ring === 'research' ? 'neutral' : 'warn'} data-testid="header-ring">
              ring: {recorded.ring ?? 'not reported'}
            </Badge>
            <StageStrip stage={recorded.stage} />
          </>
        )}
        {workingState && (
          <>
            <Badge tone="outline">{workingState.mode}</Badge>
            <Badge tone="outline">{workingState.workflow}</Badge>
            <Badge tone={workingState.ring === 'research' ? 'neutral' : 'warn'}>ring: {workingState.ring}</Badge>
            <StageStrip stage={workingState.stage} />
          </>
        )}
        {/* Three readings, not four. `paused` was the fourth, and it was a
            local flag set by a button whose route is not mounted — so this badge
            said **paused** over a run that was still working. A status badge
            that reports the click rather than the platform is worse than no
            badge. */}
        <Badge tone={running ? 'accent' : convoKnown ? 'neutral' : 'warn'} data-testid="convo-status">
          {running ? 'working' : convoKnown ? 'idle' : 'status unknown'}
        </Badge>
        {/* §2b: a session's journal says so in the header, with its agenda
            depth. The depth is the list's `agenda_open`, which is `null` when
            the session log did not replay — shown as "unknown" rather than as
            a depth of zero, which would read as a session with nothing left. */}
        {continuous && (
          <>
            <Badge tone="info">continuous</Badge>
            <span className="sub">
              agenda{' '}
              {sessionRow?.agenda_open === null || sessionRow?.agenda_open === undefined
                ? '(unknown)'
                : sessionRow.agenda_open}
            </span>
            {/* §9's day budget bar, drawn only when there is a spend to draw.
                Nothing served counts dollars against a day, so `spentUsd` is
                `null` from here and the header says the budget and that the
                spend is not recorded. An empty track would say nothing was
                spent today, which nobody measured. */}
            <DayBudgetBar
              usdPerDay={session.data?.budget?.usd_per_day ?? null}
              spentUsd={daySpendFromDigests(digests.data?.digests)}
              basis="digests"
            />
          </>
        )}
        {stream.state === 'stale' &&
          (staleAge(stream.staleForSecs) === null ? (
            <Badge tone="warn" data-testid="stream-never-connected">
              disconnected · nothing received
            </Badge>
          ) : (
            <StaleBadge age={staleAge(stream.staleForSecs) as string} />
          ))}
        {/* A closed stream drew nothing at all, while still disabling the
            controls through `controls.disconnected` — so "you are not signed
            in" and "this conversation is not yours" both arrived as a dead
            toolbar with no sentence attached. The hook already knows which. */}
        {stream.state === 'closed' && stream.error && (
          <Badge tone="warn" data-testid="stream-closed-reason">
            not live · {stream.error}
          </Badge>
        )}
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

      <div className="desk-body">
        <div className="desk-center">
      <div className="desk-scroll">
        {/* The journal (§2b): the digests the platform wrote about this session,
            above the turns, newest first. A bounded conversation has none and
            asks for none. */}
        {continuous && (
          <section aria-label="Digests">
            <DigestCards
              digests={digests.data?.digests ?? null}
              loading={digests.isPending}
              onOpen={openDigestRef}
            />
          </section>
        )}
        <CardStream
          // A different conversation is a different stream with its own
          // remembered disclosure set; remounting is how that is said.
          key={conversationId}
          conversationId={conversationId}
          cards={cards}
          resolve={resolve}
          detailsAll={detailsAll}
          actions={{
            onSteer: steer,
            onOpenDossier,
            // The second argument is the reason the card asked the person to
            // type. It was declared on `CardActions`, sent by `ApproveControls`
            // and dropped here, so `spend_holdout_look` — which refuses without
            // twenty characters of it — saw an approval with no reason at all,
            // and the person watched their justification vanish between the
            // field and the wire.
            onApprove: (id, note) => void deskApi.answerApproval(id, 'approve', note).then(answered),
            onDeny: (id) => void deskApi.answerApproval(id, 'deny').then(answered),
            onAnswer: (_id, text) => runId && void deskApi.steer(runId, { kind: 'answer', text }),
            onStartResearch: (q) => void conversationsApi.send(conversationId, `mode=research\n${q}`),
            onOpenId: openEvidence,
          }}
        />
      </div>

      <div className="desk-dock">
        <ControlsRow {...controls} />
        {slashNote && (
          <p className="sub" role="alert" data-testid="slash-note">
            {slashNote}
          </p>
        )}
        <Composer
          ref={composer}
          running={running}
          profiles={engine ? [engine.choice] : profileList}
          model={chosen?.id ?? ''}
          onModelChange={setModel}
          connect={
            engine?.keyMissing ? (
              <ConnectProviderCta
                detail="This chat was started on an AI service you have no API key for. Start a new chat, or add an API key to continue this one."
              />
            ) : undefined
          }
          onSubmit={submit}
          // §3.3: "**Write strategy** is an action beside *attach* (opens the
          // SLv2 editor, §8)". That editor is `StrategyEditor`, mounted by
          // `DeskPage` at `/desk/strategy` and already in the desk's own nav —
          // not `/strategy`, which is the older `StrategyCreationPage`.
          //
          // The editor's compile call is itself unbuilt, and the editor says so
          // in its own copy rather than pretending. That is the difference this
          // whole change turns on: a page that states what it cannot do is a
          // destination; a button that states nothing is not.
          //
          // Passed only when a navigator was given, because `Composer` draws
          // the button only when it has somewhere to go.
          onWriteStrategy={onOpenPath ? () => onOpenPath('/desk/strategy') : undefined}
          onSlash={runSlash}
          onOffer={(q) => {
            // Until S-10.2 classifies server-side, the offer is rendered locally
            // as a card so the behaviour is real rather than promised.
            setLocalCards((prev) =>
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

        </div>

      <aside className="desk-side" hidden={!sideOpen}>
        <RightPanel tab={tab} onTab={setTab} continuous={continuous}>
          {tab === 'state' && (
            <StateTab
              state={workingState}
              absence={stateAbsence}
              resolve={resolve}
              pendingApproval={approvals.length > 0}
              recorded={recorded}
              onOpenId={openEvidence}
              verdicts={
                hypotheses.isError ? 'unreadable' : hypotheses.data?.served ? hypotheses.data.data.hypotheses : null
              }
            />
          )}
          {tab === 'evidence' && (
            <EvidenceTab
              resolve={resolve}
              absence={stateAbsence}
              // The stored tool results on the stream, then whatever the working
              // state adds. The state route is not mounted, so the stream is the
              // source that actually answers on this platform.
              entries={evidenceEntries(workingState, evidence)}
              focusedId={focusedId}
            />
          )}
          {tab === 'memory' && (
            <MemoryTab
              brief={runId ? (brief.isError ? null : brief.data) : undefined}
              absence={runId ? undefined : 'no_run'}
              onForget={(bucket, id) => deskApi.forget(bucket, id).then(() => void brief.refetch())}
            />
          )}
          {tab === 'tools' && (
            <ToolsTab
              mode={workingState?.mode ?? 'chat'}
              exposed={exposure?.exposed ?? null}
              exposure={exposure}
              catalogue={catalogue.data?.tools ?? null}
              catalogueLoading={catalogue.isPending}
              searchedFor={searches}
              // Only into a run this conversation has: a pin posted at nothing
              // would be the inert button this prop replaced.
              onPin={runId ? (tool) => deskApi.pinTool(runId, tool) : undefined}
            />
          )}
          {tab === 'cost' && (
            <CostTab calls={calls} bytesPerTokenCalibrated={null} bytesPerTokenEstimate={null} />
          )}
          {tab === 'agenda' && (
            <AgendaTab
              items={agenda.data?.items ?? null}
              loading={agenda.isPending}
              onVeto={(id) => {
                if (!sessionId) return
                void deskApi.vetoAgendaItem(sessionId, id).then(() => void agenda.refetch())
              }}
              // The board has drawn this button since it was written and
              // nothing was passed to it, so under `supervised` — where every
              // item waits for a human — the control that releases one reached
              // nothing.
              onApprove={(id) => {
                if (!sessionId) return
                void deskApi.approveAgendaItem(sessionId, id).then(() => void agenda.refetch())
              }}
              // A drag is a `user` priority event, and what goes up is the
              // board's whole order: the route subtracts the cards that did not
              // move and refuses a one-id body, so the short list the client
              // used to compute is the one shape it cannot post.
              onReorder={(order) => {
                if (!sessionId) return
                void deskApi.reorderAgenda(sessionId, order).then(() => void agenda.refetch())
              }}
              onOpenChild={(cid) => onOpenConversation?.(cid)}
              childStages={childStages}
              onAdd={(question) => {
                if (!sessionId) return Promise.reject(new Error('the session could not be read'))
                return deskApi.addAgendaItem(sessionId, { question }).then(() => void agenda.refetch())
              }}
            />
          )}
          {tab === 'charter' && (
            <CharterTab
              charter={charterView(session.data)}
              // The session read carries the `charter_amended` events with a
              // diff on each. The tab reported the version number and said no
              // route read them back, which stopped being true.
              amendments={sessionAmendments(session.data)}
              raw={session.data?.charter ?? null}
              onAmend={(next) => {
                if (!sessionId) return Promise.reject(new Error('the session could not be read'))
                return deskApi.amendCharter(sessionId, next).then(() => void session.refetch())
              }}
              loading={session.isPending && !!sessionId}
            />
          )}
        </RightPanel>
      </aside>
      </div>
    </>
  )
}
