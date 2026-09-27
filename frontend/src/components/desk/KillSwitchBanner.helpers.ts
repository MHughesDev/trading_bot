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
