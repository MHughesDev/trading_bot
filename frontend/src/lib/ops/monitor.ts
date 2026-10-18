// What the monitor's push channel and measurement routes mean, as pure
// functions (COMP-007 §3, §6; Set S S-7.10).
//
// The rules this file keeps, because each has already been the repository's bug
// somewhere else:
//
//   * **A stream that said nothing is not a platform with nothing wrong.** The
//     page prints the stream's own state next to the events, and an age is only
//     ever an age of something that arrived.
//   * **Unknown is not fresh.** A feed verdict the page does not recognise is
//     rendered as itself, never folded into `fresh`.
//   * **A drill is not an alarm.** It is announced, marked DRILL, and never
//     posts an operating-system notification: nobody is to be woken for a
//     rehearsal, and nobody is to read a rehearsal as the real thing.
import type { FeedVerdict, MonitorEventRow } from '@/api/ops'

// ── Frames ───────────────────────────────────────────────────────────────────

export interface KillSwitchSnapshot {
  lease: string
  orders_permitted: boolean
  halted_scopes: string[]
  /** Present when the server could not read the switch: unknown, not armed. */
  error: string | null
}

export type MonitorFrameEvent = MonitorEventRow & { reason: string | null }

export type MonitorFrame =
  | { kind: 'kill_switch'; snapshot: KillSwitchSnapshot }
  | { kind: 'monitor_event'; event: MonitorFrameEvent }
  | { kind: 'stream_error'; message: string }
  | { kind: 'unknown' }

/**
 * Tells the three frames of `GET /api/monitor/stream` apart by shape.
 *
 * `useDeskStream` hands over the parsed JSON and not the SSE `event:` name, so
 * the discriminator is the field only that frame carries. A frame that matches
 * none is `unknown` — dropped by the caller, not guessed into a snapshot.
 */
export function parseMonitorFrame(raw: unknown): MonitorFrame {
  if (!raw || typeof raw !== 'object') return { kind: 'unknown' }
  const o = raw as Record<string, unknown>
  if (typeof o.event_id === 'string' && typeof o.kind === 'string' && typeof o.scope === 'string') {
    return {
      kind: 'monitor_event',
      event: {
        event_id: o.event_id,
        occurred_at: String(o.occurred_at ?? ''),
        scope: o.scope,
        family: String(o.family ?? ''),
        kind: o.kind,
        metric: typeof o.metric === 'string' ? o.metric : null,
        observed_value: typeof o.observed_value === 'number' ? o.observed_value : null,
        limit_value: typeof o.limit_value === 'number' ? o.limit_value : null,
        reason: typeof o.reason === 'string' ? o.reason : null,
        actor: String(o.actor ?? ''),
        is_drill: o.is_drill === true,
      },
    }
  }
  if (typeof o.lease === 'string') {
    return {
      kind: 'kill_switch',
      snapshot: {
        lease: o.lease,
        orders_permitted: o.orders_permitted === true,
        halted_scopes: Array.isArray(o.halted_scopes)
          ? o.halted_scopes.filter((s): s is string => typeof s === 'string')
          : [],
        error: typeof o.error === 'string' ? o.error : null,
      },
    }
  }
  if (typeof o.message === 'string') return { kind: 'stream_error', message: o.message }
  return { kind: 'unknown' }
}

/** Newest first, one row per `event_id`, capped. A replayed frame is not a second event. */
export function mergeEvents<E extends { event_id: string; occurred_at: string }>(
  prev: E[],
  incoming: E,
  cap = 100,
): E[] {
  if (prev.some((e) => e.event_id === incoming.event_id)) return prev
  return [incoming, ...prev]
    .sort((a, b) => (a.occurred_at < b.occurred_at ? 1 : a.occurred_at > b.occurred_at ? -1 : 0))
    .slice(0, cap)
}

// ── Event wording ────────────────────────────────────────────────────────────

export type EventSeverity = 'critical' | 'warning' | 'info'

const CRITICAL = new Set(['tripped', 'halted'])
const WARNING = new Set(['breach', 'scaled_down', 'warning', 'window_opened'])

