/**
 * The global banner rendered a healthy platform when nothing had answered
 * (AGENT-012 §11, §14; §19.6; Set S task S-6.21).
 *
 * A track ago the Monitor *page's* badge grew a third word for exactly this:
 * `kill.data?.kill_switch_active === true` made a failed read render a green
 * **armed**. The chrome the operator actually walks past kept the defect in its
 * other shape — `killSwitchTripped(undefined)` is `false`, and the banner
 * returned `null` on it. The absence of a banner *is* the claim that trading is
 * fine, made on every route, from a read that never came back.
 *
 * `optional()` only folds 404/405/501 into `null` and rethrows everything else,
 * so both halves have to be covered: `null` from a route that says it is not
 * there, and `undefined` from a 500, a 401 or a dropped connection.
 *
 * The one thing that must stay silent is the first read while it is in flight.
 * A banner that flashes on every page load is a banner the operator learns to
 * look past, and then the real one has no job left.
 */
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { MemoryRouter } from 'react-router-dom'
import { describe, expect, it, vi } from 'vitest'

import { KillSwitchBanner, killSwitchTripped, killSwitchView } from '../KillSwitchBanner'

const killSwitch = vi.hoisted(() => vi.fn())
vi.mock('@/api/desk', () => ({ deskApi: { killSwitch } }))

interface State {
  trading_enabled: boolean
  kill_switch_active: boolean
  scope?: string | null
}

const armed: State = { trading_enabled: true, kill_switch_active: false }
const halted: State = { trading_enabled: false, kill_switch_active: true, scope: 'global' }

describe('a kill-switch read that did not answer is its own state', () => {
  it('does not call a settled, empty read silent', () => {
    for (const nothing of [null, undefined]) {
      expect(killSwitchView({ data: nothing, isPending: false })).toEqual({ kind: 'unknown' })
    }
  })

  it('stays silent while the first read is still in flight', () => {
    expect(killSwitchView({ data: undefined, isPending: true })).toEqual({ kind: 'silent' })
  })

  it('still says halted when the endpoint answered that it is', () => {
    expect(killSwitchView({ data: halted, isPending: false })).toEqual({ kind: 'halted', state: halted })
  })

  it('stays silent on an answer that trading is running', () => {
    expect(killSwitchView({ data: armed, isPending: false })).toEqual({ kind: 'silent' })
  })

  it('treats trading_enabled: false as a halt even without the active flag', () => {
    // The wire carries two fields and either one is a halt; the reading
    // `killSwitchTripped` already had has to survive the rewrite around it.
    const disabled: State = { trading_enabled: false, kill_switch_active: false }
    expect(killSwitchTripped(disabled)).toBe(true)
    expect(killSwitchView({ data: disabled, isPending: false }).kind).toBe('halted')
  })
})

/**
 * Rendered as well as computed, because "renders nothing" was the whole defect
 * and a view function agreeing with itself would not have caught it.
 */
function mount() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter>
        <div data-testid="chrome">
          <KillSwitchBanner />
        </div>
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe('the banner shows the unknown state instead of nothing', () => {
  it('names the endpoint that did not answer, and does not claim a trip', async () => {
    killSwitch.mockRejectedValue(new Error('500'))
    mount()
    const banner = await screen.findByTestId('kill-switch-unknown-banner')
    expect(banner.textContent).toMatch(/Trading state unknown/)
    expect(banner.textContent).toMatch(/not the same as trading being armed/)
    // Not the red one either: an unknown state is not a trip.
    expect(screen.queryByTestId('kill-switch-banner')).toBeNull()
  })

  it('treats a route that says it is not there the same way', async () => {
    // `optional()` turns 404/405/501 into `null`, which is not an answer about
    // trading.
    killSwitch.mockResolvedValue(null)
    mount()
    await screen.findByTestId('kill-switch-unknown-banner')
  })

  it('shows the halted banner, undismissable, when the switch is tripped', async () => {
    killSwitch.mockResolvedValue(halted)
    mount()
    const banner = await screen.findByTestId('kill-switch-banner')
    expect(banner.textContent).toMatch(/Trading is halted/)
    // §19.6: there is no dismiss control on this component at all.
    expect(banner.querySelector('button')).toBeNull()
  })

  it('renders nothing once the platform answers that trading is running', async () => {
    killSwitch.mockResolvedValue(armed)
    mount()
    // The chrome is the anchor: without it these two absences would also pass
    // against a component that never mounted.
    expect(screen.getByTestId('chrome')).toBeInTheDocument()
    await waitFor(() => expect(killSwitch).toHaveBeenCalled())
    await waitFor(() => {
      expect(screen.queryByTestId('kill-switch-banner')).toBeNull()
      expect(screen.queryByTestId('kill-switch-unknown-banner')).toBeNull()
    })
  })
})
