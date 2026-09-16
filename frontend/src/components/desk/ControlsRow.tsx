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

export interface DeskControls {
  paused: boolean
  /** True while the agent is working — never used to disable anything. */
  busy: boolean
  /** The stream is not live; steering and approving cannot land. */
  disconnected: boolean
  /** Whether an approval currently has focus, so `a`/`x` mean something. */
  hasPendingApproval: boolean
  detailsOpen: boolean
  onPause: () => void
  onResume: () => void
  onStop: () => void
  onSteer: () => void
  onApprove: () => void
  onDeny: () => void
  onEscalate: () => void
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
        case ' ':
          e.preventDefault()
          if (k.paused) k.onResume()
          else k.onPause()
          break
        case 'a':
          if (k.hasPendingApproval) k.onApprove()
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
      {props.paused ? (
        <Button size="xs" variant="primary" onClick={props.onResume} data-testid="resume">
          Resume <kbd>space</kbd>
        </Button>
      ) : (
        <Button size="xs" variant="secondary" onClick={props.onPause} data-testid="pause">
          Pause <kbd>space</kbd>
        </Button>
      )}
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
          <Button size="xs" variant="primary" onClick={props.onApprove} disabled={props.disconnected}>
            Approve <kbd>a</kbd>
          </Button>
          <Button size="xs" variant="secondary" onClick={props.onDeny} disabled={props.disconnected}>
            Deny <kbd>x</kbd>
          </Button>
        </>
      )}
      <Button size="xs" variant="ghost" onClick={props.onEscalate}>
        Escalate…
      </Button>
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
        consequence="The working state is kept and the run can be forked from where it stopped. Nothing already spent is refunded."
        confirmLabel="Stop"
        alternative={
          <Button
            size="sm"
            onClick={() => {
              setConfirmStop(false)
              props.onPause()
            }}
          >
            Pause instead
          </Button>
        }
        onConfirm={() => {
          setConfirmStop(false)
          props.onStop()
        }}
      />
    </div>
  )
}
