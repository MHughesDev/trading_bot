// The controls row and the keyboard map (AGENT-012 §3.4, principle 10; task S-6.6).
//
// Meridian principle 10, restated as a constraint on this file: **the human's
// controls are always visible and always work** — including while the agent is
// mid-call. That is why the key handler is bound to the document rather than to
// a focused element, and why nothing here is disabled by a `busy` flag. A
// control that greys out while the model is thinking is a control that is absent
// exactly when it is wanted.
//
// The one thing that *does* disable Steer and Approve is a dropped event stream
// (§14): they need the stream to land, and a button that silently does nothing is
// worse than one that says why.
import { useState } from 'react'
import { Button } from '@/components/primitives/Button'
import { ConfirmDialog } from '@/components/primitives/Modal'
import { Tooltip } from '@/components/primitives/Overlay'

/**
 * What the **server** says about this run's pause. Never a local guess.
 *
 * The history matters, because this type is the shape of the fix. Pause, Resume
 * and Escalate were three buttons in front of three routes this platform did not
 * mount; `deskApi` sent all three through `optional()`, which turns a 404 into
 * `null`, so a click produced no request that could be seen to fail. Pause was
 * the worst, because it did not merely do nothing: its caller set a local
 * `paused` flag *before* the request, so the header read **paused** over a run
 * that was still spending trials. All three were deleted.
 *
 * `…/pause` and `…/resume` landed with ADR-D-27, and they landed with the cure
 * for that specific defect built in — `paused` and `paused_until` are on every
 * **run read**, not only on the pause reply, so a client that reloads can ask.
 * This type refuses to represent the flag that caused the bug: there is no
 * variant for "I clicked pause, so we are paused". `unobserved` is a third
 * state, not a synonym for `false` — `paused: null` means this replica is not
 * driving the run and did not measure it, and a pause posted into that answers
 * 409, so no control is drawn for it.
 *
 * **Escalate is still deleted**: `…/runs/{id}/escalate` is still unmounted.
 */
export type PauseState =
  | { kind: 'unobserved' }
  | { kind: 'running' }
  | { kind: 'paused'; until: string | null }

/**
 * **Stop stays what it was**: `POST /api/agent/conversations/{id}/cancel` is
 * mounted, so `esc` and the dialog reach something real. Its dialog no longer
 * claims there is no pause on this platform — there is one now — but it still
 * does not offer one as the gentler alternative, because routing a person from
 * the control that ends a run to one that delays it by fifteen minutes is how
 * the original defect got its reach.
 *
 * Asserted by `a-control-nobody-wired-is-not-a-control.test.tsx`.
 */
export interface DeskControls {
  /** True while the agent is working — never used to disable anything. */
  busy: boolean
  /** The stream is not live; steering and approving cannot land. */
  disconnected: boolean
  /** Whether an approval currently has focus, so `a`/`x` mean something. */
  hasPendingApproval: boolean
  /**
   * The waiting approval is one the platform will refuse without a written
   * reason (an irreversible envelope — `spend_holdout_look` wants twenty
   * characters of it). The keyboard has nowhere to type one, so `a` must not
   * fire a request that is going to be rejected in silence.
   */
  approvalNeedsReason: boolean
  detailsOpen: boolean
  /**
   * The run's pause as the server reports it. `unobserved` draws no pause
   * control, because a pause this replica cannot hold is a 409 and not a button.
   */
  pause: PauseState
  onStop: () => void
  onPause: () => void
  onResume: () => void
  onSteer: () => void
  onApprove: () => void
  onDeny: () => void
  onBudget: () => void
  onToggleDetails: () => void
  onReplay: () => void
  onFocusComposer: () => void
  onShortcuts: () => void
}

/**
 * The deadline as a clock time, or a stated absence.
 *
 * A pause whose `until` did not arrive is still a pause — the run is held — so
 * the label says it is held and declines to invent a time. Printing "stops at
 * —" or falling back to `now` would both be a deadline the reader is wrong
 * about, which is the failure `pause_on_expiry` exists on the wire to prevent.
 */
