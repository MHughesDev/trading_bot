// Browser notifications, opt-in, with quiet hours (AGENT-012 §13, §15; Set S
// task S-6.22).
//
// # What this can and cannot do, stated once so no surface has to guess
//
// This is the **Notification API**, not push. It posts an OS-level toast from a
// page that is already open. There is no service worker in this app and no push
// subscription anywhere on the platform, so a closed tab receives nothing —
// which is the one thing people assume "browser notifications" means. Every
// surface that offers the opt-in has to say so, or it is promising delivery it
// cannot make.
//
// # Why the decision is a value and not a branch
//
// Four separate things can stop a notification — the user has not opted in, the
// browser refuses, the page is in front of them already, or it is the middle of
// their night — and every one of them is a different sentence. A boolean would
// collapse them into "nothing happened", which is exactly the silence a person
// reads as "the feature is broken". `notifyDecision` returns the reason so the
// settings panel's test button can say which of the four it was.
//
// # The rule this file exists to keep
//
// An unreadable quiet-hours window is **not** midnight. `parseInt('')` is `NaN`
// and `Number('')` is `0`, and a `0` here means "quiet hours start at 00:00",
// which silences nothing and reads as a working setting. `minutesOfDay` returns
// `null` on anything that is not `HH:MM`, and `notifyDecision` refuses to
// deliver rather than guessing a window — with its own reason code, so the
// settings panel can show the field as broken instead of the user discovering
// it by never being told anything.

/** Whether this browser can post a notification at all, and whether it will. */
export type NotifyState =
  /** No `Notification` constructor — an old browser, or a non-browser runtime. */
  | 'unsupported'
  /** The API exists but the page is not a secure context, where it never works. */
  | 'insecure'
  /** Supported and permitted — nothing to ask. */
  | 'granted'
  /** The user (or a policy) refused. **Nothing in this app can re-ask.** */
  | 'denied'
  /** Supported, never asked. This is the only state where a prompt is possible. */
  | 'default'

/** The slice of `window` this module reads, so a test can supply its own. */
export interface NotifyEnv {
  Notification?: {
    permission: string
    requestPermission?: () => Promise<string>
    new (title: string, options?: { body?: string; tag?: string }): unknown
  }
  isSecureContext?: boolean
}

function env(): NotifyEnv {
  return typeof window === 'undefined' ? {} : (window as unknown as NotifyEnv)
}

/**
 * What the browser will do with a notification request right now.
 *
 * The order of the checks is the order of the sentences a person needs. A
 * browser with no `Notification` is `unsupported` whatever its context, and the
 * secure-context test comes second because a page served over plain HTTP from a
 * LAN address — how this platform is reached from a second machine — exposes
 * the constructor and then refuses every request, which would otherwise render
 * as `default` and offer a button that can only fail.
 */
export function notifyState(e: NotifyEnv = env()): NotifyState {
  const N = e.Notification
  if (!N || typeof N.permission !== 'string') return 'unsupported'
  if (e.isSecureContext === false) return 'insecure'
  if (N.permission === 'granted') return 'granted'
  if (N.permission === 'denied') return 'denied'
  return 'default'
}

/**
 * Ask the browser for permission, once.
 *
 * Returns the state afterwards. Callable only from `default`: a `denied` is a
 * browser-level setting and calling `requestPermission` again resolves
 * immediately with `denied` without showing the user anything, so a control
 * that offered it would be a button that reports its own failure as a click
 * that did nothing.
 */
export async function requestNotifyPermission(e: NotifyEnv = env()): Promise<NotifyState> {
  const before = notifyState(e)
  if (before !== 'default') return before
  const req = e.Notification?.requestPermission
  if (!req) return 'unsupported'
  try {
    await req.call(e.Notification)
  } catch {
    // Some browsers reject rather than resolving `denied`. Either way the
    // answer is read back off `permission` below, never assumed from the call.
  }
  return notifyState(e)
}

/**
 * Minutes past midnight for a `HH:MM` string, or `null` when it is not one.
 *
 * **Never a number on bad input.** The whole point of this function is that a
 * malformed window does not become `00:00`.
 */
export function minutesOfDay(hhmm: unknown): number | null {
  if (typeof hhmm !== 'string') return null
  const m = /^([01]\d|2[0-3]):([0-5]\d)$/.exec(hhmm.trim())
  if (!m) return null
  return Number(m[1]) * 60 + Number(m[2])
}

/** Minutes past midnight for a `Date`, read off the device's own clock. */
export function minutesOfDayFrom(d: Date): number {
  return d.getHours() * 60 + d.getMinutes()
}

