/**
 * A budget bar with no cap must not draw itself as a full one, or an empty one
 * (AGENT-012 §6.2; Set S task S-6.13).
 *
 * `BudgetView` carries `steps_total` and then three *remainders* — tokens, money
 * and trials spent — with no denominators. The meter computed one anyway:
 *
 *   const tokensCap = caps?.tokens ?? budget.tokens_remaining
 *   const tokensUsed = Math.max(0, tokensCap - budget.tokens_remaining)  // 0
 *
 * so a run that had spent half a million tokens rendered **"0 / 40,000"** — a
 * measurement nobody made, shown as the most reassuring number available. The
 * trials bar was the same defect pointing the other way: `max(spent, 1)` as its
 * own cap made every run render "4 / 4 · 90%+", which reads as a run about to be
 * cut off when nothing had said how many trials it had.
 *
 * Both are the platform's one non-negotiable, inside the component whose whole
 * job is to say how much is left: zero means measured and it was zero, absent
 * means nobody said.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { BudgetView } from '@/api/desk'
import { BudgetMeter, budgetBars } from '../Shared'

function budget(over: Partial<BudgetView> = {}): BudgetView {
  return {
    steps_remaining: 6,
    steps_total: 20,
    tokens_cap: null,
    tokens_remaining: 40_000,
    usd_tenth_cents_cap: null,
    usd_tenth_cents_remaining: 12_000,
    trials_cap: null,
    trials_spent: 4,
    n_eff: 2.4,
    ...over,
  }
}

describe('a bar is capped only when something said what the cap is', () => {
  it('caps steps from the snapshot, which is the one field that carries a total', () => {
    const [steps] = budgetBars(budget())
    expect(steps.value).toEqual({ capped: true, used: 14, cap: 20 })
  })

  it('leaves tokens, money and trials uncapped when no cap was passed', () => {
    const [, tokens, money, trials] = budgetBars(budget())
    expect(tokens.value).toEqual({ capped: false, value: 40_000, word: 'remaining' })
    expect(money.value).toEqual({ capped: false, value: 12_000, word: 'remaining' })
    expect(trials.value).toEqual({ capped: false, value: 4, word: 'spent' })
  })

  it('uses a cap the caller was actually given', () => {
    const bars = budgetBars(budget(), { tokens: 100_000, usdTenthCents: 25_000, trials: 10 })
    expect(bars[1].value).toEqual({ capped: true, used: 60_000, cap: 100_000 })
    expect(bars[3].value).toEqual({ capped: true, used: 4, cap: 10 })
  })

  it('treats a zero steps_total as no total, not as a cap of zero', () => {
    // `emptyWorkingState` starts every field at zero, so a snapshot that has not
    // arrived yet would otherwise render "0 / 0 · 90%+" — a run reported as out
    // of steps before it has taken one.
    const [steps] = budgetBars(budget({ steps_total: 0, steps_remaining: 0 }))
    expect(steps.value).toEqual({ capped: false, value: 0, word: 'remaining' })
  })
})

describe('the meter prints the absence rather than a zero', () => {
  it('says "no cap recorded" where it used to print 0 / remaining', () => {
    render(<BudgetMeter budget={budget()} />)
    expect(screen.getByText(/40,000 remaining · no cap recorded/)).toBeInTheDocument()
    expect(screen.getByText(/4 spent · no cap recorded/)).toBeInTheDocument()
    // The old lie, explicitly: tokens used out of tokens remaining.
    expect(screen.queryByText(/0 \/ 40,000/)).toBeNull()
  })

  it('never claims an uncapped bar is 90% spent', () => {
    render(<BudgetMeter budget={budget()} />)
    expect(screen.queryByText(/90%\+/)).toBeNull()
    // …and the warning still appears when there is a real cap to be near.
    render(<BudgetMeter budget={budget()} caps={{ trials: 4 }} />)
    expect(screen.getByText(/4 \/ 4 · 90%\+/)).toBeInTheDocument()
  })

  it('still draws four bars, so an uncapped one is present rather than hidden', () => {
    render(<BudgetMeter budget={budget()} />)
    const meters = screen.getAllByRole('meter')
    expect(meters.length).toBe(4)
    // An uncapped meter reports no value at all — a meter at 0 would be a claim.
    const uncapped = meters.filter((m) => m.getAttribute('aria-valuenow') === null)
    expect(uncapped.length).toBe(3)
    expect(uncapped[0].getAttribute('aria-valuetext')).toContain('no cap recorded')
  })

  it('keeps the capped bar exactly as it was', () => {
    render(<BudgetMeter budget={budget()} />)
    expect(screen.getByText(/14 \/ 20/)).toBeInTheDocument()
  })
})
