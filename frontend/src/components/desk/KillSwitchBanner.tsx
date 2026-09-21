// The global kill-switch banner (AGENT-012 §11, §14; §19.6).
//
// The acceptance is unusually strict and deliberately so: the banner appears on
// **every** route within one event of the trip and **cannot be dismissed while
// tripped**. So there is no dismiss control in this component at all — not a
// disabled one, not one behind a confirm. A banner with a close button is a
// banner that will be closed.
//
// It lives in `AppLayout`, above the router outlet, which is why "every route"
// is a structural property rather than something each page has to remember.
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { deskApi, type KillSwitchState } from '@/api/desk'

/** The event a live stream can fire to beat the poll. */
export const KILL_SWITCH_EVENT = 'desk:kill-switch'

export function killSwitchTripped(s: KillSwitchState | null | undefined): boolean {
  if (!s) return false
  return s.kill_switch_active === true || s.trading_enabled === false
}

/**
 * What the chrome may claim about live trading, given what answered.
 *
 * `killSwitchTripped(undefined)` is `false`, and the banner rendered `null` on
 * it — so a `/api/trading/status` that **did not answer** produced exactly the
 * screen a healthy, armed platform produces: no banner, on every route. The
 * Monitor page's own badge grew a third word (`state unknown`) for precisely
 * this a track ago; the chrome that the operator actually walks past did not,
 * and the chrome is the surface §19.6 is written about.
 *
 * `optional()` only folds 404/405/501 into `null` and rethrows everything else,
 * so both halves of "nothing came back" have to be handled: `null` from a route
 * that says it is not there, and `undefined` from a 500, a 401 or a dropped
 * connection.
 *
 * Silence while the **first** read is in flight is deliberate and is not the
 * same defect. A banner that flashes "state unknown" on every page load is a
 * banner the operator learns to look past, which costs the real one its only
 * job. Unknown is claimed once the request has settled with nothing.
 */
export type KillSwitchView =
  | { kind: 'silent' }
  | { kind: 'halted'; state: KillSwitchState }
  | { kind: 'unknown' }

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

export function KillSwitchBanner() {
  const q = useQuery({
    queryKey: ['desk', 'kill-switch'],
    queryFn: () => deskApi.killSwitch(),
    // Three seconds is the poll floor; a live event refetches immediately, so
    // "within one event" holds wherever the stream is connected and degrades to
    // "within three seconds" where it is not. Said plainly rather than claimed.
    refetchInterval: 3_000,
    refetchOnWindowFocus: true,
  })

  const view = killSwitchView(q)
  if (view.kind === 'silent') return null

  if (view.kind === 'unknown') {
    return (
      <div
        className="kill-banner unknown"
        role="status"
        aria-live="polite"
        data-testid="kill-switch-unknown-banner"
      >
        <span aria-hidden>?</span>
        <span>
          Trading state unknown — /api/trading/status did not answer. This is not the same as
          trading being armed.
        </span>
        <Link to="/desk/monitor" style={{ marginLeft: 'auto', color: 'inherit' }}>
          Open the monitor
        </Link>
      </div>
    )
  }

  const s = view.state
  return (
    <div className="kill-banner" role="alert" aria-live="assertive" data-testid="kill-switch-banner">
      <span aria-hidden>■</span>
      <span>
        Trading is halted{s.scope ? ` · ${s.scope}` : ''}
        {s.tripped_by ? ` · tripped by ${s.tripped_by}` : ''}
        {s.reason ? ` · ${s.reason}` : ''}
      </span>
      <Link to="/desk/monitor" style={{ marginLeft: 'auto', color: 'inherit' }}>
        Open the monitor
      </Link>
    </div>
  )
}
