/**
 * Desk home's *book* panel, and the tile beside it that called every hosted
 * profile calibrated (AGENT-012 §2.2, §2.4; Set S tasks S-6.14, S-6.20).
 *
 * Three defects, one shape.
 *
 * 1. **A failed monitor read explained itself as an unbuilt phase.** The panel
 *    rendered "The monitor is not built yet — COMP-007 lands with Set S phase 7"
 *    on `monitor.data === null || undefined`. COMP-007 landed;
 *    `/api/monitor/strategies` is served, and the Monitor page six routes away
 *    already says so in its own empty state. A false explanation is worse than
 *    none, because it tells the operator there is nothing here to look at — the
 *    same sentence that hid the served catalogue behind a phase number for days.
 *
 * 2. **§2.4 asks for control-limit status per strategy and the panel printed a
 *    count.** "3 strategies under control limits" is the same character for a
 *    strategy the monitor scaled to 10% and one running at full size. A count is
 *    not a status.
 *
 * 3. **`measured: p.multi_step_evidence !== null || !p.is_local`** — the exact
 *    expression S-6.20 removed from the Models page, live in **two** more
 *    places: desk home's tier tile and the conversation header's badge. Both
 *    printed a measured badge for every hosted profile on the strength of it
 *    being hosted. One fix travelled and two copies did not, which is the
 *    failure mode a shared function exists to prevent — so the rule now lives
 *    beside the badge it feeds and all three surfaces call it.
 */
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { render as rtlRender, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import type { ReactElement } from 'react'
import { describe, expect, it, vi } from 'vitest'

import type { AgentProfile } from '@/api/agent'
import { TheBook } from '../DeskHome'
import { monitorRow } from '@/components/desk/Operations.helpers'
import { tierTiles } from '@/components/desk/Shared.helpers'

vi.mock('@/api/desk', () => ({ deskApi: { haltAll: vi.fn(() => Promise.resolve({ data: {} })) } }))

const kill = { kill_switch_active: false }

/** The panel links to `/desk/monitor`, so it needs a router around it. */
function render(ui: ReactElement) {
  return rtlRender(<MemoryRouter>{ui}</MemoryRouter>)
}

describe('the book tells a failed monitor read from an empty monitor', () => {
  it('does not blame an unbuilt phase for a request that failed', () => {
    render(<TheBook rows={null} loading={false} kill={kill} killLoading={false} />)
    expect(screen.getByText(/The monitor did not answer/)).toBeInTheDocument()
    expect(screen.getByText(/is served \(COMP-007\)/)).toBeInTheDocument()
    // The sentence that was there before, and the phase number it pointed at.
    expect(screen.queryByText(/not built yet/)).toBeNull()
    expect(screen.queryByText(/phase 7/)).toBeNull()
  })

  it('says an empty list is a fact about the monitor, not about the panel', () => {
    render(<TheBook rows={[]} loading={false} kill={kill} killLoading={false} />)
    expect(screen.getByText(/No strategy is under the monitor/)).toBeInTheDocument()
    expect(screen.queryByText(/did not answer/)).toBeNull()
  })

  it('does not claim either while the read is still in flight', () => {
    render(<TheBook rows={null} loading kill={kill} killLoading />)
    expect(screen.queryByText(/did not answer/)).toBeNull()
    expect(screen.queryByText(/No strategy is under the monitor/)).toBeNull()
    // …and the panel drew, so the absences above are the loading branch rather
    // than a component that rendered nothing.
    expect(screen.getByText('The book')).toBeInTheDocument()
  })
})

describe('the book shows a status per strategy, not a count', () => {
  const rows = [
    monitorRow({ strategy_id: 'st_gap', state: 'nominal', scale_pct: 100 }),
    monitorRow({ strategy_id: 'st_mom', state: 'scaled_down', scale_pct: 10 }),
    monitorRow({ strategy_id: 'st_old' }),
  ]

  it('names each strategy and its control-limit state', () => {
    render(<TheBook rows={rows} loading={false} kill={kill} killLoading={false} />)
    expect(screen.getByTestId('book-rows').children).toHaveLength(3)
    expect(screen.getByText('st_gap')).toBeInTheDocument()
    expect(screen.getByText('nominal')).toBeInTheDocument()
    expect(screen.getByText('scaled_down')).toBeInTheDocument()
    // Two different strategies must not print the same thing.
    expect(screen.getByText('100% of full size')).toBeInTheDocument()
    expect(screen.getByText('10% of full size')).toBeInTheDocument()
  })

  it('keeps a scale nobody reported out of the numbers', () => {
    render(<TheBook rows={rows} loading={false} kill={kill} killLoading={false} />)
    expect(screen.getByText('scale not reported')).toBeInTheDocument()
    expect(screen.getByText('state not reported')).toBeInTheDocument()
    // Not 0%: the monitor shutting a strategy down and nobody having said are
    // opposite facts.
    expect(screen.queryByText('0% of full size')).toBeNull()
  })

  it('shows a genuine zero as a zero, which is the other half of the rule', () => {
    render(
      <TheBook
        rows={[monitorRow({ strategy_id: 'st_dead', state: 'halted', scale_pct: 0 })]}
        loading={false}
        kill={kill}
        killLoading={false}
      />,
    )
    expect(screen.getByText('0% of full size')).toBeInTheDocument()
  })

  it('carries the kill-switch state as a badge, with the unknown word', () => {
    render(<TheBook rows={[]} loading={false} kill={null} killLoading={false} />)
    expect(screen.getByTestId('book-kill-badge').textContent).toMatch(/state unknown/)
  })

  it('offers Halt all, which §2.4 asks for and the panel did not have', () => {
    render(<TheBook rows={[]} loading={false} kill={kill} killLoading={false} />)
    expect(screen.getByRole('button', { name: 'Halt all' })).toBeInTheDocument()
  })
})

function profile(over: Partial<AgentProfile> = {}): AgentProfile {
  return {
    model_id: 'claude-opus-5',
    provider: 'anthropic',
    tier: 'frontier',
    is_local: false,
    tool_calling: 'multi_step',
    max_steps: 40,
    multi_step_evidence: null,
    requires_api_key: true,
    is_default: true,
    installed: null,
    resident: null,
    backend_reachable: null,
    ...over,
  }
}

describe('every surface computes measured the same way now', () => {
  it('does not call a hosted profile measured', () => {
    const p = profile()
    // The expression the tile used to hold, spelled out so its disagreement
    // with what the page now renders is the assertion rather than a comment.
    const oldReading = p.multi_step_evidence !== null || !p.is_local
    expect(oldReading).toBe(true)
    expect(tierTiles([p])[0].measured).toBe(false)
  })

  it('does not call a local one measured either, which was already right', () => {
    expect(tierTiles([profile({ is_local: true })])[0].measured).toBe(false)
  })

  it('calls a profile with a scorecard row measured, wherever it runs', () => {
    const evidence = {
      eval: 'multi_step_v3',
      device: 'hosted',
      trials: 20,
      clean: 20,
      recorded: '2026-09-01',
    }
    expect(tierTiles([profile({ multi_step_evidence: evidence })])[0].measured).toBe(true)
    expect(
      tierTiles([profile({ is_local: true, multi_step_evidence: evidence })])[0].measured,
    ).toBe(true)
  })

  it('makes no tiles at all from a list that never arrived', () => {
    // Which is not the same as none being configured; the panel's three states
    // handle the difference and this only refuses to invent a row.
    expect(tierTiles(undefined)).toEqual([])
    expect(tierTiles([])).toEqual([])
  })
})

describe('the conversation header keeps the same rule', () => {
  it('draws its badge from the shared tiles, not its own reading', () => {
    // Asserted structurally: the badge's list is `tierTiles(...)` on all three
    // surfaces now, and the assignment they each used to hold must not come
    // back. A behavioural assertion would need the whole conversation screen
    // mounted, and the defect was a copied expression rather than a rendering.
    const here = dirname(fileURLToPath(import.meta.url))
    for (const file of ['Conversation.tsx', 'DeskHome.tsx', 'Models.tsx']) {
      const src = readFileSync(resolve(here, '..', file), 'utf8')
      expect(src.length).toBeGreaterThan(0)
      expect(src).not.toMatch(/measured:\s*p\.multi_step_evidence !== null \|\| !p\.is_local/)
    }
    // …and the shared one is reachable and gives the right answer, so the three
    // absences above are not a regex that matches nothing anywhere.
    expect(tierTiles([profile()])[0].measured).toBe(false)
  })
})
