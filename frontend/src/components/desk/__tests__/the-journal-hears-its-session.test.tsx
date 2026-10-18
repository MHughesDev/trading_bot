/**
 * S-11.13 / S-11.14 — the journal of a continuous session subscribes to the
 * session's own event log (`GET /api/research/sessions/{id}/events`), so a
 * claim, a veto or a reorder reaches an open journal within one event rather
 * than on the next five-second poll. And an ordinary chat subscribes to no
 * session stream at all.
 */
import { render, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { describe, expect, it, vi } from 'vitest'

const streams = vi.hoisted(() => ({ urls: [] as Array<string | null>, handlers: new Map<string, () => void>() }))
vi.mock('@/hooks/useDeskStream', () => ({
  useDeskStream: (o: { url: string | null; onEvent: () => void }) => {
    streams.urls.push(o.url)
    if (o.url) streams.handlers.set(o.url, o.onEvent)
    return { state: 'closed', cursor: null, staleForSecs: null, everReceived: false, error: null, reconnect: () => {} }
  },
}))
vi.mock('@/hooks/useDeskAnnouncer', () => ({ useDeskAnnouncer: () => {} }))

const desk = vi.hoisted(() => {
  const nil = () => Promise.resolve(null)
  return {
    sessions: vi.fn(),
    session: vi.fn(),
    agenda: vi.fn(),
    digests: vi.fn(() => Promise.resolve({ digests: [] })),
    catalogue: vi.fn(nil),
    running: vi.fn(() => Promise.resolve({ runs: [], not_recorded: [] })),
    state: vi.fn(() => Promise.resolve({ served: false })),
    run: vi.fn(nil),
    messages: vi.fn(() => Promise.resolve({ data: { messages: [], last_seq: 0 } })),
  }
})
const convo = vi.hoisted(() => ({
  get: vi.fn(() => Promise.resolve({ data: { conversation: { title: 'journal', running: false }, turns: [] } })),
  listProfiles: vi.fn(() => Promise.resolve({ data: { profiles: [] } })),
}))
vi.mock('@/api/desk', async (orig) => ({ ...(await orig<object>()), deskApi: desk }))
vi.mock('@/api/agent', async (orig) => ({ ...(await orig<object>()), conversationsApi: convo }))

import { Conversation } from '../Conversation'

function open(conversationId: string) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <Conversation conversationId={conversationId} onOpenRun={() => {}} onOpenDossier={() => {}} />
    </QueryClientProvider>,
  )
}

const row = {
  session_id: 's_1',
  conversation_id: 'c_journal',
  continuous: true,
  question: null,
  tier: 'frontier',
  created_at: '2026-10-01T00:00:00Z',
  state: 'running',
  runs_started: 1,
  agenda_open: 2,
}

describe('the journal hears its session', () => {
  it('subscribes to the session event log, and a frame re-reads the board', async () => {
    desk.sessions.mockResolvedValue({ sessions: [row] })
    desk.session.mockResolvedValue({ session_id: 's_1', conversation_id: 'c_journal', continuous: true, charter: null, budget: null, state: { kind: 'running' } })
    desk.agenda.mockResolvedValue({ items: [] })
    open('c_journal')
    await waitFor(() => expect(streams.urls).toContain('/api/research/sessions/s_1/events'))
    const before = desk.agenda.mock.calls.length
    streams.handlers.get('/api/research/sessions/s_1/events')?.()
    await waitFor(() => expect(desk.agenda.mock.calls.length).toBeGreaterThan(before))
  })

  it('an ordinary chat opens no session stream', async () => {
    streams.urls.length = 0
    desk.sessions.mockResolvedValue({ sessions: [] })
    open('c_chat')
    await waitFor(() => expect(desk.sessions).toHaveBeenCalled())
    expect(streams.urls.filter((u) => u?.startsWith('/api/research/sessions'))).toEqual([])
  })
})
