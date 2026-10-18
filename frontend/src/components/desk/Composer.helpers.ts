import type { Workflow } from '@/api/desk'
import type { Constraints } from './Composer'

export const EMPTY_CONSTRAINTS: Constraints = { instrument: '', timeframe: '', window: '', balance: '' }

export function classifyNeedsResearch(question: string): boolean {
  return RESEARCH_SHAPED.test(question)
}

/** The workflow a question's shape defaults to (§3.3, overridable). */
export function defaultWorkflow(question: string): Workflow {
  const q = question.toLowerCase()
  if (/\bfalsif|disprove|refute/.test(q)) return 'falsify'
  if (/\breplicat|reproduce/.test(q)) return 'replicate'
  if (/\bcompare|versus|\bvs\b/.test(q)) return 'compare'
  if (/\bwhy|diagnos|regress|broke/.test(q)) return 'diagnose'
  if (/\bsurvey|which of|across all/.test(q)) return 'survey'
  if (/\boptimi[sz]e|tune|sweep/.test(q)) return 'optimise'
  return 'investigate'
}

/**
 * The slash commands that are actually answered (§3.3).
 *
 * This list used to read `['/steer', '/pause', '/pin', '/why', '/model',
 * '/budget', '/tier']`, it was exported and imported by nothing, and the only
 * `onSlash` implementation in the tree handles three of those seven. The other
 * four went through `send()`'s slash branch, which calls `onSlash` and then
 * **clears the box** — so typing `/pin` deleted what you had written and did
 * nothing else. `/pause` has now gone from the handler too, because the route
 * behind it is not mounted.
 *
 * So the list is the handled set, and `send()` checks against it. An
 * unrecognised command is refused in the composer with the text left alone,
 * which is the difference between "that is not a command" and "your question is
 * gone".
 *
 * Asserted by `a_slash_command_nobody_answers_does_not_eat_the_question`.
 *
 * **`/pause`, `/resume`, `/agenda` and `/charter` came back 2026-10-03**, each
 * with its handler: `slashAction` (`lib/desk/sessions.ts`) maps every entry
 * here to what it does, the journal's are the session routes served since
 * S-11.14, and a run's pause is the route ADR-D-27 mounted. A test asserts no
 * entry maps to nothing.
 */
export const SLASH_COMMANDS = ['/steer', '/why', '/budget', '/pause', '/resume', '/agenda', '/charter'] as const

/**
 * A crude classifier, replaced by the harness's own at S-10.2.
 *
 * It exists so the Chat → Research offer card is a real behaviour rather than a
 * placeholder: a Chat turn that would need a trial must end with an offer, not
 * with the agent quietly spending one.
 */
const RESEARCH_SHAPED =
  /\b(backtest|sweep|optimi[sz]e|walk[- ]forward|edge|alpha|does .* predict|is .* profitable|compare .* against|falsif|replicate|hypothes)/i

/**
 * What `POST /api/research/sessions` refuses today, said in the form before the
 * request rather than as a 400 after it. Each names the task whose landing
 * lifts it; the route's own refusal text says the same.
 */
export const CHARTER_REFUSED = {
  autonomous:
    'refused by the platform until S-11.9 reads the judgement threshold — notify starts items and tells you about each one',
  replenish:
    'refused by the platform until S-11.11 builds the run that refills a drained agenda; with it off the session completes when the agenda empties',
} as const
