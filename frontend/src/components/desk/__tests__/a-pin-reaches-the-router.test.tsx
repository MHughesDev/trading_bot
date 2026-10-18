/**
 * "Pin for this run" is a control with a route behind it (AGENT-012 §3.5;
 * Set S task S-6.11).
 *
 * The button was removed on 2026-09-21 because it took a handler nobody passed
 * and no route recorded a pin. On 2026-10-04 the platform grew both halves:
 * `POST /api/agent/runs/{id}/pins` queues the tool on the run's control handle,
 * the local driver feeds it to the loop as `Input::Pin` at a step boundary, and
 * the loop's router reads human pins ahead of everything else.
 *
 * So the rule these cases hold is the old one turned round: a button exists
 * exactly where something is behind it. With a live run (an `onPin`), each
 * visible tool has the button and a click reaches the route with that tool's
 * name; with no run there is no button and the panel says why.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import type { ToolCard } from '@/api/desk'
import { ToolsTab } from '../RightPanel'

const tools: ToolCard[] = [
  { name: 'get_bars', namespace: 'data', risk: 'read', spends_trial: false, ring: 'research', notes: 'OHLCV over a window' },
  { name: 'run_backtest', namespace: 'backtest', risk: 'write', spends_trial: true, ring: 'research', notes: null },
]

describe('the Tools tab pins into a live run', () => {
  it('draws no pin control without a run, and says why', () => {
    render(<ToolsTab mode="research" exposed={['get_bars']} catalogue={tools} searchedFor={[]} />)
    expect(screen.queryByRole('button', { name: /pin for this run/i })).toBeNull()
    expect(screen.getByTestId('no-pin-control')).toHaveTextContent(/no live run/i)
  })

  it('does not draw the explanation over a catalogue that could not be read', () => {
    render(<ToolsTab mode="research" exposed={null} catalogue={null} searchedFor={[]} />)
    expect(screen.getByTestId('catalogue-unavailable')).toBeInTheDocument()
    expect(screen.queryByTestId('no-pin-control')).toBeNull()
  })

  it('sends the clicked tool by name and says it is queued, not available', async () => {
    const onPin = vi.fn().mockResolvedValue({ data: { pinned: true } })
    render(<ToolsTab mode="research" exposed={['get_bars']} catalogue={tools} searchedFor={[]} onPin={onPin} />)
    const buttons = screen.getAllByRole('button', { name: /pin for this run/i })
    expect(buttons).toHaveLength(2)
    fireEvent.click(buttons[1])
    expect(onPin).toHaveBeenCalledTimes(1)
    expect(onPin).toHaveBeenCalledWith('run_backtest')
    await waitFor(() => expect(screen.getByTestId('pin-queued-run_backtest')).toBeInTheDocument())
    expect(screen.getByTestId('pin-queued-run_backtest')).toHaveTextContent(/queued/i)
    // A pin posted twice is one pin; the button is spent.
    expect(buttons[1]).toBeDisabled()
    expect(screen.queryByTestId('no-pin-control')).toBeNull()
  })

  it('names the reason when the platform refuses (409: not driving this run)', async () => {
    const onPin = vi.fn().mockRejectedValue({ response: { status: 409 } })
    render(<ToolsTab mode="research" exposed={null} catalogue={tools} searchedFor={[]} onPin={onPin} />)
    fireEvent.click(screen.getAllByRole('button', { name: /pin for this run/i })[0])
    await waitFor(() => expect(screen.getByTestId('pin-failed-get_bars')).toBeInTheDocument())
    expect(screen.getByTestId('pin-failed-get_bars')).toHaveTextContent(/not driving/i)
    expect(screen.queryByTestId('pin-queued-get_bars')).toBeNull()
  })
})
