/**
 * The continuous session's day budget in the journal header (AGENT-014 §9;
 * Set S task S-11.13).
 *
 * §9 asks for "a **continuous** badge and the day budget bar". The badge is
 * easy; the bar is the trap. `GET /api/research/sessions/{id}` answers
 * `budget.usd_per_day` and **nothing that counts against it** — there is no
 * spend-to-date on any served route, and `agent_runs` records `trials_spent`,
 * not dollars.
 *
 * A bar drawn from a budget with no spend is an empty track beside "$25.00/day",
 * and an empty track says *nothing has been spent today*. Nobody measured that.
 * It is the same defect as the tokens bar that printed "0 / 40,000" for a run
 * that had spent half a million, and as the kill-switch banner that rendered
 * nothing on a failed read — an absent measurement rendered as the reassuring
 * one.
 *
 * So: no spend, no bar. The header says the budget and says the spend is not
 * recorded, and the bar appears the day something counts it.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { DayBudgetBar } from '../Shared'

describe('the day budget bar', () => {
  it('draws no bar when nothing counts spend, and says so', () => {
    render(<DayBudgetBar usdPerDay={25} spentUsd={null} />)
    const said = screen.getByTestId('day-budget-no-spend').textContent ?? ''
    expect(said).toContain('$25.00/day')
    expect(said).toContain('spend today is not recorded')
    // The assertion that matters: no meter at all. A meter at 0% is the lie.
    expect(screen.queryByRole('meter')).toBeNull()
  })

  it('treats an absent spend the same as an explicit null, never as zero', () => {
    render(<DayBudgetBar usdPerDay={25} />)
    expect(screen.getByTestId('day-budget-no-spend')).toBeInTheDocument()
    expect(screen.queryByText(/\$0\.00/)).toBeNull()
  })

  it('says the budget is not recorded rather than showing a bar with no denominator', () => {
    render(<DayBudgetBar usdPerDay={null} spentUsd={null} />)
    expect(screen.getByTestId('day-budget-unknown').textContent).toContain('not recorded')
    expect(screen.queryByRole('meter')).toBeNull()
  })

  it('draws the bar the moment a spend arrives', () => {
    render(<DayBudgetBar usdPerDay={25} spentUsd={5} />)
    const meter = screen.getByRole('meter')
    expect(meter.getAttribute('aria-valuenow')).toBe('20')
    expect(screen.getByTestId('day-budget-bar').textContent).toContain('$5.00 / $25.00/day')
  })

  it('shows a zero spend as a zero, because measured and zero is a real answer', () => {
    render(<DayBudgetBar usdPerDay={25} spentUsd={0} />)
    expect(screen.getByRole('meter').getAttribute('aria-valuenow')).toBe('0')
    expect(screen.queryByTestId('day-budget-no-spend')).toBeNull()
  })
})
