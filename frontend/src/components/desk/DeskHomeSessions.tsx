// Continuous sessions on desk home (AGENT-012 §2 amended; AGENT-014 §9; task
// S-11.13).
//
// A continuous session is **one row** in *Running now* — its state, the item it
// is most likely to claim next, its agenda depth and its day budget — and a
// stalled one moves to *Needs you* with the diagnosis and three actions:
// resume · amend · stop. The word for the thing is "session"; there is no other.
import { useState } from 'react'
import { Link } from 'react-router-dom'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { deskApi } from '@/api/desk'
import { agendaDepth, dayBudget, nextItem, sessionRefusal, sessionState } from '@/lib/desk/sessions'
import { DayBudgetBar } from './Shared'
import type { ContinuousRow } from './useContinuousSessions'

function title(r: ContinuousRow): string {
  const seeds = r.detail?.charter?.seeds ?? []
  const objective = r.detail?.charter?.objective
  const word = typeof objective === 'string' ? objective : (objective?.kind ?? null)
  if (seeds.length > 0) return seeds[0].question
  return word ? `continuous session · ${String(word).replace(/_/g, ' ')}` : `continuous session ${r.session.session_id.slice(0, 8)}`
}

/** The next-item sentence, with how the board knows it. */
function NextItem({ r }: { r: ContinuousRow }) {
  const n = nextItem(r.agenda)
  if (n.item) {
    return (
      <p className="sub" data-testid={`session-next-${r.session.session_id}`}>
        next: {n.item.question}{' '}
        <span className="lbl-t">({n.why === 'yours' ? 'yours — claimed before anything scored' : 'highest score on the board'})</span>
      </p>
    )
  }
  return (
    <p className="sub" data-testid={`session-next-${r.session.session_id}`}>
      {n.why === 'unknown'
        ? 'next item unknown — the agenda did not answer'
        : n.why === 'unscored'
          ? 'next item not decided — nothing queued is scored yet, and the scorer picks, not the board'
          : 'nothing queued'}
    </p>
  )
}

/** One continuous session in *Running now*. */
export function SessionRunningRow({ r }: { r: ContinuousRow }) {
  const st = sessionState(r.detail ? r.detail.state : r.session.state)
  const depth = agendaDepth(r.agenda) ?? r.session.agenda_open
  return (
    <div className="desk-card" data-testid={`session-row-${r.session.session_id}`}>
      <div className="desk-card-hd">
        <Link to={`/desk/c/${r.session.conversation_id}`}>{title(r)}</Link>
        <Badge tone="info">continuous</Badge>
        <Badge tone={st.kind === 'running' ? 'accent' : st.kind === 'unknown' ? 'warn' : 'neutral'}>
          {st.kind === 'unknown' ? 'state unknown' : st.kind}
        </Badge>
        <Badge tone="outline">{r.session.tier}</Badge>
      </div>
      <NextItem r={r} />
      <p className="sub">
        {/* A depth of zero and an unknown depth are different sessions: one has
            nothing left, the other could not be read. */}
        agenda depth {depth === null || depth === undefined ? '(unknown)' : depth} ·{' '}
        {r.session.runs_started === null ? 'runs started not recorded' : `${r.session.runs_started} runs started`}
      </p>
      <DayBudgetBar usdPerDay={r.detail === undefined ? null : dayBudget(r.detail)} spentUsd={null} />
      {st.kind === 'paused' && st.why && <p className="sub">paused: {st.why}</p>}
    </div>
  )
}

/**
 * A stalled session in *Needs you* — the diagnosis, and the three things a
 * person can do about it. Amend opens the journal's Charter tab, where the
 * amendment is written; resume and stop are the session routes, and a refusal
 * from the transition trigger is printed in its own words.
 */
export function StalledSessionRow({ r, onChanged }: { r: ContinuousRow; onChanged?: () => void }) {
  const st = sessionState(r.detail ? r.detail.state : r.session.state)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  function act(action: 'resume' | 'stop') {
    setBusy(true)
    setError(null)
    deskApi
      .sessionAction(r.session.session_id, action)
      .then(() => onChanged?.())
      .catch((e: unknown) => setError(sessionRefusal(e)))
      .finally(() => setBusy(false))
  }
  return (
    <li className="desk-card" data-testid={`stalled-${r.session.session_id}`}>
      <div className="desk-card-hd">
        <Badge tone="warn">stalled</Badge>
        <Link to={`/desk/c/${r.session.conversation_id}`}>{title(r)}</Link>
      </div>
      <p data-testid={`stalled-diagnosis-${r.session.session_id}`}>
        {st.kind === 'stalled' && st.diagnosis
          ? st.diagnosis
          : // The list says stalled and the detail carried no diagnosis — said
            // rather than left blank, because "stalled" alone is the one word
            // S-11.12 exists to replace.
            'stalled, and the diagnosis could not be read — open the journal for the stall digest'}
      </p>
      <div className="desk-actions">
        <Button size="xs" disabled={busy} onClick={() => act('resume')}>
          Resume
        </Button>
        <Link to={`/desk/c/${r.session.conversation_id}?tab=charter`} className="btn ghost xs">
          Amend
        </Link>
        <Button size="xs" variant="secondary" disabled={busy} onClick={() => act('stop')}>
          Stop
        </Button>
      </div>
      {error && (
        <p className="sub" role="alert" data-testid={`stalled-error-${r.session.session_id}`}>
          {error}
        </p>
      )}
    </li>
  )
}
