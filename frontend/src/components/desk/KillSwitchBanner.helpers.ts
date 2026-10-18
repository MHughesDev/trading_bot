import type { KillSwitchState } from '@/api/desk'
import type { KillSwitchView } from './KillSwitchBanner'

/**
 * The query key every control that halts or resumes trading must invalidate.
 *
 * This replaces an exported `KILL_SWITCH_EVENT = 'desk:kill-switch'` that
 * nothing in `frontend/src` dispatched and nothing listened for — a constant
 * with the shape of a mechanism and none of one, which the next caller reaches
 * for and believes. The comment beside it claimed "a live event refetches
 * immediately, so 'within one event' holds wherever the stream is connected",
 * and that was false twice over: there was no event, and there is no stream —
 * nothing on the platform pushes kill-switch state to a browser at all.
 *
 * What is real is react-query's cache, which `Operations.tsx` was already using
 * and `DeskHome.tsx` was not. So the key is exported instead of an event name,
 * because it is the thing that actually makes the banner move.
 */
export const KILL_SWITCH_QUERY_KEY = ['desk', 'kill-switch'] as const

export function killSwitchTripped(s: KillSwitchState | null | undefined): boolean {
  if (!s) return false
  return s.kill_switch_active === true || s.trading_enabled === false
}

export function killSwitchView(q: {
  data: KillSwitchState | null | undefined
  isPending: boolean
}): KillSwitchView {
  if (q.data !== null && q.data !== undefined) {
    return killSwitchTripped(q.data) ? { kind: 'halted', state: q.data } : { kind: 'silent' }
  }
  // No state in hand. React Query keeps the last success across a failed
  // refetch, so reaching here means nothing has ever answered on this mount.
  if (q.isPending) return { kind: 'silent' }
  return { kind: 'unknown' }
}

/** The push channel's path. One place, so the banner and its test agree. */
export const KILL_SWITCH_STREAM = '/api/desk/kill-switch/events'

/**
 * What one frame of `GET /api/desk/kill-switch/events` does to the banner's
 * state (§19.6, "within one event").
 *
 * The frame carries the halted scopes **read from `monitor.kill_switch`**, not
 * the trading-status mirror. So:
 *
 *  - a global halt is applied to the cached state **at once** — that is the
 *    event, and waiting for a refetch of the mirror would put the poll back
 *    between the trip and the banner. The mirror is not re-read in the same
 *    breath, because it refreshes on the same notification and a read that
 *    raced it would put the stale "armed" back over the frame;
 *  - anything else (a reset, a strategy-scoped halt, an unreadable frame) asks
 *    for a refetch rather than clearing the banner itself: the frame knows the
 *    monitor's table, and only the status route knows whether the gate agrees.
 *
 * `null` for `next` means "leave the cache alone".
 */
export function applyKillSwitchFrame(
  prev: KillSwitchState | null | undefined,
  content: Record<string, unknown>,
): { next: KillSwitchState | null; refetch: boolean } {
  if (content.readable === true && content.global_halted === true) {
    return {
      next: {
        ...(prev ?? {}),
        trading_enabled: false,
        kill_switch_active: true,
        scope: 'global',
      },
      refetch: false,
    }
  }
  return { next: null, refetch: true }
}