function pausedLabel(until: string | null): string {
  if (!until) return 'held · deadline not reported'
  const t = new Date(until)
  if (Number.isNaN(t.getTime())) return 'held · deadline not reported'
  return `held until ${t.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
}

function pausedTooltip(until: string | null): string {
  const tail = 'On expiry the run stops rather than resuming.'
  if (!until) return `This run is held at a step boundary. ${tail}`
  const t = new Date(until)
  if (Number.isNaN(t.getTime())) return `This run is held at a step boundary. ${tail}`
  return `Held until ${t.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}. ${tail}`
}

export function ControlsRow(props: DeskControls) {
  const [confirmStop, setConfirmStop] = useState(false)
  const needsStream = props.disconnected ? 'the event stream is down; this cannot land' : undefined

  return (
    <div className="desk-controls" role="toolbar" aria-label="Run controls">
      <Button size="xs" variant="secondary" onClick={() => setConfirmStop(true)} data-testid="stop">
        Stop <kbd>esc</kbd>
      </Button>
      {props.pause.kind === 'running' && (
        <Tooltip content="Hold the run at its next step boundary. Bounded: it stops on its own if nobody resumes it.">
          <span>
            <Button size="xs" variant="secondary" onClick={props.onPause} data-testid="pause">
              Pause <kbd>space</kbd>
            </Button>
          </span>
        </Tooltip>
      )}
      {props.pause.kind === 'paused' && (
        <Tooltip content={pausedTooltip(props.pause.until)}>
          <span>
            <Button size="xs" variant="primary" onClick={props.onResume} data-testid="resume">
              Resume <kbd>space</kbd>
            </Button>
          </span>
        </Tooltip>
      )}
      {props.pause.kind === 'paused' && (
        <span className="desk-controls-note" data-testid="paused-until">
          {pausedLabel(props.pause.until)}
        </span>
      )}
      <Tooltip content={needsStream ?? 'Send an instruction into the running turn'}>
        <span>
          <Button size="xs" variant="secondary" onClick={props.onSteer} disabled={props.disconnected} data-testid="steer">
            Steer <kbd>s</kbd>
          </Button>
        </span>
      </Tooltip>
      {props.hasPendingApproval && (
        <>
          <Tooltip
            content={
              props.approvalNeedsReason
                ? 'This one is recorded permanently and the platform refuses it without a written reason. Approve it on the card, where the reason can be typed.'
                : (needsStream ?? 'Approve the waiting request')
            }
          >
            <span>
              <Button
                size="xs"
                variant="primary"
                onClick={props.onApprove}
                disabled={props.disconnected || props.approvalNeedsReason}
                data-testid="approve-from-controls"
              >
                Approve <kbd>a</kbd>
              </Button>
            </span>
          </Tooltip>
          <Button size="xs" variant="secondary" onClick={props.onDeny} disabled={props.disconnected}>
            Deny <kbd>x</kbd>
          </Button>
        </>
      )}
      <Button size="xs" variant="ghost" onClick={props.onBudget}>
        Budget
      </Button>
      <Button size="xs" variant="ghost" onClick={props.onToggleDetails} aria-pressed={props.detailsOpen}>
        Details <kbd>d</kbd>
      </Button>
      <Button size="xs" variant="ghost" onClick={props.onReplay}>
        Replay <kbd>r</kbd>
      </Button>
      <ConfirmDialog
        open={confirmStop}
        onCancel={() => setConfirmStop(false)}
        title="Stop this run?"
        consequence="The working state is kept and the run can be forked from where it stopped. Nothing already spent is refunded. This ends the run — a pause only holds it for a bounded window and then stops it anyway."
        confirmLabel="Stop"
        onConfirm={() => {
          setConfirmStop(false)
          props.onStop()
        }}
      />
    </div>
  )
}
