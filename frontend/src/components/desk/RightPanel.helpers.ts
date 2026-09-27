import type { DeskCard, Met, WorkingState } from '@/api/desk'
import type { StateAbsence } from './RightPanel'

/**
 * How one acceptance criterion reads. **Three states, and `null` is a state.**
 *
 * `met` arrives as `true | false | null` (ADR-D-25; `enum Met` in
 * `crates/harness/src/state.rs`), and `null` means nobody has evaluated it.
 * Nothing on this platform evaluates acceptance criteria yet, so **every
 * criterion on every run is `null` today** — which makes the two-state rendering
 * this panel would otherwise fall into a report that every run is failing
 * everything it was asked to do.
 *
 * So `null` gets neither mark. Not a cross, which is a measurement that came
 * back negative; not a tick, which is the opposite one; not an empty checkbox
 * beside ticked ones, which reads as a cross to anyone scanning the list. It
 * gets the words *not yet evaluated* and a neutral tone, and those words are the
 * same ones the harness card prints (`Met::mark`), so the desk and the transcript
 * say the same thing about the same criterion.
 *
 * Exported so the claim can be tested without a DOM, and so no second caller
 * restates the rule and gets it wrong — that is how the "hosted means measured"
 * badge survived three fixes on this codebase.
 */
export function acceptanceMark(met: Met): {
  key: 'met' | 'not-met' | 'unevaluated'
  word: string
  glyph: string | null
  tone: 'pos' | 'neg' | 'neutral'
} {
  if (met === null) return { key: 'unevaluated', word: 'not yet evaluated', glyph: null, tone: 'neutral' }
  return met
    ? { key: 'met', word: 'met', glyph: '✓', tone: 'pos' }
    : { key: 'not-met', word: 'NOT met', glyph: '✕', tone: 'neg' }
}

/** What the run is waiting on, in the words a person would use. */
export function waitingOn(state: WorkingState, opts: { jobPct?: number | null; pendingApproval?: boolean; paused?: boolean }): string {
  if (opts.paused) return 'paused — waiting on you to resume'
  if (opts.pendingApproval) return 'waiting on you to approve or deny'
  if (opts.jobPct !== null && opts.jobPct !== undefined) return `waiting on a job · ${Math.round(opts.jobPct)}%`
  const active = state.plan.find((s) => s.status.status === 'active')
  if (active) return `running step: ${active.goal}`
  const next = state.plan.find((s) => s.status.status === 'pending')
  if (next) return `about to run: ${next.goal}`
  if (state.last_error) return `recovering from: ${state.last_error.code}`
  return 'waiting on the model'
}

/** The sentence to print when there is no snapshot, given what we know about why. */
export function absenceCopy(absence?: StateAbsence): { message: string; detail: string } {
  if (absence === 'not_served') {
    return {
      message: 'Nothing on this platform reports a working state',
      detail:
        'The State tab renders the harness’s own state snapshot, and the route that would serve it — /api/agent/runs/{id}/state — is not mounted. So this is not a snapshot that has yet to arrive: no step will produce one, and the panel will not rebuild it by replaying the cards beside it, because a state reconstructed from the stream is a second answer that can disagree with the agent’s.',
    }
  }
  if (absence === 'no_run') {
    return {
      message: 'This conversation has no run',
      detail: 'The working state belongs to a run. Send a turn and this fills in — if the state route is serving.',
    }
  }
  return {
    message: 'No working state yet',
    detail: 'The State tab renders the harness’s latest state snapshot. Nothing here says why there is none.',
  }
}

/** Cards that carry a pending decision, used to light `a`/`x` in the controls row. */
export function pendingApprovals(cards: DeskCard[]): DeskCard[] {
  return cards.filter((c) => c.kind === 'approval' || c.kind === 'proposal')
}
