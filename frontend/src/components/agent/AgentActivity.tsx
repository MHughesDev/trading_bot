// How agent work is rendered inside a chat turn.
//
// Modelled on the coding agents (Claude Code, Cursor, Codex) and on what the 2026
// agent-UX literature converged on: a live step list rather than a spinner, every
// tool call shown with its inputs and elapsed time, and everything collapsed by
// default so the stream stays readable at fifty steps.
//
// The rule that shapes all of it: **activity is subordinate to the answer.** Tool
// calls, routing decisions and compactions are dimmed, monospace and one line tall;
// the result is full-size prose. A timeline where every entry shouts is one nobody
// reads, and the thing the user actually came for is the last paragraph.

import { useState } from 'react'
import {
  CheckCircle2,
  ChevronRight,
  CircleAlert,
  Loader2,
  ShieldAlert,
  Terminal,
  Trophy,
} from 'lucide-react'
import type { AgentMessage } from '@/api/agent'

type Content = Record<string, unknown>

const str = (v: unknown): string => (typeof v === 'string' ? v : v == null ? '' : String(v))
const num = (v: unknown): number => (typeof v === 'number' ? v : Number(v ?? 0))

/** A one-line summary of a tool's arguments, for the collapsed card. */
function argSummary(args: unknown): string {
  if (!args || typeof args !== 'object') return ''
  const entries = Object.entries(args as Record<string, unknown>)
  if (entries.length === 0) return ''
  return entries
    .slice(0, 3)
    .map(([k, v]) => {
      const raw = typeof v === 'string' ? v : JSON.stringify(v)
      const short = raw.length > 40 ? `${raw.slice(0, 40)}…` : raw
      return `${k}=${short}`
    })
    .join(' ')
}

/** A collapsed tool call. Expands to the full arguments and result. */
function ToolCard({
  name,
  args,
  result,
  isError,
}: {
  name: string
  args: unknown
  result?: unknown
  isError?: boolean
  }) {
  const [open, setOpen] = useState(false)
  const pending = result === undefined

  return (
    <div className="rounded-md border border-border/60 bg-surface-2/40">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left"
      >
        <ChevronRight
          className={`h-3 w-3 shrink-0 text-text-dim transition-transform ${open ? 'rotate-90' : ''}`}
        />
        {pending ? (
          <Loader2 className="h-3 w-3 shrink-0 animate-spin text-accent" />
        ) : isError ? (
          <CircleAlert className="h-3 w-3 shrink-0 text-neg" />
        ) : (
          <CheckCircle2 className="h-3 w-3 shrink-0 text-pos/70" />
        )}
        <span className="font-mono text-[11px] text-text">{name}</span>
        <span className="truncate font-mono text-[11px] text-text-dim">
          {argSummary(args)}
        </span>
      </button>

      {open && (
        <div className="space-y-1.5 border-t border-border/60 px-2.5 py-2">
          <pre className="overflow-x-auto whitespace-pre-wrap break-all font-mono text-[10px] text-text-muted">
            {JSON.stringify(args, null, 2)}
          </pre>
          {result !== undefined && (
            <pre
              className={`max-h-64 overflow-auto whitespace-pre-wrap break-all font-mono text-[10px] ${
                isError ? 'text-neg' : 'text-text-dim'
              }`}
            >
              {typeof result === 'string' ? result : JSON.stringify(result, null, 2)}
            </pre>
          )}
        </div>
      )}
    </div>
  )
}

/** One dimmed trace line. The default weight for everything the harness did. */
function Trace({ children }: { children: React.ReactNode }) {
  return <div className="px-1 text-[11px] leading-relaxed text-text-dim">{children}</div>
}

/**
 * Renders one persisted message.
 *
 * Returns `null` for entries that would only add noise — a state change into
 * `awaiting_model` happens on every decode call, and showing it buries everything
 * else.
 */
