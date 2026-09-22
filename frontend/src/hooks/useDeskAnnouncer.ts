// Turns what the conversation screen already knows into announcements
// (AGENT-012 §15; Set S task S-6.22).
//
// One hook, two outputs, and they are not the same channel:
//
//   * the **live region** always gets the line. A screen-reader user is on this
//     page; quiet hours are about being interrupted when you are not.
//   * the **browser notification** goes through `notifyDecision`, which can
//     refuse it four different ways, and only fires when the page is hidden.
//
// Collapsing those two would mean a blind user's quiet-hours setting silenced
// their screen reader, which is not what anybody sets it for.
import { useEffect, useRef } from 'react'
import { announce } from '@/lib/announce'
import { cardAnnouncements, runFinishedAnnouncement, type DeskAnnouncement } from '@/lib/desk/announce'
import { deliverNotification, notifyState, pageVisibility } from '@/lib/notify'
import { usePrefs } from '@/store/prefs'
import type { DeskCard } from '@/api/desk'

export interface DeskAnnouncerInput {
  /** Resets the seen-set: a different conversation's history is not this one's news. */
  conversationId: string
  cards: DeskCard[]
  /**
   * `true` running, `false` finished, **`null` unknown**.
   *
   * The third value is the point. See `runFinishedAnnouncement`.
   */
  running: boolean | null
  /** The conversation's title, for the run-finished line. `null` when unread. */
  title: string | null
}

/**
 * Say one thing, on both channels.
 *
 * The preferences are read with `usePrefs.getState()` **at the moment of the
 * announcement** rather than subscribed to. Subscribing would re-run the
 * announcing effect on every preference change, and the effect's first act on a
 * new conversation is to seed its seen-set — so toggling quiet hours mid-run
 * would either re-announce the whole conversation or silently swallow the next
 * card, depending on which branch the re-run took. A setting that changes what
 * is announced *next* is correct; one that changes what has already happened is
 * not.
 */
function say(a: DeskAnnouncement) {
  announce(a.text)
  const p = usePrefs.getState()
  deliverNotification(a.text, a.detail, {
    enabled: p.browserNotifications,
    state: notifyState(),
    quietHours: {
      enabled: p.quietHoursEnabled,
      start: p.quietHoursStart,
      end: p.quietHoursEnd,
    },
    now: new Date(),
    visibility: pageVisibility(),
  })
}

export function useDeskAnnouncer({ conversationId, cards, running, title }: DeskAnnouncerInput) {
  const seen = useRef<Set<string>>(new Set())
  const seededFor = useRef<string | null>(null)
  const prevRunning = useRef<boolean | null>(null)
  const runningFor = useRef<string | null>(null)

  useEffect(() => {
    const all = cardAnnouncements(cards)
    // First pass over a conversation: everything already on it is history, and
    // history is not news. Nine pending approvals on open announce nothing.
    if (seededFor.current !== conversationId) {
      seededFor.current = conversationId
      seen.current = new Set(all.map((a) => a.key))
      return
    }
    for (const a of all) {
      if (seen.current.has(a.key)) continue
      seen.current.add(a.key)
      say(a)
    }
  }, [cards, conversationId])

  useEffect(() => {
    // A conversation switch is not a state change of the old run. Reset first,
    // so moving from a working conversation to an idle one says nothing.
    if (runningFor.current !== conversationId) {
      runningFor.current = conversationId
      prevRunning.current = running
      return
    }
    const a = runFinishedAnnouncement(prevRunning.current, running, title)
    prevRunning.current = running
    if (a) say(a)
  }, [running, conversationId, title])
}
