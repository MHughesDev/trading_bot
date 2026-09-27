// The global kill-switch banner (AGENT-012 §11, §14; §19.6).
//
// The acceptance is unusually strict and deliberately so: the banner appears on
// **every** route within one event of the trip and **cannot be dismissed while
// tripped**. So there is no dismiss control in this component at all — not a
// disabled one, not one behind a confirm. A banner with a close button is a
// banner that will be closed.
//
// **The "within one event" half is not met, and this file says so rather than
// implying it.** See the comment on `refetchInterval` below: it is a three-
// second poll, plus immediate invalidation for a halt made from this client.
// Meeting the criterion needs a push channel for kill-switch state, which the
// platform does not have.
//
// It lives in `AppLayout`, above the router outlet, which is why "every route"
// is a structural property rather than something each page has to remember.
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { deskApi, type KillSwitchState } from '@/api/desk'
import { KILL_SWITCH_QUERY_KEY, killSwitchTripped, killSwitchView } from './KillSwitchBanner.helpers'

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

export function KillSwitchBanner() {
  const q = useQuery({
    queryKey: KILL_SWITCH_QUERY_KEY,
    queryFn: () => deskApi.killSwitch(),
    // **Three seconds. That is the guarantee, and it is the only one.**
    //
    // §19.6 asks for the banner within one event of the trip, and this comment
    // used to claim it: "a live event refetches immediately, so 'within one
    // event' holds wherever the stream is connected". Nothing streams
    // kill-switch state to a browser — `useDeskStream` is per-run and there is
    // no global channel — and the event the sentence referred to was dispatched
    // by nobody. A trip from the risk engine or from another operator reaches
    // this banner by poll and by nothing else, so it can be up to three seconds
    // late, and the comment that said otherwise was the only thing standing
    // between that fact and the acceptance criterion it fails.
    //
    // The one case that *is* immediate is a halt made from this client, because
    // the control invalidates `KILL_SWITCH_QUERY_KEY` — which is a property of
    // those two call sites, not of this banner, and is worth exactly what it
    // says and no more.
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
