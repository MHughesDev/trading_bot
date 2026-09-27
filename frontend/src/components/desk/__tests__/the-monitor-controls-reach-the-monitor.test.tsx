/**
 * `Scale down` was a button that reached nothing (AGENT-012 §11; Set S task
 * S-6.21).
 *
 * `<Button size="xs" variant="ghost">Scale down</Button>` — no `onClick`, on the
 * operator's page, beside a live strategy. It rendered, it took the hover, it
 * took the click, and `POST /api/monitor/strategies/{id}/{action}` had been
 * served since COMP-007 landed. This repository's dominant defect shape with the
 * halves swapped: a served route with no caller, wearing a control's clothes.
 *
 * It is the worst place in the desk for that shape, because the failure is
 * silent *and* reassuring. An operator scales a losing strategy back, the dialog
 * closes, the table still says 100%, and the natural reading of a table that did
 * not change is that it had not refreshed yet.
 *
 * So these assert three things the wiring has to get right, each of which the
 * route refuses on and none of which the person can see from a 400:
 *
 * 1. The body matches the action. `scale` refuses without `scale_pct` and
 *    outside `1..=100`; `halt` refuses without a reason; `resume` is a reset and
 *    needs a `confirm_scope` equal to `strategy:<id>` **byte for byte**.
 * 2. A refusal is shown. `require_control` 403s an agent or service token and
 *    the reset gate 409s inside its rate limit, and a control that swallows
 *    those reports failure as silence.
 * 3. The confirm is two-step and states the consequence with the numbers.
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { StrategyActionDialog } from '../Operations'
import { strategyActionBody, strategyActionRefusal, strategyScope } from '@/components/desk/Operations.helpers'

describe('the body is the one the action refuses without', () => {
  it('sends a scale percentage, because `scale_pct is required`', () => {
    const built = strategyActionBody('scale', { strategyId: 'st_7', reason: '', scalePct: 25 })
    expect(built.ok).toBe(true)
    expect(built.ok && built.body.scale_pct).toBe(25)
  })

  it('refuses 0 here rather than letting the route refuse it there', () => {
    // `scale_pct must be in 1..=100`. Zero is not "off": halting is a separate
    // action with a separate record, and offering 0 as a scale would let a
    // person believe they had stopped a strategy with a scale event.
    const zero = strategyActionBody('scale', { strategyId: 'st_7', reason: '', scalePct: 0 })
    expect(zero.ok).toBe(false)
    expect(zero.ok === false && zero.refusal).toMatch(/halt it/)
    expect(strategyActionBody('scale', { strategyId: 'st_7', reason: '', scalePct: 101 }).ok).toBe(false)
    // …and the valid end of the range still builds, so the two above are the
    // bound working rather than the function refusing everything.
    expect(strategyActionBody('scale', { strategyId: 'st_7', reason: '', scalePct: 100 }).ok).toBe(true)
  })

  it('will not build a halt without a reason, which the route requires', () => {
    const blank = strategyActionBody('halt', { strategyId: 'st_7', reason: '   ' })
    expect(blank.ok).toBe(false)
    const given = strategyActionBody('halt', { strategyId: 'st_7', reason: 'drawdown past the limit' })
    expect(given.ok).toBe(true)
    expect(given.ok && given.body.reason).toBe('drawdown past the limit')
    // The route defaults this itself, but sending it means the desk is not
    // silently choosing what happens to an open position.
    expect(given.ok && given.body.halt_policy).toBe('hold')
  })

  it('types the resume confirmation exactly as the reset gate compares it', () => {
    expect(strategyScope('st_7')).toBe('strategy:st_7')
    const wrong = strategyActionBody('resume', {
      strategyId: 'st_7',
      reason: 'reconciled with the venue',
      typed: 'STRATEGY:ST_7',
    })
    // `ResetRequest::validate` compares `self.typed_confirmation != expected`
    // with no case folding, and `ConfirmDialog` folds case — which would have
    // sent an uppercase word into a 409 nobody could explain.
    expect(wrong.ok).toBe(false)
    const right = strategyActionBody('resume', {
      strategyId: 'st_7',
      reason: 'reconciled with the venue',
      typed: 'strategy:st_7',
    })
    expect(right.ok).toBe(true)
    expect(right.ok && right.body.confirm_scope).toBe('strategy:st_7')
    expect(right.ok && right.body.reason).toBe('reconciled with the venue')
  })

  it('refuses a resume with no reason, which is a reset with no reason', () => {
    expect(
      strategyActionBody('resume', { strategyId: 'st_7', reason: '', typed: 'strategy:st_7' }).ok,
    ).toBe(false)
  })
})

describe('a refusal from the monitor is read, never swallowed', () => {
  it('names the scope refusal an agent token draws', () => {
    const text = strategyActionRefusal({
      response: {
        status: 403,
        data: {
          error: 'monitor_control_required',
          detail: 'tripping, scaling, halting, resuming or resetting the monitor requires a human session',
        },
      },
    })
    expect(text).toContain('monitor_control_required')
    expect(text).toContain('human session')
  })

  it('says nothing was changed when the request never landed', () => {
    const text = strategyActionRefusal(new Error('Network Error'))
    expect(text).toContain('Nothing was changed')
  })

  it('does not invent a reason when the monitor gave none', () => {
    const text = strategyActionRefusal({ response: { status: 500, data: {} } })
    expect(text).toContain('500')
    expect(text).toContain('Nothing was changed')
  })
})

describe('the dialog is a second step, and it states the consequence', () => {
  it('renders nothing at all until an action is chosen', () => {
    render(
      <StrategyActionDialog
        action={null}
        strategyId="st_7"
        currentScale={100}
        onCancel={() => {}}
        onConfirm={() => {}}
      />,
    )
    expect(screen.queryByTestId('strategy-action-confirm')).toBeNull()
  })

  it('names the strategy and the numbers before it will confirm', () => {
    render(
      <StrategyActionDialog
        action="scale"
        strategyId="st_7"
        currentScale={100}
        onCancel={() => {}}
        onConfirm={() => {}}
      />,
    )
    const said = screen.getByTestId('strategy-action-consequence').textContent ?? ''
    expect(said).toContain('st_7')
    expect(said).toContain('50%')
    expect(said).toContain('down from 100%')
  })

  it('says the current scale is unknown rather than implying a change from 100%', () => {
    // `scale_pct` is null when the monitor did not report one, and "scaled down
    // to 25%" silently claims it was at full size before.
    render(
      <StrategyActionDialog
        action="scale"
        strategyId="st_7"
        currentScale={null}
        onCancel={() => {}}
        onConfirm={() => {}}
      />,
    )
    expect(screen.getByTestId('strategy-action-consequence').textContent).toContain(
      'has not reported its current scale',
    )
  })

  it('sends the body the route wants when the person confirms', () => {
    const onConfirm = vi.fn()
    render(
      <StrategyActionDialog
        action="scale"
        strategyId="st_7"
        currentScale={100}
        onCancel={() => {}}
        onConfirm={onConfirm}
      />,
    )
    fireEvent.change(screen.getByLabelText('Percentage of full size'), { target: { value: '25' } })
    fireEvent.change(screen.getByLabelText('Reason'), { target: { value: 'behaviour drift' } })
    fireEvent.click(screen.getByTestId('strategy-action-confirm'))
    expect(onConfirm).toHaveBeenCalledWith({ scale_pct: 25, reason: 'behaviour drift' })
  })

  it('will not confirm a halt with no reason, and says why', () => {
    const onConfirm = vi.fn()
    render(
      <StrategyActionDialog
        action="halt"
        strategyId="st_7"
        currentScale={100}
        onCancel={() => {}}
        onConfirm={onConfirm}
      />,
    )
    expect(screen.getByTestId('strategy-action-refusal').textContent).toContain('needs a reason')
    fireEvent.click(screen.getByTestId('strategy-action-confirm'))
    expect(onConfirm).not.toHaveBeenCalled()

    fireEvent.change(screen.getByLabelText('Reason'), { target: { value: 'past the drawdown limit' } })
    fireEvent.click(screen.getByTestId('strategy-action-confirm'))
    expect(onConfirm).toHaveBeenCalledWith({
      reason: 'past the drawdown limit',
      halt_policy: 'hold',
    })
  })

  it('shows the refusal the monitor gave instead of closing quietly', () => {
    render(
      <StrategyActionDialog
        action="halt"
        strategyId="st_7"
        currentScale={100}
        error="resume_refused — another reset was made 40 seconds ago"
        onCancel={() => {}}
        onConfirm={() => {}}
      />,
    )
    const alert = screen.getByTestId('strategy-action-error')
    expect(alert.textContent).toContain('resume_refused')
    expect(alert.getAttribute('role')).toBe('alert')
  })
})
