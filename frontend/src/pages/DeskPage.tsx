// The Research Desk (AGENT-012; task S-6.1).
//
// One surface, replacing `/agent` (which polled and rendered sixteen message
// kinds through one row component) and `/research` (which consumed SSE and
// rendered a second, unrelated card). Both were written in the deprecated
// back-compat Tailwind aliases; nothing under `components/desk/` is, and
// `no-deprecated-aliases.test.ts` asserts it rather than leaving it to review.
//
// Routing lives here rather than in `App.tsx` so the shared file takes one route
// line instead of eleven.
import { useState } from 'react'
import { Navigate, Route, Routes, useNavigate, useParams } from 'react-router-dom'
import { useQueries, useQuery } from '@tanstack/react-query'
import { Button } from '@/components/primitives/Button'
import { conversationsApi } from '@/api/agent'
import { deskApi } from '@/api/desk'
import { childrenFromAgendas, railGroups } from '@/lib/desk/rail'
import { RAIL_BECOMES_SHEET_PX, viewportIsAtMost } from '@/lib/desk/viewport'
import { ApprovalsInbox } from '@/components/desk/ApprovalsInbox'
import { Catalogue } from '@/components/desk/Catalogue'
import { Conversation } from '@/components/desk/Conversation'
import { DeskHome } from '@/components/desk/DeskHome'
import { DossierReader } from '@/components/desk/Dossier'
import { MemoryBrowser } from '@/components/desk/MemoryBrowser'
import { ProposalsQueue } from '@/components/desk/ProposalsQueue'
import { Models } from '@/components/desk/Models'
import { Monitor, StrategyEditor, Usage } from '@/components/desk/Operations'
import { RunDetail } from '@/components/desk/RunDetail'

