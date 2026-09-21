/**
 * The agenda board renders an item the scorer has not reached (Set S task
 * S-11.1, S-11.14).
 *
 * The board was written against the finished shape — every item carrying a
 * `score` with five numbers — and the first server to serve it does not have a
 * scorer yet (S-11.3). `i.score.value` on a null score is a crash, and it is the
 * crash that would have happened on the very first continuous session anybody
 * started: a caller and a callee that agree on the path and disagree about what
 * comes back.
 *
 * So these test the two things the real first response has: an unscored item,
 * and a status the board's five columns do not have a place for.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { AgendaItem } from '@/api/desk'
import { AgendaTab } from '../RightPanel'

function item(over: Partial<AgendaItem> = {}): AgendaItem {
  return {
    item_id: 'i_1',
    question: 'does momentum survive costs?',
    workflow: 'investigate',
    sources: ['charter_seed'],
    score: null,
    state: 'queued',
    ...over,
  }
}

describe('AgendaTab', () => {
  it('renders an item with no score instead of crashing on it', () => {
    render(<AgendaTab items={[item()]} />)
    expect(screen.getByText('does momentum survive costs?')).toBeTruthy()
    expect(screen.getByText(/not scored yet/)).toBeTruthy()
  })

  it('says why a user item goes next, rather than showing it as worthless', () => {
    render(<AgendaTab items={[item({ sources: ['user'], bypasses_scoring: true })]} />)
    expect(screen.getByText(/you asked for this one, so it goes next/)).toBeTruthy()
  })

  it('shows the true status when the column does not say it', () => {
    // `expired` and a verdict both sit in `done`. A reader has to be able to
    // tell them apart, and the column alone cannot.
    render(<AgendaTab items={[item({ state: 'done', status: 'expired' })]} />)
    expect(screen.getByText('expired')).toBeTruthy()
  })

  it('stays quiet when the status and the column are the same word', () => {
    render(<AgendaTab items={[item({ state: 'queued', status: 'queued' })]} />)
    // The column heading reads "queued · 1", so an exact match for the bare
    // word can only be the status badge — and there should not be one, because
    // repeating the column heading back at the reader says nothing.
    expect(screen.queryAllByText('queued').length).toBe(0)
    // The badge does appear when the two differ, which the test above covers;
    // this asserts the heading is still there so a zero here cannot be a board
    // that rendered nothing at all.
    expect(screen.getByText(/queued · 1/)).toBeTruthy()
  })

  it('still renders the five numbers when the scorer has run', () => {
    render(
      <AgendaTab
        items={[
          item({
            score: { value: 1, novelty: 0.6, feasibility: 1, cost: 2.1, total: 0.29 },
          }),
        ]}
      />,
    )
    expect(screen.getByText(/novelty 0\.6/)).toBeTruthy()
    expect(screen.queryByText(/not scored yet/)).toBeNull()
  })
})