/**
 * Whether `now` falls inside `[start, end)`, including a window that wraps
 * midnight — which is the only shape anyone actually sets.
 *
 * `start === end` is **not** a 24-hour window. A zero-length window silences
 * nothing; reading it as "always quiet" would turn a mis-set field into total
 * silence, and the settings panel says so beside the fields.
 */
export function inQuietHours(now: number, start: number, end: number): boolean {
  if (start === end) return false
  if (start < end) return now >= start && now < end
  return now >= start || now < end
}

export type NotifyRefusal =
  /** The user has not turned browser notifications on. */
  | 'opted_out'
  /** The browser cannot or will not post one — see `notifyState`. */
  | 'not_permitted'
  /** The page is in front of the person; the live region has already said it. */
  | 'page_visible'
  /** Inside the quiet window. */
  | 'quiet_hours'
  /** Quiet hours are on and their window is not readable. */
  | 'quiet_hours_unreadable'

export type NotifyDecision = { deliver: true } | { deliver: false; why: NotifyRefusal }

export interface NotifyOptions {
  /** `prefs.browserNotifications`. */
  enabled: boolean
  state: NotifyState
  quietHours: { enabled: boolean; start: string; end: string }
  /** The device's clock, injected so the quiet-hours tests are not time-of-day dependent. */
  now: Date
  /**
   * `'visible' | 'hidden'`, or `null` where there is no document to ask.
   *
   * A test notification passes `null`: it is a deliberate request to see one,
   * and refusing it because the settings page is on screen would make the only
   * control that proves the opt-in works the one control it silences.
   */
  visibility: 'visible' | 'hidden' | null
}

/**
 * Whether to post a notification now, and if not, which of the four reasons.
 *
 * Order matters and is the order of increasing surprise: an opt-out is the
 * user's own choice, a refusal is the browser's, visibility is the situation,
 * and quiet hours is the one the user will have forgotten they set.
 */
export function notifyDecision(o: NotifyOptions): NotifyDecision {
  if (!o.enabled) return { deliver: false, why: 'opted_out' }
  if (o.state !== 'granted') return { deliver: false, why: 'not_permitted' }
  if (o.visibility === 'visible') return { deliver: false, why: 'page_visible' }
  if (o.quietHours.enabled) {
    const start = minutesOfDay(o.quietHours.start)
    const end = minutesOfDay(o.quietHours.end)
    // Not a guess, and not a silent pass. A person who asked for quiet hours
    // and whose window cannot be read is not told "00:00 to 00:00"; they are
    // told the field is broken, on the panel that owns it.
    if (start === null || end === null) return { deliver: false, why: 'quiet_hours_unreadable' }
    if (inQuietHours(minutesOfDayFrom(o.now), start, end)) {
      return { deliver: false, why: 'quiet_hours' }
    }
  }
  return { deliver: true }
}

/** What each refusal means, in the words the settings panel prints. */
export const REFUSAL_TEXT: Record<NotifyRefusal, string> = {
  opted_out: 'Browser notifications are off, so nothing was posted.',
  not_permitted: 'This browser will not post a notification — see the state above.',
  page_visible: 'This page is in front of you, so the announcement went to the live region instead.',
  quiet_hours: 'It is inside your quiet hours, so nothing was posted.',
  quiet_hours_unreadable:
    'Quiet hours are on and the window is not a readable HH:MM, so nothing was posted. ' +
    'An unreadable window is not midnight, and it is not ignored either — fix the fields below.',
}

/**
 * Post the notification if the decision allows it, and report what happened.
 *
 * The constructor is wrapped because some browsers throw from it on a page that
 * has no permission despite reporting `granted` (an embedded frame, most often),
 * and an exception escaping here would take down the effect that called it and
 * with it the **live region announcement**, which is the half that has to work.
 */
export function deliverNotification(
  title: string,
  body: string | null,
  o: NotifyOptions,
  e: NotifyEnv = env(),
): NotifyDecision {
  const d = notifyDecision(o)
  if (!d.deliver) return d
  const N = e.Notification
  if (!N) return { deliver: false, why: 'not_permitted' }
  try {
    new N(title, { body: body ?? undefined })
  } catch {
    return { deliver: false, why: 'not_permitted' }
  }
  return d
}

/** `document.visibilityState`, or `null` where there is no document. */
export function pageVisibility(): 'visible' | 'hidden' | null {
  if (typeof document === 'undefined') return null
  return document.visibilityState === 'hidden' ? 'hidden' : 'visible'
}
