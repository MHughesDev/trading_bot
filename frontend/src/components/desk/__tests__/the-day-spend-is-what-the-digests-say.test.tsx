/**
 * S-11.13: the day budget bar's spend.
 *
 * No route counts dollars against a day. The only dollar figure is each
 * digest's `budget.spent`, in the unit the row records. So the bar's spend is
 * the sum of today's `usd` digests, and `null` — "not recorded", never zero —
 * for anything else: a digest in trials, one with no unit, a digest from
 * another day, or no digests at all.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { SessionDigestRow } from '@/api/desk'
import { daySpendFromDigests } from '@/lib/desk/sessions'
import { DayBudgetBar } from '../Shared'

const NOW = new Date('2026-10-03T15:00:00Z')

function row(spent: number, unit: string | null, at = '2026-10-03T09:00:00Z', id = 'd'): SessionDigestRow {
  return {
    digest_id: id,
    from_seq: 1,
    to_seq: 2,
    created_at: at,
    spend_unit: unit,
    body: { budget: { runs_finished: 1, spent, knowledge_units: 1, cost_per_knowledge_unit: 1 } },
  }
}

describe('daySpendFromDigests', () => {
  it('sums today\'s usd digests', () => {
    expect(daySpendFromDigests([row(2.5, 'usd', undefined, 'a'), row(1, 'usd', undefined, 'b')], NOW)).toBe(3.5)
  })
  it('is null, not zero, when nothing today is in dollars', () => {
    expect(daySpendFromDigests([row(9, 'trials')], NOW)).toBeNull()
    expect(daySpendFromDigests([row(9, null)], NOW)).toBeNull()
    expect(daySpendFromDigests([row(9, 'usd', '2026-10-02T09:00:00Z')], NOW)).toBeNull()
    expect(daySpendFromDigests([], NOW)).toBeNull()
    expect(daySpendFromDigests(undefined, NOW)).toBeNull()
  })
  it('draws the bar and says where the figure came from', () => {
    render(<DayBudgetBar usdPerDay={25} spentUsd={3.5} basis="digests" />)
    expect(screen.getByTestId('day-budget-bar').textContent).toContain('$3.50 / $25.00/day · from digests')
  })
})
