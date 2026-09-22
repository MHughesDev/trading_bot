// The app's polite live region, as a bus (AGENT-012 §15; Set S task S-6.22).
//
// # Why a bus and not a prop
//
// The things that have to be announced — a new approval, a new question, a run
// that finished — are noticed deep inside the conversation screen, and the
// element that announces them has to be **mounted before the text changes** or
// a screen reader says nothing at all. Those two facts are incompatible with
// passing the text down: a region rendered at the moment of the announcement is
// a region that is not announced.
//
// So the region lives in the app shell, exactly as `KillSwitchBanner` does and
// for the same structural reason, and anything anywhere calls `announce`. This
// copies `useToast`'s `registerToast` rather than inventing a second pattern.
//
// # What belongs here and what does not
//
// Only events a sighted user learns from a change they did not cause. The
// composer's refusal, an inline error and the kill-switch banner already carry
// their own `role="alert"`; repeating them here would announce them twice.

type Sink = (text: string) => void

let sink: Sink | null = null

/**
 * Mounts the region. The last caller wins, because there is one shell.
 *
 * Returns nothing: `LiveRegion` re-registers on every identity change of its
 * callback rather than unregistering, so an unmount during a route transition
 * cannot leave the bus pointing at a dead setter while the next region mounts.
 */
export function registerAnnouncer(fn: Sink) {
  sink = fn
}

/**
 * Say something to screen readers.
 *
 * A no-op when no region is mounted. That is the honest behaviour for a
 * detached render in a test, and it is never the production path — `AppLayout`
 * mounts the region above `<Outlet />`.
 */
export function announce(text: string) {
  if (!text) return
  sink?.(text)
}

/** Test seam: forget the mounted region. */
export function resetAnnouncer() {
  sink = null
}