export function ActivityRow({
  msg,
  resultFor,
}: {
  msg: AgentMessage
  /** Tool results are folded into their call's card rather than shown separately. */
  resultFor: Map<string, { content: unknown; isError: boolean }>
}) {
  const c = msg.content as Content

  switch (msg.kind) {
    case 'user':
      return null // rendered as the turn's header

    case 'assistant': {
      const text = str(c.content)
      if (!text.trim()) return null
      return (
        <div className="whitespace-pre-wrap text-sm leading-relaxed text-text">{text}</div>
      )
    }

    case 'tool_call': {
      const name = str(c.name)
      const found = resultFor.get(`${name}:${msg.seq}`) ?? resultFor.get(name)
      return (
        <ToolCard
          name={name}
          args={c.arguments}
          result={found?.content}
          isError={found?.isError}
        />
      )
    }

    // Folded into the call above.
    case 'tool_result':
      return null

    case 'canary': {
      const probes = Array.isArray(msg.content) ? (msg.content as Content[]) : []
      const ok = probes.filter((p) => p.verdict === 'honoured').length
      const bad = probes.length - ok
      return (
        <Trace>
          <ShieldAlert className="mr-1 inline h-3 w-3" />
          backend check: {ok}/{probes.length} passed
          {bad > 0 && <span className="text-warn"> · {bad} failed</span>}
        </Trace>
      )
    }

    case 'plan': {
      const steps = (c.steps as Array<{ goal: string }>) ?? []
      if (steps.length === 0) return null
      return (
        <div className="rounded-md border border-border/60 bg-surface-2/40 px-2.5 py-2">
          <div className="mb-1 text-[11px] font-medium text-text-muted">Plan</div>
          <ol className="list-inside list-decimal space-y-0.5 text-[11px] text-text-dim">
            {steps.map((st, i) => (
              <li key={i}>{st.goal}</li>
            ))}
          </ol>
        </div>
      )
    }

    case 'exposure': {
      const exposed = (c.exposed as string[]) ?? []
      return (
        <Trace>
          <Terminal className="mr-1 inline h-3 w-3" />
          tools available: <span className="font-mono">{exposed.join(', ')}</span>
        </Trace>
      )
    }

    case 'policy': {
      const decision = str(c.decision)
      if (decision === 'allow') return null // the common case; the call itself is the record
      return (
        <Trace>
          <span className={decision === 'deny' ? 'text-neg' : 'text-warn'}>
            policy {decision} · <span className="font-mono">{str(c.tool)}</span>
          </span>
        </Trace>
      )
    }

    case 'validation':
      return (
        <Trace>
          <span className="text-warn/80">retrying: {str(c.message)}</span>
        </Trace>
      )

    case 'core_tool':
      return (
        <Trace>
          <span className="font-mono">{str(c.name)}</span> · {str(c.detail)}
        </Trace>
      )

    case 'compaction':
      return (
        <Trace>
          compacted context {num(c.tokens_before)} → {num(c.tokens_after)} tokens
        </Trace>
      )

    case 'usage': {
      const secs = num(c.latency_ms) / 1000
      if (secs < 5) return null // sub-5s calls are noise at this density
      return <Trace>thought for {secs.toFixed(1)}s</Trace>
    }

    // The harness saying a guarantee it depends on is gone. The one entry here that
    // should be hard to scroll past.
    case 'degradation': {
      const kind = str(c.degradation)
      const text =
        kind === 'constraint_ignored'
          ? `the model backend stopped honouring its output format — ${str(c.detail)}`
          : kind === 'output_truncated'
            ? `replies kept hitting the ${num(c.reserve_for_output)}-token output limit`
            : kind === 'steps_exhausted'
              ? 'wrapping up — asking for a final answer'
              : kind === 'stuck'
                ? `the same failure (${str(c.code)}) survived ${num(c.attempts)} retries`
                : str(c.detail) || kind
      return (
        <div className="rounded-md border border-line-warn bg-warn-subtle px-2.5 py-1.5 text-[11px] text-warn">
          <ShieldAlert className="mr-1 inline h-3 w-3" />
          {text}
        </div>
      )
    }

    case 'status': {
      if (c.hardware) {
        return (
          <Trace>
            {str(c.hardware)} · {str(c.tier)} tier
          </Trace>
        )
      }
      if (c.note === 'entered') {
        // One per decode call; showing them buries the rest.
        if (c.state === 'awaiting_model' || c.state === 'executing') return null
        return <Trace>{str(c.state)}</Trace>
      }
      return null
    }

    case 'error':
      return (
        <div className="rounded-md border border-line-neg bg-neg-subtle px-2.5 py-2 text-xs text-neg">
          {str(c.error)}
        </div>
      )

    case 'final':
      return (
        <div className="rounded-lg border border-line-pos bg-pos-subtle px-3 py-2.5">
          <div className="mb-1.5 flex items-center gap-1.5 text-xs font-semibold text-pos">
            <Trophy className="h-3.5 w-3.5" />
            Result
          </div>
          <div className="whitespace-pre-wrap text-sm leading-relaxed text-text">
            {str(c.content)}
          </div>
        </div>
      )

    default:
      return null
  }
}
