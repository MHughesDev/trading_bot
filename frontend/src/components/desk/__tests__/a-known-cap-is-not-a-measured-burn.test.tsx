/**
 * A ceiling and a position under it are two facts, and the desk may only draw a
 * fill when it holds both (ADR-D-25, ADR-D-08; `BudgetView` in
 * `crates/harness/src/state.rs`).
 *
 * The harness now sends `tokens_cap` — a real number, 2,000,000 or 40,000,000 —
 * alongside a `tokens_remaining` that is `null` on every run, because nothing on
 * this platform counts tokens (OQ-16). That combination did not exist in the
 * `BarValue` union before: the uncapped branch prints the remainder, so a `null`
 * one prints **"null remaining"**, and the capped branch needs a `used` that
 * would have to be invented from the only number in the room.
 *
 * Both inventions have shipped here. `cap - 0` renders a run that has burned
 * nothing as a budget fully spent; `cap = remaining` made `used` identically
 * zero and printed "0 / 40,000" for a run that had spent half a million tokens.
 * The fix is not a better guess, it is the absence of one: the track is drawn
 * because the cap is real, and no fill is drawn because nobody measured a
 * position.
 *
 * The dollar axis has neither half — no price table (ADR-D-03), no per-run
 * spend — and gets its own sentence rather than `$0.00`, which is a measurement
 * claiming the run was free.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { BudgetView } from '@/api/desk'
import { emptyWorkingState } from '@/api/desk'
import { BudgetMeter, budgetBars } from '../Shared'

/** What `/api/agent/runs/{id}/state` will answer for a live run today. */
function asSentToday(over: Partial<BudgetView> = {}): BudgetView {
  return {
    steps_remaining: 6,
    steps_total: 20,
    tokens_cap: 2_000_000,
    tokens_remaining: null,
    usd_tenth_cents_cap: null,
    usd_tenth_cents_remaining: null,
    trials_cap: null,
    trials_spent: 4,
    n_eff: null,
    ...over,
  }
}

describe('a cap with no burn is its own shape, not either of the other two', () => {
  it('keeps the cap and refuses to invent the usage', () => {
    const [, tokens] = budgetBars(asSentToday())
    expect(tokens.value).toEqual({ capped: true, used: null, cap: 2_000_000 })
  })

  it('does not read the cap as the burn, which would say the budget is spent', () => {
    const [, tokens] = budgetBars(asSentToday())
    // `used: 2_000_000` — the shape `cap - (remaining ?? 0)` produces.
    expect(tokens.value).not.toEqual({ capped: true, used: 2_000_000, cap: 2_000_000 })
  })

  it('does not read the absent burn as zero, which would say nothing is spent', () => {
    const [, tokens] = budgetBars(asSentToday())
    expect(tokens.value).not.toEqual({ capped: true, used: 0, cap: 2_000_000 })
  })

  it('still reports both halves when the run has both', () => {
    const [, tokens] = budgetBars(asSentToday({ tokens_remaining: 1_500_000 }))
    expect(tokens.value).toEqual({ capped: true, used: 500_000, cap: 2_000_000 })
  })

  it('takes a cap from the snapshot now, not only from the caller', () => {
    const [, tokens] = budgetBars(asSentToday({ tokens_remaining: 1_000_000 }))
    expect(tokens.value).toEqual({ capped: true, used: 1_000_000, cap: 2_000_000 })
  })

  it('lets a caller who was told a cap override the snapshot', () => {
    const [, tokens] = budgetBars(asSentToday({ tokens_remaining: 100 }), { tokens: 1_000 })
    expect(tokens.value).toEqual({ capped: true, used: 900, cap: 1_000 })
  })

  it('says nothing at all about an axis with neither a cap nor a reading', () => {
    const [, , money] = budgetBars(asSentToday())
    expect(money.value).toEqual({ capped: false, value: null })
  })

  it('leaves the axes that still have no cap exactly as they were', () => {
    const bars = budgetBars(asSentToday())
    expect(bars[0].value).toEqual({ capped: true, used: 14, cap: 20 })
    expect(bars[3].value).toEqual({ capped: false, value: 4, word: 'spent' })
  })
})

describe('the meter draws the cap and not a position under it', () => {
  it('names the cap and says the usage is not recorded', () => {
    render(<BudgetMeter budget={asSentToday()} />)
    expect(screen.getByText('cap 2,000,000 · usage not recorded')).toBeInTheDocument()
  })

  it('never prints the word null where a number would go', () => {
    const { container } = render(<BudgetMeter budget={asSentToday()} />)
    expect(container.textContent).not.toContain('null')
    expect(container.textContent).not.toContain('NaN')
  })

  it('draws no fill for a position nobody measured', () => {
    render(<BudgetMeter budget={asSentToday()} />)
    const tokens = screen.getByLabelText('tokens used')
    expect(tokens.querySelector('i')).toBeNull()
    expect(tokens.getAttribute('aria-valuenow')).toBeNull()
    expect(tokens.getAttribute('aria-valuetext')).toBe('cap 2,000,000; usage not recorded')
    expect(tokens.className).toContain('unknown')
  })

  it('never reports the token axis as spent or as untouched', () => {
    const { container } = render(<BudgetMeter budget={asSentToday()} />)
    expect(container.textContent).not.toContain('2,000,000 / 2,000,000')
    expect(container.textContent).not.toContain('0 / 2,000,000')
    expect(container.textContent).not.toContain('90%+')
  })

  it('says the dollar axis is not recorded rather than pricing the run at zero', () => {
    const { container } = render(<BudgetMeter budget={asSentToday()} />)
    expect(screen.getByText('not recorded')).toBeInTheDocument()
    expect(container.textContent).not.toContain('$0.00')
    const money = screen.getByLabelText('$ used')
    expect(money.querySelector('i')).toBeNull()
    expect(money.getAttribute('aria-valuenow')).toBeNull()
  })

  it('still draws four bars, so an unmeasured axis is present rather than hidden', () => {
    render(<BudgetMeter budget={asSentToday()} />)
    expect(screen.getAllByRole('meter').length).toBe(4)
    expect(screen.getByText(/14 \/ 20/)).toBeInTheDocument()
  })
})

describe('an absent measurement starts absent, because nothing downstream can restore it', () => {
  it('defaults every uncounted axis to null rather than to zero', () => {
    const b = emptyWorkingState().budget
    expect(b.tokens_cap).toBeNull()
    expect(b.tokens_remaining).toBeNull()
    expect(b.usd_tenth_cents_cap).toBeNull()
    expect(b.usd_tenth_cents_remaining).toBeNull()
    expect(b.trials_cap).toBeNull()
  })

  it('would not survive a zero, because `0 ?? x` is `0`', () => {
    const b = emptyWorkingState().budget
    // The whole reason the default is null and not 0: a later caller's fallback
    // only fires on null, so a zero here is permanent.
    expect(b.tokens_cap ?? 'nobody said').toBe('nobody said')
    const theZeroThatShippedTwice = (): number | null => 0
    expect(theZeroThatShippedTwice() ?? 'nobody said').toBe(0)
  })

  it('renders a snapshot nobody has filled in as four absences, not as a spent budget', () => {
    const { container } = render(<BudgetMeter budget={emptyWorkingState().budget} />)
    expect(container.textContent).not.toContain('90%+')
    expect(container.textContent).not.toContain('$0.00')
    expect(container.textContent).not.toContain('0 / 0')
    expect(container.querySelectorAll('.budget-track > i').length).toBe(0)
  })
})
