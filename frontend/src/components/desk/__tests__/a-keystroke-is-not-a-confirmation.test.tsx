/**
 * `esc` ended a run without asking (AGENT-012 §3.4; Meridian §5.5; task S-6.6).
 *
 * §3.4 spells the binding "**`esc` stop (confirm)**", and `ControlsRow` honours
 * the second word for the mouse: the button opens a `ConfirmDialog` that states
 * the consequence — the working state is kept, nothing spent is refunded, a pause
 * is not the gentler option. `useDeskKeymap` answered `Escape` with
 * `k.onStop()`, which in `Conversation.tsx:289` is
 * `deskApi.cancel(conversationId)`. The request. No dialog, no consequence, no
 * second thought.
 *
 * Three things made that worse than a missing confirmation:
 *
 *  - The `Escape` branch ran **before** the hook's `isTyping` guard, on purpose,
 *    so that stopping did not require clicking out of the composer. The result was
 *    that `Escape` pressed while typing a question ended the run.
 *  - `Overlay`'s popovers and selects close on `Escape` and do not stop
 *    propagation, and neither does `Modal`. So dismissing the model picker, a
 *    tooltip or another dialog also cancelled the run behind it.
 *  - `Escape` is the one key every person on every platform presses to mean
 *    "never mind". It was wired to the only irreversible control on the row.
 *
 * The binding now opens the dialog the button opens, from the same component that
 * owns it. These cases are the refusals: a keystroke is not a confirmation, and a
 * dismissal is not a decision.
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { ControlsRow, type DeskControls } from '../ControlsRow'
import { useDeskKeymap } from '../useDeskKeymap'

function spies() {
  return {
    onStop: vi.fn(),
    onPause: vi.fn(),
    onResume: vi.fn(),
    onSteer: vi.fn(),
    onApprove: vi.fn(),
    onDeny: vi.fn(),
    onBudget: vi.fn(),
    onToggleDetails: vi.fn(),
    onReplay: vi.fn(),
    onFocusComposer: vi.fn(),
    onShortcuts: vi.fn(),
  }
}

/** The screen as `Conversation` assembles it: the row, and the map beside it. */
function Desk(props: DeskControls) {
  useDeskKeymap(props)
  return <ControlsRow {...props} />
}

function mount(over: Partial<DeskControls> = {}) {
  const s = spies()
  const props: DeskControls = {
    busy: true,
    disconnected: false,
    hasPendingApproval: false,
    approvalNeedsReason: false,
    detailsOpen: false,
    pause: { kind: 'running' },
    ...s,
    ...over,
  }
  render(<Desk {...props} />)
  return s
}

describe('esc asks before it ends a run', () => {
  it('opens the dialog rather than sending the cancel', () => {
    const s = mount()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(s.onStop).not.toHaveBeenCalled()
    expect(screen.getByText('Stop this run?')).toBeTruthy()
  })

  it('states what stopping costs before anything is sent', () => {
    // A confirmation with no consequence in it is a second click, not a decision.
    const s = mount()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.getByText(/Nothing already spent is refunded/)).toBeTruthy()
    expect(s.onStop).not.toHaveBeenCalled()
  })

  it('sends the cancel once the dialog is confirmed, so the keyboard still reaches Stop', () => {
    const s = mount()
    fireEvent.keyDown(document, { key: 'Escape' })
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    expect(s.onStop).toHaveBeenCalledTimes(1)
  })

  it('sends nothing when the dialog is cancelled', () => {
    const s = mount()
    fireEvent.keyDown(document, { key: 'Escape' })
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(s.onStop).not.toHaveBeenCalled()
    expect(screen.queryByText('Stop this run?')).toBeNull()
  })

  it('still reaches Stop from inside the composer, which is why it was bound first', () => {
    const s = mount()
    const input = document.createElement('input')
    document.body.appendChild(input)
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(screen.getByText('Stop this run?')).toBeTruthy()
    expect(s.onStop).not.toHaveBeenCalled()
    input.remove()
  })
})

describe('a dismissal is not a decision', () => {
  it('leaves esc alone while a dialog is open, so closing one does not queue a stop', () => {
    // `Modal` renders `role="dialog"` and does not stop propagation. Before this
    // change, Escape pressed to dismiss *any* modal on the desk also cancelled the
    // run underneath it.
    const s = mount()
    const dialog = document.createElement('div')
    dialog.setAttribute('role', 'dialog')
    document.body.appendChild(dialog)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(s.onStop).not.toHaveBeenCalled()
    expect(screen.queryByText('Stop this run?')).toBeNull()
    dialog.remove()
  })

  it('leaves esc alone while a popover, select or tooltip is open', () => {
    const s = mount()
    for (const role of ['tooltip', 'listbox', 'menu']) {
      const el = document.createElement('div')
      el.setAttribute('role', role)
      document.body.appendChild(el)
      fireEvent.keyDown(document, { key: 'Escape' })
      expect(screen.queryByText('Stop this run?'), role).toBeNull()
      el.remove()
    }
    expect(s.onStop).not.toHaveBeenCalled()
  })

  it('ignores a modified escape, which belongs to the browser', () => {
    mount()
    for (const mods of [{ metaKey: true }, { ctrlKey: true }, { altKey: true }]) {
      fireEvent.keyDown(document, { key: 'Escape', ...mods })
      expect(screen.queryByText('Stop this run?')).toBeNull()
    }
  })

  it('does not reopen the dialog when escape is pressed while it is already open', () => {
    // §5.5 forbids dismissing a destructive confirm with Escape, so the second
    // press must be inert rather than a re-arm.
    const s = mount()
    fireEvent.keyDown(document, { key: 'Escape' })
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.getAllByText('Stop this run?')).toHaveLength(1)
    expect(s.onStop).not.toHaveBeenCalled()
  })
})

describe('the keyboard map no longer reaches past the dialog', () => {
  it('has no Escape branch in useDeskKeymap at all', () => {
    // A second listener calling `onStop` on Escape would restore the defect
    // silently, because the dialog would open *and* the run would end — which
    // looks, on screen, exactly like the fix working.
    const s = spies()
    const props: DeskControls = {
      busy: true,
      disconnected: false,
      hasPendingApproval: false,
      approvalNeedsReason: false,
      detailsOpen: false,
      pause: { kind: 'running' },
      ...s,
    }
    // The hook alone, with no ControlsRow and so no dialog to open.
    function Bare() {
      useDeskKeymap(props)
      return null
    }
    render(<Bare />)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(s.onStop).not.toHaveBeenCalled()
  })

  it('still binds the rest of the map, so this did not disarm the row', () => {
    const s = mount({ hasPendingApproval: true })
    fireEvent.keyDown(document, { key: 's' })
    fireEvent.keyDown(document, { key: 'd' })
    fireEvent.keyDown(document, { key: 'r' })
    fireEvent.keyDown(document, { key: 'a' })
    fireEvent.keyDown(document, { key: 'x' })
    fireEvent.keyDown(document, { key: '?' })
    fireEvent.keyDown(document, { key: ' ' })
    expect(s.onSteer).toHaveBeenCalled()
    expect(s.onToggleDetails).toHaveBeenCalled()
    expect(s.onReplay).toHaveBeenCalled()
    expect(s.onApprove).toHaveBeenCalled()
    expect(s.onDeny).toHaveBeenCalled()
    expect(s.onShortcuts).toHaveBeenCalled()
    expect(s.onPause).toHaveBeenCalled()
  })
})
