/**
 * Every tool in the catalogue carried a **Pin for this run** button that could
 * not pin anything (AGENT-012 §3.5; Set S task S-6.11).
 *
 * Two failures, one on top of the other. `Conversation.tsx` passed the tab no
 * `onPin`, so the button was inert at the only call site — the "built but
 * unreached" shape this repository produces more than any other. And had a
 * handler been passed, there was nothing behind it: no route on the platform
 * records a per-run pin (`/api/agent/runs/{id}/pins` answers 404, and 404 here
 * means unmounted — every mounted route answers 401 unauthenticated), and the
 * harness would not read one if it did. `Driver::route` builds its `pinned` list
 * from exactly three sources:
 *
 *   1. `self.discovered_tools` — what the model's own `search_tools` returned;
 *   2. the active plan step's named tool;
 *   3. a three-hit search over that step's goal text.
 *
 * A human message is none of the three. The spec's parenthetical — "sends a
 * `search_tools`-equivalent" — describes a mechanism that does not exist:
 * `search_tools` is a tool the *model* calls and the harness answers, and there
 * is no user-initiated equivalent anywhere on the platform.
 *
 * So the control is gone and the panel names the lever that does work. This is
 * the same rule the monitor learned: a control that does not reach the thing it
 * claims to change passes every test and does nothing, and the person who clicks
 * it believes the model can now see the tool.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { ToolCard } from '@/api/desk'
import { ToolsTab } from '../RightPanel'

const tools: ToolCard[] = [
  {
    name: 'get_bars',
    namespace: 'data',
    risk: 'read',
    spends_trial: false,
    ring: 'research',
    notes: 'OHLCV over a window',
  },
  {
    name: 'run_backtest',
    namespace: 'backtest',
    risk: 'write',
    spends_trial: true,
    ring: 'research',
    notes: null,
  },
]

function panel() {
  return render(
    <ToolsTab mode="research" exposed={['get_bars']} catalogue={tools} searchedFor={[]} />,
  )
}

describe('the Tools tab offers no pin, because the platform has none', () => {
  it('draws no pin control on any tool', () => {
    panel()
    expect(screen.queryByRole('button', { name: /pin/i })).toBeNull()
    expect(screen.queryByText(/Pin for this run/)).toBeNull()
  })

  it('says so, rather than leaving the reader to wonder where the control went', () => {
    panel()
    expect(screen.getByTestId('no-pin-control')).toBeInTheDocument()
  })

  it('names the three things that actually decide the next step, and steering as the way in', () => {
    panel()
    const why = screen.getByTestId('no-pin-control')
    expect(why).toHaveTextContent(/plan step/)
    expect(why).toHaveTextContent(/goal text/)
    expect(why).toHaveTextContent(/search_tools/)
    expect(why).toHaveTextContent(/steer/i)
  })

  it('still lists the catalogue, so removing the control did not remove the tools', () => {
    panel()
    // Twice for `get_bars`: once as an exposure chip, once as a catalogue card.
    expect(screen.getAllByText('get_bars').length).toBeGreaterThan(0)
    expect(screen.getByText('run_backtest')).toBeInTheDocument()
    // And the thing a person needs before reaching for one is still on the card.
    expect(screen.getByText('spends a trial')).toBeInTheDocument()
  })

  it('does not draw the explanation over a catalogue that could not be read', () => {
    // "You cannot pin from here" is a sentence about a list of tools. With no
    // list it would be an answer to a question nobody asked, under the sentence
    // saying the tools are unknown.
    render(<ToolsTab mode="research" exposed={null} catalogue={null} searchedFor={[]} />)
    expect(screen.getByTestId('catalogue-unavailable')).toBeInTheDocument()
    expect(screen.queryByTestId('no-pin-control')).toBeNull()
  })
})
