// Steering, interrupt and stop (COMP-006 §3, UI-03; AGENT-001 §16).
//
// The three controls are deliberately different weights. Steering is a message and
// costs nothing; interrupt ends the current turn; stop ends the session. Stop asks
// for confirmation because it is the only one of the three that cannot be undone by
// typing again — the container goes away and the session's context with it.

import { useState } from 'react'
import { Send, Square, CircleStop } from 'lucide-react'
import { researchApi } from '@/api/research'

interface Props {
  sessionId: string
  /** Whether the session can still receive anything. */
  steerable: boolean
}

export function SteeringInput({ sessionId, steerable }: Props) {
  const [text, setText] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [confirmStop, setConfirmStop] = useState(false)

  async function send(kind: 'steer' | 'interrupt' | 'stop') {
    setBusy(true)
    setError(null)
    try {
      await researchApi.steer(sessionId, {
        kind,
        text: kind === 'steer' ? text : undefined,
      })
      if (kind === 'steer') setText('')
      setConfirmStop(false)
    } catch (e) {
      const detail =
        (e as { response?: { data?: { message?: string; fix?: string } } }).response?.data
      // Show the platform's own `fix` when it sent one. A refusal a user cannot act
      // on just gets retried.
      setError(detail?.fix ?? detail?.message ?? 'the message could not be queued')
    } finally {
      setBusy(false)
    }
  }

  if (!steerable) {
    return (
      <div className="rounded-lg border border-border bg-surface-2 px-3 py-2 text-xs text-text-dim">
        This session has ended. Its transcript and report stay readable; start a new
        session to continue the work.
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-2">
      {error && (
        <div className="rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-xs text-warning">
          {error}
        </div>
      )}
      {/* The input gets its own row. Side by side, three buttons and a textarea
          cannot both fit a narrow centre column, and flexbox resolves that by
          crushing the textarea to a few pixels rather than wrapping. */}
      <textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey) && text.trim()) {
            void send('steer')
          }
        }}
        rows={2}
        placeholder="Steer the session…  (Cmd/Ctrl+Enter to send)"
        className="w-full resize-y rounded-lg border border-border bg-surface px-3 py-2 text-sm text-text placeholder:text-text-dim focus:border-accent focus:outline-none"
      />
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          disabled={busy || !text.trim()}
          onClick={() => void send('steer')}
          className="flex h-8 items-center gap-1.5 rounded-lg bg-accent px-3 text-xs font-medium text-background hover:bg-accent-hover disabled:opacity-40"
        >
          <Send className="h-3.5 w-3.5" />
          Send
        </button>
        <button
          type="button"
          disabled={busy}
          title="End the current turn without saying anything"
          onClick={() => void send('interrupt')}
          className="flex h-8 items-center gap-1.5 rounded-lg border border-border px-3 text-xs text-text-muted hover:text-text disabled:opacity-40"
        >
          <Square className="h-3.5 w-3.5" />
          Interrupt
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => (confirmStop ? void send('stop') : setConfirmStop(true))}
          onBlur={() => setConfirmStop(false)}
          className={`flex h-8 items-center gap-1.5 rounded-lg border px-3 text-xs disabled:opacity-40 ${
            confirmStop
              ? 'border-pnl-down bg-pnl-down/10 text-pnl-down'
              : 'border-border text-text-muted hover:text-text'
          }`}
        >
          <CircleStop className="h-3.5 w-3.5" />
          {confirmStop ? 'Stop for good?' : 'Stop'}
        </button>
      </div>
    </div>
  )
}
