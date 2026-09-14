// The session timeline (COMP-006 §3, UI-02).
//
// Streams over SSE from `/api/agent/sessions/{id}/events`. Three things about this
// component are deliberate:
//
// 1. **Tool cards are collapsed by default.** A research session makes hundreds of
//    tool calls, and a timeline that renders every payload in full is one a human
//    scrolls past rather than reads. The card shows what was called and what came
//    back in one line; the full output is one click away.
// 2. **Events are keyed and deduplicated by their durable row id.** EventSource
//    replays from `Last-Event-ID` on reconnect, and a reconnect that re-delivers the
//    last event is normal, not an error.
// 3. **A compaction is a visible marker.** The conversation genuinely changed shape
//    at that point, and a reader trying to work out why the agent "forgot" something
//    needs to see where it happened.

import { useEffect, useMemo, useRef, useState } from 'react'
import { ChevronRight, Scissors, Flag, AlertTriangle } from 'lucide-react'
import { openTimeline, type TimelineEvent } from '@/api/research'

interface Props {
  sessionId: string
}

/** Event kinds that are structural markers rather than content. */
const MARKER_KINDS = new Set(['compaction', 'checkpoint', 'state'])

function oneLine(payload: Record<string, unknown>, max = 160): string {
  const text =
    (payload.text as string) ??
    (payload.summary as string) ??
    (payload.message as string) ??
    JSON.stringify(payload)
  const flat = String(text).replace(/\s+/g, ' ').trim()
  return flat.length > max ? `${flat.slice(0, max)}…` : flat
}

function ToolCard({ event }: { event: TimelineEvent }) {
  const [open, setOpen] = useState(false)
  const name =
    (event.payload.name as string) ?? (event.payload.tool as string) ?? event.kind
  return (
    <div className="rounded-lg border border-border bg-surface-2">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs"
      >
        <ChevronRight
          className={`h-3.5 w-3.5 shrink-0 transition-transform ${open ? 'rotate-90' : ''}`}
        />
        <span className="font-mono font-medium text-text">{name}</span>
        <span className="min-w-0 flex-1 truncate text-text-dim">
          {oneLine(event.payload)}
        </span>
      </button>
      {open && (
        <pre className="max-h-96 overflow-auto border-t border-border px-3 py-2 text-[11px] leading-relaxed text-text-dim">
          {JSON.stringify(event.payload, null, 2)}
        </pre>
      )}
    </div>
  )
}

function Marker({ event }: { event: TimelineEvent }) {
  const Icon =
    event.kind === 'compaction' ? Scissors : event.kind === 'state' ? Flag : Flag
  const label =
    event.kind === 'compaction'
      ? 'Context compacted — the conversation was summarised here'
      : oneLine(event.payload, 120)
  return (
    <div className="flex items-center gap-2 py-1 text-[11px] uppercase tracking-wide text-text-dim">
      <Icon className="h-3 w-3" />
      <span className="h-px flex-1 bg-border" />
      <span>{label}</span>
      <span className="h-px flex-1 bg-border" />
    </div>
  )
}

function TimelineStream({ sessionId }: Props) {
  // The whole stream state is one object carrying the session it belongs to.
  // Clearing two pieces of state at the top of the effect would leave a render
  // where the old session's events sit under the new session's header, and would
  // cost a cascading render every time the selection changes.
  const [stream, setStream] = useState<{
    sessionId: string
    events: TimelineEvent[]
    error: string | null
  }>({ sessionId, events: [], error: null })
  const bottom = useRef<HTMLDivElement>(null)
  const pinned = useRef(true)

  useEffect(() => {
    const close = openTimeline(
      sessionId,
      (e) =>
        setStream((prev) =>
          // A reconnect replays the last event it delivered. Dropping the duplicate
          // is normal operation, not an error worth surfacing.
          prev.events.some((p) => p.id === e.id)
            ? prev
            : { ...prev, events: [...prev.events, e] },
        ),
      (reason) => setStream((prev) => ({ ...prev, error: reason })),
    )
    return close
  }, [sessionId])

  const { events, error } = stream

  // Follow the tail only while the reader is already at the bottom. Yanking the
  // viewport while someone is reading an earlier tool card is how a live log becomes
  // unusable.
  useEffect(() => {
    if (pinned.current) bottom.current?.scrollIntoView({ block: 'end' })
  }, [events])

  const rendered = useMemo(
    () =>
      events.map((e) => {
        if (MARKER_KINDS.has(e.kind)) return <Marker key={e.id} event={e} />
        if (e.kind === 'tool_use' || e.kind === 'tool_result')
          return <ToolCard key={e.id} event={e} />
        if (e.kind === 'stream_error')
          return (
            <div
              key={e.id}
              className="flex items-start gap-2 rounded-lg border border-pnl-down/40 bg-pnl-down/10 px-3 py-2 text-xs text-pnl-down"
            >
              <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span>{oneLine(e.payload)}</span>
            </div>
          )
        return (
          <div key={e.id} className="rounded-lg border border-border bg-surface px-3 py-2">
            <div className="mb-1 text-[10px] uppercase tracking-wide text-text-dim">
              {e.kind}
            </div>
            <div className="whitespace-pre-wrap text-sm text-text">
              {oneLine(e.payload, 4000)}
            </div>
          </div>
        )
      }),
    [events],
  )

  return (
    <div
      className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto pr-1"
      onScroll={(ev) => {
        const el = ev.currentTarget
        pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40
      }}
    >
      {error && (
        <div className="rounded-lg border border-warning/40 bg-warning/10 px-3 py-2 text-xs text-warning">
          {error}
        </div>
      )}
      {events.length === 0 && !error && (
        <div className="py-8 text-center text-sm text-text-dim">
          No events yet. The timeline streams live as the session runs.
        </div>
      )}
      {rendered}
      <div ref={bottom} />
    </div>
  )
}

/**
 * Remounts the stream when the session changes.
 *
 * A `key` rather than a reset inside the component: React throwing the old
 * instance away is exactly the semantics wanted here, and it is the one way to
 * clear per-session state without a render that shows the wrong session's events.
 */
export function Timeline({ sessionId }: Props) {
  return <TimelineStream key={sessionId} sessionId={sessionId} />
}
