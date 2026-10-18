// The global kill-switch banner (AGENT-012 §11, §14; §19.6).
//
// The acceptance is unusually strict and deliberately so: the banner appears on
// **every** route within one event of the trip and **cannot be dismissed while
// tripped**. So there is no dismiss control in this component at all — not a
// disabled one, not one behind a confirm. A banner with a close button is a
// banner that will be closed.
//
// **"Within one event" is a push now, with the poll underneath it.**
// `GET /api/desk/kill-switch/events` holds a `LISTEN` on `monitor_kill_switch`
// — the channel `monitor.kill_switch`'s trigger has published on since 0075 —
// and sends the halted scopes on every change. A global halt is applied to
// this banner from the frame itself (`applyKillSwitchFrame`). The three-second
// poll stays: it is what the banner falls back to when the stream is down, and
// what reconciles a reset.
//
// It lives in `AppLayout`, above the router outlet, which is why "every route"
// is a structural property rather than something each page has to remember.
import { useCallback } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { deskApi, type KillSwitchState } from '@/api/desk'
import { useDeskStream } from '@/hooks/useDeskStream'
import type { WireEvent } from '@/lib/desk/events'
import {
  KILL_SWITCH_QUERY_KEY,
  KILL_SWITCH_STREAM,
  applyKillSwitchFrame,
  killSwitchView,
} from './KillSwitchBanner.helpers'

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
  const qc = useQueryClient()
  // The push half (§19.6). Every frame either sets the halted state directly
  // or asks the status route again; see `applyKillSwitchFrame` for which.
  const onFrame = useCallback(
    (e: WireEvent) => {
      const { next, refetch } = applyKillSwitchFrame(
        qc.getQueryData<KillSwitchState | null>(KILL_SWITCH_QUERY_KEY),
        e.content ?? {},
      )
      if (next) qc.setQueryData(KILL_SWITCH_QUERY_KEY, next)
      if (refetch) void qc.invalidateQueries({ queryKey: KILL_SWITCH_QUERY_KEY })
    },
    [qc],
  )
  useDeskStream({ url: KILL_SWITCH_STREAM, onEvent: onFrame })
  const q = useQuery({
    queryKey: KILL_SWITCH_QUERY_KEY,
    queryFn: () => deskApi.killSwitch(),
    // **Three seconds, underneath the push.** The stream above is what makes a
    // trip from the risk engine or another operator reach this banner within
    // one event; this poll is what holds when the stream is down (the route
    // unregistered, the `LISTEN` refused, the connection dropped), and what
    // clears the banner after a reset. A halt made from this client also
    // invalidates `KILL_SWITCH_QUERY_KEY` directly.
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
