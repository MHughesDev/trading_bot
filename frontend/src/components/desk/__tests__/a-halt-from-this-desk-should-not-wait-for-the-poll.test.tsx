/**
 * An exported constant with the shape of a mechanism, and a comment defending it
 * (AGENT-012 §11, §14, §19.6; Set S task S-6.21).
 *
 * `KillSwitchBanner.tsx` exported `KILL_SWITCH_EVENT = 'desk:kill-switch'`.
 * Nothing in `frontend/src` dispatched it and nothing listened for it. Beside it
 * the banner's `refetchInterval` carried the sentence *"a live event refetches
 * immediately, so 'within one event' holds wherever the stream is connected"* —
 * false in both halves. There was no event. There is also no stream:
 * `useDeskStream` is per-run, and nothing on the platform pushes kill-switch
 * state to a browser at all. So that prose was load-bearing for S-6.21's entire
 * acceptance criterion and for a phase-7 exit criterion, and what it defended
 * did not exist.
 *
 * This is the `prioritisedBy` shape again: a dead export is not inert, because
 * the next caller reaches for it and believes it works.
 *
 * The constant is gone. What replaced it is what this codebase already uses for
 * exactly this — the react-query key the banner reads under — and making the
 * swap turned up the real bug the dead event was standing in front of.
 * `Operations.tsx` invalidates `['desk','kill-switch']` after its global halt;
 * **`DeskHome.tsx` did not**, so a halt from the desk-home book panel left the
 * chrome silent for up to the full three-second poll, on the one screen where
 * "trading is halted" is the only thing the operator is looking for.
 *
 * What is *not* claimed, here or in the file it guards: that the banner appears
 * within one event of a trip. It does not. A trip from the risk engine or from
 * another operator arrives by poll and by nothing else.
 */
import { fireEvent, render as rtlRender, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import type { ReactElement } from 'react'
import { describe, expect, it, vi } from 'vitest'

const haltAll = vi.hoisted(() => vi.fn(() => Promise.resolve({ data: {} })))
vi.mock('@/api/desk', () => ({ deskApi: { haltAll } }))

import * as banner from '../KillSwitchBanner'
import * as bannerHelpers from '../KillSwitchBanner.helpers'
import { TheBook } from '../DeskHome'

const kill = { kill_switch_active: false }

function render(ui: ReactElement) {
  return rtlRender(<MemoryRouter>{ui}</MemoryRouter>)
}

/** Open the confirm, type the word, press the dialog's own button. */
function confirmHalt() {
  fireEvent.click(screen.getByRole('button', { name: 'Halt all' }))
  fireEvent.change(screen.getByLabelText(/Type HALT to confirm/i), { target: { value: 'HALT' } })
  // Two buttons now carry this name — the panel's trigger and the dialog's
  // footer. The dialog's is the later one in the tree.
  const buttons = screen.getAllByRole('button', { name: 'Halt all' })
  fireEvent.click(buttons[buttons.length - 1])
}

describe('the banner exports no mechanism it does not have', () => {
  it('no longer exports an event name nobody dispatches', () => {
    expect(Object.keys(banner)).not.toContain('KILL_SWITCH_EVENT')
  })

  it('exports the query key instead, which is what actually moves the banner', () => {
    // The key lives in the helpers sibling now: the component module holds the
    // component, and everything testable without rendering moved out. The
    // assertion is unchanged in substance — a shared key that both the banner and
    // whoever halts can name, rather than an event name nobody dispatches.
    expect(bannerHelpers.KILL_SWITCH_QUERY_KEY).toEqual(['desk', 'kill-switch'])
  })
})

describe('a halt from the desk home tells the chrome that reports it', () => {
  it('signals once the halt has landed, rather than leaving it to the poll', async () => {
    haltAll.mockClear()
    const onHalted = vi.fn()
    render(<TheBook rows={[]} loading={false} kill={kill} killLoading={false} onHalted={onHalted} />)
    confirmHalt()
    await waitFor(() => expect(haltAll).toHaveBeenCalled())
    await waitFor(() => expect(onHalted).toHaveBeenCalled())
  })

  it('does not signal on a halt the monitor refused', async () => {
    // A 403 from `permits_monitor_control` leaves trading armed. Telling the
    // banner to re-read would be harmless; running the success path on a refusal
    // is how a panel comes to report a halt that never happened, so the error
    // branch is the only one that may run.
    haltAll.mockClear()
    haltAll.mockImplementationOnce(() => Promise.reject(new Error('403')))
    const onHalted = vi.fn()
    render(<TheBook rows={[]} loading={false} kill={kill} killLoading={false} onHalted={onHalted} />)
    confirmHalt()
    await waitFor(() => expect(screen.getByTestId('book-halt-error')).toBeInTheDocument())
    expect(onHalted).not.toHaveBeenCalled()
  })

  it('still halts when nobody is listening, so the signal is not the point of the button', async () => {
    haltAll.mockClear()
    render(<TheBook rows={[]} loading={false} kill={kill} killLoading={false} />)
    confirmHalt()
    await waitFor(() => expect(haltAll).toHaveBeenCalled())
  })
})
