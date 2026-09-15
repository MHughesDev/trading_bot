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

  if (!killSwitchTripped(q.data)) return null
  const s = q.data as KillSwitchState

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