export function eventSeverity(kind: string): EventSeverity {
  if (CRITICAL.has(kind)) return 'critical'
  if (WARNING.has(kind)) return 'warning'
  return 'info'
}

/** The kinds a person is told about unprompted. Blips and releases are noise. */
const ANNOUNCED = new Set([
  'tripped',
  'halted',
  'breach',
  'scaled_down',
  'reset',
  'resumed',
  'window_opened',
])

const KIND_WORDS: Record<string, string> = {
  tripped: 'kill switch tripped',
  halted: 'halted',
  breach: 'control-limit breach',
  scaled_down: 'scaled down',
  reset: 'kill switch reset',
  resumed: 'resumed',
  window_opened: 'maintenance window opened',
  window_closed: 'maintenance window closed',
  scale_released: 'scale released',
  warning: 'warning',
  blip: 'blip',
  drill: 'drill',
}

export function kindWords(kind: string): string {
  return KIND_WORDS[kind] ?? kind
}

export interface MonitorAnnouncement {
  /** Stable: the same event announces once. */
  key: string
  text: string
  detail: string | null
  /** Whether it may post an OS notification, beyond the live region. */
  notify: boolean
  severity: EventSeverity
}

/** `0.42 against a limit of 0.30`, or `null` when neither number was recorded. */
export function valueVsLimit(e: { observed_value?: number | null; limit_value?: number | null }): string | null {
  const v = e.observed_value ?? null
  const l = e.limit_value ?? null
  if (v === null && l === null) return null
  if (v === null) return `limit ${l}`
  if (l === null) return `value ${v}`
  return `${v} against a limit of ${l}`
}

/**
 * What one monitor event announces, or `null` for the ones that stay on the page.
 *
 * A drill always announces — marked DRILL, so the desk banner's own marking and
 * this agree — and never notifies.
 */
export function eventAnnouncement(e: MonitorFrameEvent): MonitorAnnouncement | null {
  const words = kindWords(e.kind)
  const where = e.scope === 'global' ? 'platform-wide' : e.scope
  const why = [e.metric ? `metric ${e.metric}` : null, valueVsLimit(e), e.reason].filter(Boolean).join(' · ')
  if (e.is_drill) {
    return {
      key: e.event_id,
      text: `DRILL: ${words} (${where}). Nothing real was refused.`,
      detail: why || null,
      notify: false,
      severity: 'info',
    }
  }
  if (!ANNOUNCED.has(e.kind)) return null
  return {
    key: e.event_id,
    text: `${words.charAt(0).toUpperCase()}${words.slice(1)}: ${where}`,
    detail: why || null,
    notify: eventSeverity(e.kind) !== 'info',
    severity: eventSeverity(e.kind),
  }
}

// ── Ages and verdicts ────────────────────────────────────────────────────────

/** `null` in, `not recorded` out: an age nobody measured is not "0 s". */
export function ageWords(secs: number | null | undefined): string {
  if (secs === null || secs === undefined || !Number.isFinite(secs)) return 'not recorded'
  const s = Math.max(0, Math.round(secs))
  if (s < 90) return `${s} s`
  if (s < 90 * 60) return `${Math.round(s / 60)} min`
  if (s < 48 * 3600) return `${Math.round(s / 3600)} h`
  return `${Math.round(s / 86400)} days`
}

export interface VerdictView {
  word: string
  tone: 'pos' | 'warn' | 'neg' | 'neutral'
  /** One sentence on what it means to a person looking at it. */
  meaning: string
}

