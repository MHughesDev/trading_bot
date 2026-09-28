import { useEffect, useRef } from 'react'
import type { DeskControls } from './ControlsRow'

/**
 * The desk keyboard map (§3.4).
 *
 * **`esc` is not here, and that is the fix.** §3.4 spells the binding
 * "`esc` stop (confirm)", and this hook used to answer `Escape` by calling
 * `onStop()` — which in `Conversation.tsx` is `deskApi.cancel(conversationId)`,
 * the request itself. So the mouse path went through `ControlsRow`'s
 * `ConfirmDialog`, stating what is lost and what is not refunded, and the
 * keyboard path ended the run outright. On the universal dismiss key: the branch
 * ran *before* the `isTyping` guard, so `Escape` while typing in the composer
 * cancelled the run, and `Overlay`'s popovers and selects close on `Escape`
 * without stopping propagation, so dismissing a model picker did too.
 *
 * `esc` still reaches Stop from inside the composer — that property was worth
 * keeping and is tested — but it now opens the dialog the button opens.
 * `ControlsRow` owns that dialog, so `ControlsRow` owns the key: a shortcut bound
 * in one file to state held in another is how these two came to disagree.
 */
export function useDeskKeymap(c: DeskControls) {
  // The latest-controls ref is refreshed in an effect, not during render.
  // Writing a ref while rendering is a real hazard under concurrent rendering:
  // a render that React throws away still leaves its write behind, so the
  // listener can end up holding controls from a tree that was never committed.
  // Every read here happens from a `keydown` handler, which runs long after the
  // commit, so the effect refresh is not a timing change -- it is the same value
  // arriving by a route that cannot be poisoned by an abandoned render.
  const ref = useRef(c)
  useEffect(() => {
    ref.current = c
  })

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const k = ref.current
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