function Rail({ onNew, hidden }: { onNew: () => void; hidden?: boolean }) {
  const navigate = useNavigate()
  const { conversationId } = useParams()
  const list = useQuery({
    queryKey: ['desk', 'conversations'],
    queryFn: () => conversationsApi.list().then((r) => r.data),
    refetchInterval: 15_000,
  })
  // §2b: a continuous session's children are grouped under its journal and
  // carry a chip back to the agenda card that started them. The edge comes from
  // the agenda — `child_conversation_id` — because the conversations route does
  // not send `parent_conversation_id`, which `railGroups` explains at length.
  const sessions = useQuery({
    queryKey: ['desk', 'sessions'],
    queryFn: () => deskApi.sessions(),
    staleTime: 30_000,
  })
  const continuous = (sessions.data?.sessions ?? []).filter((s) => s.continuous)
  const agendas = useQueries({
    queries: continuous.map((s) => ({
      queryKey: ['desk', 'agenda', s.session_id],
      queryFn: () => deskApi.agenda(s.session_id),
      staleTime: 30_000,
    })),
  })
  const chips = childrenFromAgendas(
    continuous.map((s, i) => ({
      session_id: s.session_id,
      conversation_id: s.conversation_id,
      items: agendas[i]?.data?.items ?? [],
    })),
  )
  const groups = railGroups(list.data?.conversations ?? [], sessions.data?.sessions, chips)

  function label(c: { conversation_id: string; title: string | null }) {
    return c.title ?? c.conversation_id.slice(0, 8)
  }

  return (
    <nav className="desk-rail" hidden={hidden} aria-label="Conversations and desk sections">
      <div className="desk-rail-hd">
        <span className="lbl">Conversations</span>
        <Button size="xs" variant="ghost" onClick={onNew}>
          New
        </Button>
      </div>
      <ul>
        {groups.map((g) => (
          <li key={g.conversation.conversation_id}>
            <button
              type="button"
              aria-current={g.conversation.conversation_id === conversationId}
              onClick={() => navigate(`/desk/c/${g.conversation.conversation_id}`)}
            >
              {g.journal ? `${label(g.conversation)} · journal` : label(g.conversation)}
            </button>
            {g.children.length > 0 && (
              <ul aria-label={`Runs under ${label(g.conversation)}`}>
                {g.children.map((child) => (
                  <li key={child.conversation.conversation_id}>
                    <button
                      type="button"
                      aria-current={child.conversation.conversation_id === conversationId}
                      onClick={() => navigate(`/desk/c/${child.conversation.conversation_id}`)}
                    >
                      {label(child.conversation)}
                    </button>
                    {child.chip && (
                      // The chip back to the agenda card. It opens the journal,
                      // which is where the board is — there is no route to a
                      // single item, and the board is what the card is on.
                      <button
                        type="button"
                        className="badge outline"
                        title={child.chip.question}
                        onClick={() => navigate(`/desk/c/${child.chip?.conversation_id}`)}
                      >
                        agenda: {child.chip.question}
                      </button>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </li>
        ))}
      </ul>
      <div className="desk-rail-hd">
        <span className="lbl">Desk</span>
      </div>
      <ul>
        {[
          ['/desk', 'Home'],
          ['/desk/approvals', 'Approvals'],
          // Separate from Approvals on purpose (AGENT-013 §7.5): that inbox
          // answers a *running* agent's pause, and this queue holds inert
          // requests decided in their own time. One list would put a timer's
          // urgency on decisions nothing is waiting for.
          ['/desk/proposals', 'Proposals'],
          ['/desk/memory', 'Memory'],
          ['/desk/catalogue', 'Catalogue'],
          ['/desk/models', 'Models'],
          ['/desk/monitor', 'Monitor'],
          ['/desk/usage', 'Usage'],
          ['/desk/strategy', 'Strategy editor'],
        ].map(([path, label]) => (
          <li key={path}>
            <button type="button" onClick={() => navigate(path)}>
              {label}
            </button>
          </li>
        ))}
      </ul>
    </nav>
  )
}

function ConversationRoute() {
  const { conversationId } = useParams()
  const navigate = useNavigate()
  if (!conversationId) return <Navigate to="/desk" replace />
  return (
    <Conversation
      conversationId={conversationId}
      onOpenRun={(runId) => navigate(`/desk/c/${conversationId}/run/${runId}`)}
      onOpenDossier={(ticket) => navigate(`/desk/c/${conversationId}/dossier/${ticket}`)}
      // A continuous session's agenda card links to the child conversation that
      // ran it (§2b), which is a sibling route rather than one nested under this
      // conversation.
      onOpenConversation={(id) => navigate(`/desk/c/${id}`)}
      // Where a digest line goes when it names something that lives on another
      // desk page — a finding in memory, a proposal in the queue.
      onOpenPath={(to) => navigate(to)}
    />
  )
}

function RunRoute() {
  const { conversationId, runId } = useParams()
  const navigate = useNavigate()
  if (!runId) return <Navigate to="/desk" replace />
  return <RunDetail runId={runId} onBack={() => navigate(`/desk/c/${conversationId}`)} />
}

function DossierRoute() {
  const { ticket } = useParams()
  if (!ticket) return <Navigate to="/desk" replace />
  return <DossierReader ticket={ticket} />
}

export function DeskPage() {
  const navigate = useNavigate()
  /**
   * A column beside the conversation, a sheet over it.
   *
   * Under 768px `desk.css` makes the rail `position: fixed; left: 0;
   * width: min(280px, 88vw)`, so at 375px an open rail covers 280 of 375 pixels
   * of the conversation it is meant to navigate. `viewportIsAtMost` answers
   * `false` where the read is unavailable, so every existing test and every
   * desk-width browser still gets the rail open.
   */
  const [railOpen, setRailOpen] = useState(() => !viewportIsAtMost(RAIL_BECOMES_SHEET_PX))

  async function newConversation() {
    const { data } = await conversationsApi.create()
    navigate(`/desk/c/${data.conversation_id}`)
  }

  function startFrom(seed: string) {
    void (async () => {
      const { data } = await conversationsApi.create()
      await conversationsApi.send(data.conversation_id, seed)
      navigate(`/desk/c/${data.conversation_id}`)
    })()
  }

  return (
    <div className="desk">
      <Rail hidden={!railOpen} onNew={() => void newConversation()} />
      <div className="desk-main">
        <Button
          size="xs"
          variant="ghost"
          className="desk-rail-toggle"
          onClick={() => setRailOpen((o) => !o)}
          aria-label="Toggle the conversation rail"
        >
          ☰
        </Button>
        <Routes>
          <Route index element={<DeskHome />} />
          <Route path="c/:conversationId" element={<ConversationRoute />} />
          <Route path="c/:conversationId/run/:runId" element={<RunRoute />} />
          <Route path="c/:conversationId/dossier/:ticket" element={<DossierRoute />} />
          <Route
            path="approvals"
            element={
              <div className="desk-scroll">
                <ApprovalsInbox />
              </div>
            }
          />
          <Route path="proposals" element={<ProposalsQueue />} />
          <Route path="memory" element={<MemoryBrowser onOpenConversation={startFrom} />} />
          <Route path="catalogue" element={<Catalogue />} />
          <Route path="models" element={<Models />} />
          <Route path="monitor" element={<Monitor />} />
          <Route path="usage" element={<Usage />} />
          <Route path="strategy" element={<StrategyEditor />} />
          <Route path="dossiers" element={<Navigate to="/desk/memory" replace />} />
          <Route path="settings" element={<Navigate to="/settings" replace />} />
          <Route path="*" element={<Navigate to="/desk" replace />} />
        </Routes>
      </div>
    </div>
  )
}
