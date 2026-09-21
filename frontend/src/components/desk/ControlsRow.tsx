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
import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/primitives/Button'
import { ConfirmDialog } from '@/components/primitives/Modal'
import { Tooltip } from '@/components/primitives/Overlay'

/**
 * **Pause, Resume and Escalate were deleted from this row, not repaired.**
 *
 * They were three buttons in front of three routes this platform does not
 * mount. `POST /api/agent/runs/{id}/pause`, `…/resume` and `…/escalate` all
 * answer **404** where every mounted route answers 401, and `deskApi` sent all
 * three through `optional()`, which turns a 404 into `null` — so the click
 * produced no request that could be seen to fail, no toast, and no console
 * line.
 *
 * Pause was the worst of the three, because it did not merely do nothing. Its
 * caller set a local `paused` flag first, so the conversation header changed to
 * **paused** over a run that was still spending trials. That is not a dead
 * control, it is a false instrument reading, and `space` was bound to it.
 * "Pause instead" was also offered as the gentler alternative inside the Stop
 * dialog — the one place a person is talked out of the control that works, into
 * the one that does not.
 *
 * **Stop stays**: `POST /api/agent/conversations/{id}/cancel` is mounted (405
 * to a GET, 401 to an unauthenticated POST), so `esc` and the dialog reach
 * something real.
 *
 * Asserted by `a_control_the_platform_cannot_serve_is_not_drawn`.
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
  onStop: () => void
  onSteer: () => void
  onApprove: () => void
  onDeny: () => void
  onBudget: () => void
  onToggleDetails: () => void
  onReplay: () => void
  onFocusComposer: () => void
  onShortcuts: () => void
}

/** True when a keystroke belongs to whatever the person is typing into. */
function isTyping(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null
  if (!el || !el.tagName) return false
  const tag = el.tagName.toLowerCase()
  return tag === 'input' || tag === 'textarea' || tag === 'select' || el.isContentEditable === true
}

/**
 * The desk keyboard map (§3.4).
 *
 * `esc` works even from inside the composer, because stopping a run is the one
 * thing a person should never have to click out of a text field to do.
 */
export function useDeskKeymap(c: DeskControls) {
  const ref = useRef(c)
  ref.current = c
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const k = ref.current
      if (e.key === 'Escape') {
        k.onStop()
        return
      }
      if (isTyping(e.target) || e.metaKey || e.ctrlKey || e.altKey) return
      switch (e.key) {
        case 'a':
          // Not `hasPendingApproval` alone: an approval that needs a reason
          // cannot be given one from the keyboard, and sending it without would
          // be refused by the platform and swallowed here.
          if (k.hasPendingApproval && !k.approvalNeedsReason) k.onApprove()
          break
        case 'x':
          if (k.hasPendingApproval) k.onDeny()
          break
        case 's':
          k.onSteer()
          break
        case 'd':
          k.onToggleDetails()
          break
        case 'r':
          k.onReplay()
          break
        case '/':
          e.preventDefault()
          k.onFocusComposer()
          break
        case '?':
          k.onShortcuts()
          break
        default:
          break
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [])
}

export function ControlsRow(props: DeskControls) {
  const [confirmStop, setConfirmStop] = useState(false)
  const needsStream = props.disconnected ? 'the event stream is down; this cannot land' : undefined

  return (
    <div className="desk-controls" role="toolbar" aria-label="Run controls">
      <Button size="xs" variant="secondary" onClick={() => setConfirmStop(true)} data-testid="stop">
        Stop <kbd>esc</kbd>
      </Button>
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
        consequence="The working state is kept and the run can be forked from where it stopped. Nothing already spent is refunded. There is no pause on this platform, so this is the only way to make it stop working."
        confirmLabel="Stop"
        onConfirm={() => {
          setConfirmStop(false)
          props.onStop()
        }}
      />
    </div>
  )
}
