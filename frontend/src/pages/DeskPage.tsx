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
import { useQuery } from '@tanstack/react-query'
import { Button } from '@/components/primitives/Button'
import { conversationsApi } from '@/api/agent'
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
  return (
    <nav className="desk-rail" hidden={hidden} aria-label="Conversations and desk sections">
      <div className="desk-rail-hd">
        <span className="lbl">Conversations</span>
        <Button size="xs" variant="ghost" onClick={onNew}>
          New
        </Button>
      </div>
      <ul>
        {(list.data?.conversations ?? []).map((c) => (
          <li key={c.conversation_id}>
            <button
              type="button"
              aria-current={c.conversation_id === conversationId}
              onClick={() => navigate(`/desk/c/${c.conversation_id}`)}
            >
              {c.title ?? c.conversation_id.slice(0, 8)}
            </button>
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
  const [railOpen, setRailOpen] = useState(true)

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