export function feedVerdict(v: FeedVerdict | string): VerdictView {
  switch (v) {
    case 'fresh':
      return { word: 'fresh', tone: 'pos', meaning: 'The newest bar is within its cadence.' }
    case 'warning':
      return {
        word: 'late',
        tone: 'warn',
        meaning: 'The newest bar is later than its cadence allows; one more miss halts.',
      }
    case 'hard_stale':
      return {
        word: 'stale',
        tone: 'neg',
        meaning: 'Past the hard limit: strategies reading this feed are halted.',
      }
    case 'unmeasurable':
      return {
        word: 'unmeasurable',
        tone: 'warn',
        meaning: 'The gauge cannot judge this feed; that is not the same as fresh.',
      }
    case 'session_closed':
      return { word: 'session closed', tone: 'neutral', meaning: 'The market is closed; no bar is expected.' }
    case 'producer_silent':
      return {
        word: 'producer silent',
        tone: 'neg',
        meaning: 'The collector stopped affirming this feed; its age is a lower bound.',
      }
    default:
      // A verdict this build has no word for is shown as itself.
      return { word: String(v), tone: 'warn', meaning: 'A verdict this page has no description for.' }
  }
}

export function venueVerdict(v: string): VerdictView {
  const s = v.toLowerCase()
  if (s === 'reconciled' || s === 'clean' || s === 'match' || s === 'ok') {
    return { word: v, tone: 'pos', meaning: 'The venue agrees with the book.' }
  }
  if (s.includes('mismatch') || s.includes('diverg') || s.includes('fail')) {
    return { word: v, tone: 'neg', meaning: 'The venue disagrees with the book; a resume is held.' }
  }
  return { word: v, tone: 'warn', meaning: 'Not a verdict this page recognises as clean.' }
}

/** Timeframe seconds as a bar label. */
export function timeframeWords(secs: number): string {
  if (secs % 86400 === 0) return `${secs / 86400}d`
  if (secs % 3600 === 0) return `${secs / 3600}h`
  if (secs % 60 === 0) return `${secs / 60}m`
  return `${secs}s`
}

/**
 * The stream's state in the page's own words. `stale` with no event ever
 * received has no age; the sentence says so and does not invent one.
 */
export function streamWords(
  state: 'connecting' | 'live' | 'stale' | 'closed',
  everReceived: boolean,
  staleForSecs: number | null,
  error: string | null,
): string {
  switch (state) {
    case 'live':
      return everReceived ? 'live' : 'connected, waiting for the first frame'
    case 'connecting':
      return 'connecting'
    case 'stale':
      return everReceived && staleForSecs !== null
        ? `dropped ${ageWords(staleForSecs)} ago; showing the last state`
        : 'has not connected yet; nothing is known from it'
    case 'closed':
      return error ? `closed: ${error}` : 'closed'
  }
}

// ── The shared live state ────────────────────────────────────────────────────
//
// One subscriber, many readers. `MonitorNotifier` (mounted once, in the shell)
// owns the SSE connection and writes what it hears into react-query's cache
// under this key; the Monitor page reads it. Two components each opening
// `/api/monitor/stream` would be two server-side `LISTEN`s and, worse, two
// opinions about whether the stream was live.

export const MONITOR_LIVE_KEY = ['monitor', 'live'] as const

export interface MonitorLiveState {
  stream: {
    state: 'connecting' | 'live' | 'stale' | 'closed'
    everReceived: boolean
    staleForSecs: number | null
    error: string | null
  }
  /** The last kill-switch snapshot the stream sent; `null` until one arrives. */
  snapshot: KillSwitchSnapshot | null
  /** Newest first. Only what arrived on this connection: not a history. */
  events: MonitorFrameEvent[]
  /** A server-reported failure of the listener, shown, never swallowed. */
  streamError: string | null
}

export function emptyLiveState(): MonitorLiveState {
  return {
    stream: { state: 'connecting', everReceived: false, staleForSecs: null, error: null },
    snapshot: null,
    events: [],
    streamError: null,
  }
}

/** What one frame does to the shared state. Pure, so the order of frames is testable. */
export function applyFrame(prev: MonitorLiveState, frame: MonitorFrame): MonitorLiveState {
  switch (frame.kind) {
    case 'kill_switch':
      return { ...prev, snapshot: frame.snapshot, streamError: null }
    case 'monitor_event':
      return { ...prev, events: mergeEvents(prev.events, frame.event) }
    case 'stream_error':
      return { ...prev, streamError: frame.message }
    case 'unknown':
      return prev
  }
}
