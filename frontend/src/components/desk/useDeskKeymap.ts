import { useEffect, useRef } from 'react'
import type { DeskControls } from './ControlsRow'

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
          // `space` is bound only where the pause is *measured*. Under
          // `unobserved` it does nothing at all rather than firing a request
          // this replica cannot serve — the keyboard gets the same three-state
          // treatment as the buttons, because a shortcut that silently 409s is
          // the version of this control that was deleted.
          if (k.pause.kind === 'running') {
            e.preventDefault()
            k.onPause()
          } else if (k.pause.kind === 'paused') {
            e.preventDefault()
            k.onResume()
          }
          break
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

/** True when a keystroke belongs to whatever the person is typing into. */
function isTyping(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null
  if (!el || !el.tagName) return false
  const tag = el.tagName.toLowerCase()
  return tag === 'input' || tag === 'textarea' || tag === 'select' || el.isContentEditable === true
}
