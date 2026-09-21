/**
 * Live, paper, and never traded — three readings, and the third is the one that
 * gets folded into the safe one (AGENT-012 §2.4, §11; COMP-007).
 *
 * `GET /api/monitor/strategies` now sends `environment` per row, read from
 * `orders.execution_mode` — a fact recorded by `execution::journal` on each
 * order, not a strategy's name pattern-matched into an environment. The desk
 * read `strategy_id`, `state` and `scale_pct` and dropped it, so §2.4's
 * live-versus-paper column was missing from both the book panel and the
 * monitor's table.
 *
 * The route is careful about one thing and this file is the other half of it: a
 * strategy that has placed **no order** is absent from the map, and the route
 * serves `null` rather than `paper`. `null` is *unknown*. A strategy that has
 * never traded rendered as paper is a reassurance nobody measured — the same
 * shape as a scale of `null` rendered as 0% and an empty budget bar read as
 * nothing spent, both of which this desk has already been caught doing.
 *
 * `mixed` is the fourth word: a strategy with orders in more than one mode, which
 * the route refuses to collapse because both answers are true.
 */
import { render as rtlRender, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import type { ReactElement } from 'react'
import { describe, expect, it, vi } from 'vitest'

import { environmentBadge, monitorRow } from '../Operations'
import { TheBook } from '../DeskHome'

vi.mock('@/api/desk', () => ({ deskApi: { haltAll: vi.fn(() => Promise.resolve({ data: {} })) } }))

const kill = { kill_switch_active: false }

function render(ui: ReactElement) {
  return rtlRender(<MemoryRouter>{ui}</MemoryRouter>)
}

describe('the row carries the environment the orders were placed in', () => {
  it('reads it under the name the route serves it with', () => {
    expect(monitorRow({ strategy_id: 'st_1', environment: 'paper' }).environment).toBe('paper')
    expect(monitorRow({ strategy_id: 'st_1', environment: 'mixed' }).environment).toBe('mixed')
  })

  it('is null when the strategy has placed no order, and null is not paper', () => {
    expect(monitorRow({ strategy_id: 'st_1' }).environment).toBeNull()
    expect(monitorRow({ strategy_id: 'st_1', environment: null }).environment).toBeNull()
    expect(monitorRow({ strategy_id: 'st_1', environment: '' }).environment).toBeNull()
  })
})

describe('the three states read as three different things', () => {
  it('says unknown for a strategy nothing has traded, and never the word paper', () => {
    const unknown = environmentBadge(null)
    expect(unknown.word).toContain('unknown')
    expect(unknown.word).not.toContain('paper')
    expect(unknown.word).not.toContain('live')
  })

  it('says paper quietly and live loudly, because one of them is real money', () => {
    expect(environmentBadge('paper')).toEqual({ word: 'paper', tone: 'neutral' })
    expect(environmentBadge('live').word).toBe('live')
    expect(environmentBadge('live').tone).toBe('neg')
  })

  it('keeps mixed as its own answer rather than picking one of the two', () => {
    expect(environmentBadge('mixed').word).toContain('mixed')
    expect(environmentBadge('mixed').tone).toBe('warn')
  })

  it('shows a word this build does not know as it was sent', () => {
    // The CHECK on `orders.execution_mode` is `paper`-only today and widening it
    // is a migration. When it widens, the new word reaches the screen without a
    // change here — and it must not arrive disguised as one of the old ones.
    expect(environmentBadge('sandbox').word).toBe('sandbox')
    expect(environmentBadge('sandbox').tone).toBe('warn')
  })
})

describe('the book panel renders all three, distinctly', () => {
  const rows = [
    monitorRow({ strategy_id: 'st_paper', state: 'nominal', scale_pct: 100, environment: 'paper' }),
    monitorRow({ strategy_id: 'st_both', state: 'nominal', scale_pct: 100, environment: 'mixed' }),
    monitorRow({ strategy_id: 'st_new', state: 'nominal', scale_pct: 100 }),
  ]

  it('gives each row its own word', () => {
    render(<TheBook rows={rows} loading={false} kill={kill} killLoading={false} />)
    expect(screen.getByTestId('book-env-st_paper').textContent).toBe('paper')
    expect(screen.getByTestId('book-env-st_both').textContent).toContain('mixed')
    expect(screen.getByTestId('book-env-st_new').textContent).toContain('unknown')
  })

  it('does not let the strategy that has never traded read as the paper one', () => {
    render(<TheBook rows={rows} loading={false} kill={kill} killLoading={false} />)
    const never = screen.getByTestId('book-env-st_new').textContent ?? ''
    expect(never).not.toContain('paper')
    expect(never).not.toBe(screen.getByTestId('book-env-st_paper').textContent)
  })
})
