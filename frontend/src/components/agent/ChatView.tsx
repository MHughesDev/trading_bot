// The conversation itself.
//
// A transcript and a composer, and nothing else. The user types; the agent decides
// what to look at, how many backtests to run and when it is done.
//
// # Why it polls rather than streams
//
// The agent keeps working whether or not anyone is watching — it is a detached task
// writing to Postgres. That makes "what happened" a query, and reconnecting after a
// closed tab the same code path as the first load. A socket would be faster to first
// token and would need a second path for the reconnect case, which is the one that
// has to be right.

import { useEffect, useMemo, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { ArrowUp, Bot, Loader2, Square } from 'lucide-react'
import { agentApi, conversationsApi, type AgentMessage, type Turn } from '@/api/agent'
import { ActivityRow } from './AgentActivity'
import { indexResults } from './activity'

const ACTIVE = new Set(['queued', 'running', 'waiting_backtest', 'awaiting_approval'])

/** Elapsed time, ticking, for the working indicator. */
function useElapsed(since: string | null): string {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!since) return
    const t = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(t)
  }, [since])
  if (!since) return ''
  const secs = Math.max(0, Math.floor((now - new Date(since).getTime()) / 1000))
  if (secs < 60) return `${secs}s`
  const m = Math.floor(secs / 60)
  if (m < 60) return `${m}m ${secs % 60}s`
  return `${Math.floor(m / 60)}h ${m % 60}m`
}

/** One turn: the user's message, then everything the agent did about it. */
function TurnBlock({ turn, live }: { turn: Turn; live: boolean }) {
  const { data } = useQuery({
    queryKey: ['run-messages', turn.run_id],
    queryFn: () => agentApi.messages(turn.run_id, { limit: 1000 }).then((r) => r.data.messages),
    // Finished turns never change, so they are fetched once and left alone.
    refetchInterval: live ? 1500 : false,
    staleTime: live ? 0 : Infinity,
  })

  const messages: AgentMessage[] = useMemo(() => data ?? [], [data])
  const results = useMemo(() => indexResults(messages), [messages])
  const elapsed = useElapsed(live ? turn.created_at : null)

  // The last thing the agent actually did, for the working line. A live step beats a
  // spinner: one 2026 study put abandonment at 3× without it.
  const current = useMemo(() => {
    for (let i = messages.length - 1; i >= 0; i -= 1) {
      const m = messages[i]
      const c = m.content as Record<string, unknown>
      if (m.kind === 'tool_call') return `running ${String(c.name ?? 'a tool')}`
      if (m.kind === 'core_tool') return String(c.name ?? '')
      if (m.kind === 'compaction') return 'compacting context'
      if (m.kind === 'plan') return 'planning'
    }
    return 'thinking'
  }, [messages])

  return (
    <div className="space-y-3">
      {/* The user's message. */}
      <div className="flex justify-end">
        <div className="max-w-[80%] whitespace-pre-wrap rounded-2xl rounded-br-sm bg-accent/10 px-3.5 py-2 text-sm text-text">
          {turn.prompt}
        </div>
      </div>

      {/* What the agent did about it. */}
      <div className="flex gap-2.5">
        <div className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full border border-border bg-surface-2">
          <Bot className="h-3.5 w-3.5 text-text-muted" />
        </div>
        <div className="min-w-0 flex-1 space-y-1.5">
          {messages.map((m) => (
            <ActivityRow key={m.seq} msg={m} resultFor={results} />
          ))}

          {live && (
            <div className="flex items-center gap-2 px-1 py-1 text-[11px] text-text-muted">
              <Loader2 className="h-3 w-3 animate-spin text-accent" />
              <span>{current}</span>
              <span className="text-text-dim">· {elapsed}</span>
            </div>
          )}

          {!live && messages.length === 0 && (
            <div className="px-1 text-[11px] text-text-dim">no activity recorded</div>
          )}
        </div>
      </div>
    </div>
  )
}

