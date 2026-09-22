// What the desk announces, and what it deliberately does not (AGENT-012 §15;
// Set S task S-6.22's third clause).
//
// The acceptance names three: "live regions announce new approval, new question
// and run finished". All three are derived here, as pure functions over the card
// list the screen is already holding, so the announcement cannot disagree with
// what is on screen — an announcement computed from a second fetch is a second
// answer, and the two can differ.
//
// # Three rules this file is built around
//
// **History is not news.** Opening a conversation with nine pending approvals
// must not announce nine times. The announcements carry stable keys and the hook
// that consumes them seeds itself on first render, announcing nothing; after
// that, a key it has not seen is new.
//
// **A run that stopped answering did not finish.** "Finished" is a transition
// from *known running* to *known not running*. A conversation whose state cannot
// be read is `null`, never `false` — `running === false` on an unreadable
// conversation would announce a finish for every run the desk lost sight of,
// which is the loudest possible version of this repository's favourite bug.
//
// **A headline nobody wrote is not an empty string.** Every text falls back to a
// sentence that is true without it, rather than to `''` or `undefined`.
import type { DeskCard } from '@/api/desk'

export type AnnouncementKind = 'approval' | 'question' | 'run_finished'

export interface DeskAnnouncement {
  /** Stable for the life of the conversation: the same card announces once. */
  key: string
  kind: AnnouncementKind
  /** What the live region reads out. */
  text: string
  /** The body of the browser notification, when there is more than the title. */
  detail: string | null
}

/**
 * The cards that are a decision waiting on a person.
 *
 * Deliberately the same predicate as `pendingApprovals` in `RightPanel.tsx`,
 * and a test asserts the two agree on a fixture. They are two functions because
 * the announcement layer must not import a component module, and a divergence
 * between them would mean the desk announces something the queue does not show
 * or, worse, shows something it never announced.
 */
export function isDecisionCard(c: DeskCard): boolean {
  return c.kind === 'approval' || c.kind === 'proposal'
}

function headline(c: DeskCard, fallback: string): string {
  const h = typeof c.headline === 'string' ? c.headline.trim() : ''
  return h.length > 0 ? h : fallback
}

/**
 * Everything on this card list that is worth announcing, oldest first.
 *
 * Returns the full set every time rather than a delta; deciding what is *new*
 * needs memory of what was already said, and that memory belongs to the hook,
 * not to a function over a list.
 */
export function cardAnnouncements(cards: DeskCard[]): DeskAnnouncement[] {
  const out: DeskAnnouncement[] = []
  for (const c of cards) {
    if (isDecisionCard(c)) {
      out.push({
        key: `approval:${c.seq}`,
        kind: 'approval',
        text: `Waiting on you: ${headline(c, 'an approval with no headline')}`,
        detail: c.body ?? null,
      })
    } else if (c.kind === 'question') {
      out.push({
        key: `question:${c.seq}`,
        kind: 'question',
        text: `The agent asked: ${headline(c, 'a question with no headline')}`,
        detail: c.body ?? null,
      })
    }
  }
  return out
}

/**
 * The run-finished announcement, or `null` when nothing finished.
 *
 * `prev`/`next` are three-valued. `null` means the desk does not know whether
 * the run is running — a conversation that has not loaded, or one the platform
 * refused — and no transition involving an unknown is a finish. The only pair
 * that announces is `true → false`.
 */
export function runFinishedAnnouncement(
  prev: boolean | null,
  next: boolean | null,
  title: string | null,
): DeskAnnouncement | null {
  if (prev !== true || next !== false) return null
  const named = title && title.trim().length > 0 ? title.trim() : null
  return {
    // Not deduped against a seen-set: a conversation can run, finish, be
    // steered and finish again, and the second finish is news. The hook
    // announces this one straight from the transition.
    key: 'run_finished',
    kind: 'run_finished',
    text: named ? `Run finished: ${named}` : 'Run finished',
    detail: null,
  }
}
