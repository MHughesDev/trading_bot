/**
 * The two controls on the kill-switch panel, one of which could not ever have
 * worked (AGENT-012 §11; COMP-007 §5; Set S task S-6.21).
 *
 * `reset_kill_switch` takes `Json<ResumeBody>` **by value**, so a POST with no
 * body is rejected by the extractor before the handler runs. The panel's
 * `Reset (logged)` button was `void api.post('/api/trading/resume')` — no body,
 * and no `.catch`. Every click 422'd into a dropped promise. The button had
 * never once reset anything, and because it said nothing when it failed there
 * was no way to tell that from a switch clearing slowly.
 *
 * `Halt all` was the same shape with a kinder server: `trip_kill_switch` takes
 * `Option<Json<KillBody>>`, so a bodiless POST goes through and the monitor logs
 * the reason `manual kill from /api/trading/kill` against a human's name. A 403
 * from `permits_monitor_control` still reached nobody — and a silent halt on the
 * control that stops money moving is an operator walking away believing trading
 * has stopped.
 *
 * This is the same track's finding about `Scale down`, one panel up: the client
 * now refuses **here** what the route refuses **there**, before the request
 * rather than as a status code after it. `confirm_scope` is compared byte for
 * byte against `Scope::Global`'s `Display`, which is the bare word `global`.
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { GlobalActionDialog } from '../Operations'
import { GLOBAL_SCOPE, globalActionBody } from '@/components/desk/Operations.helpers'

describe('the global reset asks for what the route requires', () => {
  it('refuses without a reason, which `ResetRequest::validate` refuses too', () => {
    const built = globalActionBody('resume', { reason: '   ', typed: GLOBAL_SCOPE })
    expect(built.ok).toBe(false)
    expect(built.ok === false && built.refusal).toMatch(/reason/)
  })

  it('refuses a confirmation that is not the scope, case included', () => {
    // `validate` compares `self.typed_confirmation != expected` literally, so a
    // friendlier word would produce a dialog that always ends in a 409.
    for (const typed of ['', 'GLOBAL', 'Global', 'all', 'HALT']) {
      const built = globalActionBody('resume', { reason: 'venue reconciled', typed })
      expect(built.ok).toBe(false)
    }
  })

  it('builds the body the handler deserialises, once both are given', () => {
    const built = globalActionBody('resume', { reason: 'venue reconciled', typed: GLOBAL_SCOPE })
    expect(built).toEqual({
      ok: true,
      action: 'resume',
      body: { confirm_scope: 'global', reason: 'venue reconciled' },
    })
  })
})

describe('the global halt is reasoned rather than anonymous', () => {
  it('refuses without a reason, so nothing is logged as a default sentence', () => {
    const built = globalActionBody('halt', { reason: '', typed: GLOBAL_SCOPE })
    expect(built.ok).toBe(false)
  })

  it('carries the typed reason, which is what the monitor records', () => {
    const built = globalActionBody('halt', { reason: 'venue feed stuck', typed: GLOBAL_SCOPE })
    expect(built).toEqual({ ok: true, action: 'halt', reason: 'venue feed stuck' })
  })

  it('trims the reason rather than accepting whitespace as one', () => {
    expect(globalActionBody('halt', { reason: '  \n ', typed: GLOBAL_SCOPE }).ok).toBe(false)
    expect(globalActionBody('halt', { reason: '  x  ', typed: GLOBAL_SCOPE })).toEqual({
      ok: true,
      action: 'halt',
      reason: 'x',
    })
  })
})

describe('the dialog states the refusal before the request', () => {
  it('keeps the confirm disabled and says why', () => {
    const onConfirm = vi.fn()
    render(
      <GlobalActionDialog
        action="resume"
        halted={[]}
        onCancel={() => {}}
        onConfirm={onConfirm}
      />,
    )
    const confirm = screen.getByTestId('global-action-confirm')
    expect(confirm).toBeDisabled()
    expect(screen.getByTestId('global-action-refusal').textContent).toMatch(/reason/)
    fireEvent.click(confirm)
    expect(onConfirm).not.toHaveBeenCalled()
  })

  it('enables it once the reason and the scope are both given', () => {
    const onConfirm = vi.fn()
    render(
      <GlobalActionDialog action="resume" halted={[]} onCancel={() => {}} onConfirm={onConfirm} />,
    )
    fireEvent.change(screen.getByLabelText('Reason'), { target: { value: 'venue reconciled' } })
    fireEvent.change(screen.getByLabelText('Typed confirmation'), { target: { value: GLOBAL_SCOPE } })
    fireEvent.click(screen.getByTestId('global-action-confirm'))
    expect(onConfirm).toHaveBeenCalledWith({
      ok: true,
      action: 'resume',
      body: { confirm_scope: 'global', reason: 'venue reconciled' },
    })
  })

  it('warns that a halted venue will block the reset, and says so before the 409', () => {
    render(
      <GlobalActionDialog
        action="resume"
        halted={['venue:kraken']}
        onCancel={() => {}}
        onConfirm={() => {}}
      />,
    )
    expect(screen.getByTestId('global-venue-note').textContent).toMatch(/venue:kraken/)
  })

  it('does not promise the reset will go through on a scope list it never read', () => {
    render(
      <GlobalActionDialog action="resume" halted={null} onCancel={() => {}} onConfirm={() => {}} />,
    )
    expect(screen.getByTestId('global-venue-note').textContent).toMatch(/could not be read/)
  })

  it('reports a refusal the monitor did give, out loud', () => {
    render(
      <GlobalActionDialog
        action="halt"
        halted={[]}
        error="reset_refused — another reset was made too recently"
        onCancel={() => {}}
        onConfirm={() => {}}
      />,
    )
    const alert = screen.getByTestId('global-action-error')
    // A control that swallows a refusal reports failure as silence, which on
    // this panel reads as success.
    expect(alert).toHaveAttribute('role', 'alert')
    expect(alert.textContent).toMatch(/too recently/)
  })

  it('renders nothing when no action is open', () => {
    const { container } = render(
      <GlobalActionDialog action={null} halted={[]} onCancel={() => {}} onConfirm={() => {}} />,
    )
    expect(container.textContent).toBe('')
  })
})