export function ChatView({ conversationId }: { conversationId: string }) {
  const qc = useQueryClient()
  const [text, setText] = useState('')
  const [error, setError] = useState<string | null>(null)
  const bottomRef = useRef<HTMLDivElement>(null)
  const composerRef = useRef<HTMLTextAreaElement>(null)

  const { data } = useQuery({
    queryKey: ['conversation', conversationId],
    queryFn: () => conversationsApi.get(conversationId).then((r) => r.data),
    refetchInterval: 2000,
  })

  const turns = useMemo(() => data?.turns ?? [], [data])
  const running = data?.conversation.running ?? false

  const send = useMutation({
    mutationFn: (t: string) => conversationsApi.send(conversationId, t),
    onSuccess: () => {
      setText('')
      setError(null)
      qc.invalidateQueries({ queryKey: ['conversation', conversationId] })
      qc.invalidateQueries({ queryKey: ['conversations'] })
    },
    onError: (e) => {
      const d = (e as { response?: { data?: { message?: string } } }).response?.data
      setError(d?.message ?? 'the message could not be sent')
    },
  })

  const stop = useMutation({
    mutationFn: () => conversationsApi.cancel(conversationId),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['conversation', conversationId] }),
  })

  // The composer is the first thing to be ready on a new chat.
  useEffect(() => {
    composerRef.current?.focus()
  }, [conversationId])

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [turns.length, running])

  function submit() {
    const t = text.trim()
    if (!t || send.isPending) return
    send.mutate(t)
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="min-h-0 flex-1 space-y-6 overflow-y-auto px-1 py-2">
        {turns.length === 0 && (
          <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
            <Bot className="h-9 w-9 text-text-dim" />
            <div className="max-w-md">
              <p className="text-sm font-medium text-text">What should we look into?</p>
              <p className="mt-1 text-sm text-text-muted">
                Describe what you want researched. The agent picks the instruments and
                timeframes, designs the strategies, runs as many backtests as it needs
                and tells you what it found — including when the answer is “no edge
                here”.
              </p>
            </div>
          </div>
        )}

        {turns.map((t) => (
          <TurnBlock key={t.run_id} turn={t} live={ACTIVE.has(t.status)} />
        ))}
        <div ref={bottomRef} />
      </div>

      {/* Composer. Always present, always focused on a fresh chat. */}
      <div className="shrink-0 pt-2">
        {error && (
          <div className="mb-2 rounded-lg border border-line-neg bg-neg-subtle px-3 py-2 text-xs text-neg">
            {error}
          </div>
        )}
        <div className="rounded-xl border border-border bg-surface-2 p-2">
          <textarea
            ref={composerRef}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                submit()
              }
            }}
            rows={2}
            placeholder={
              running
                ? 'The agent is working — it will keep going if you leave.'
                : 'Ask the agent to research something…'
            }
            className="max-h-48 w-full resize-none bg-transparent px-2 py-1 text-sm text-text placeholder:text-text-dim focus:outline-none"
          />
          <div className="flex items-center justify-between px-1 pt-1">
            <span className="text-[11px] text-text-dim">
              {running ? 'working — you can close this tab' : 'Enter to send'}
            </span>
            <div className="flex items-center gap-2">
              {/* The kill switch stays visible whenever there is something to kill. */}
              {running && (
                <button
                  type="button"
                  onClick={() => stop.mutate()}
                  className="flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-xs text-text-muted transition-colors hover:border-line-neg hover:text-neg"
                >
                  <Square className="h-3 w-3" />
                  Stop
                </button>
              )}
              <button
                type="button"
                onClick={submit}
                disabled={!text.trim() || send.isPending || running}
                title={running ? 'Wait for the current turn, or stop it' : 'Send'}
                className="flex h-8 w-8 items-center justify-center rounded-lg bg-accent text-on-accent transition-opacity disabled:opacity-40"
              >
                {send.isPending ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <ArrowUp className="h-4 w-4" />
                )}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
