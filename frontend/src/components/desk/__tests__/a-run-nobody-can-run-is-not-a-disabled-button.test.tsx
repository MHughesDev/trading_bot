/**
 * Two controls on the Models page offered to run a measurement nothing on this
 * platform can run (AGENT-012 §10, AGENT-009 §4; Set S task S-6.20).
 *
 * `ProfileCard` drew **Run scorecard** and **Run canary** as `disabled` buttons
 * whose only explanation lived in a `title` attribute. A `title` is a hover
 * tooltip: absent on a touch screen, absent until focus lands for a keyboard,
 * and absent for the person who looked at the card and concluded the feature
 * was broken rather than unbuilt. A disabled control is a promise that some
 * condition will clear and the button will work. There is no condition here.
 *
 * Checked before deleting rather than assumed: no route in `crates/api/src`
 * runs either one. `routes/mod.rs` mounts `/api/agent/profiles` as a GET and
 * nothing beside it, and a search for `scorecard` or `canary` across
 * `crates/api/src/routes` returns one sentence of prose inside
 * `research_sessions.rs`. The nearest route that does exist,
 * `POST /api/models/{id}/versions/{v}/evaluate`, belongs to the ML model studio
 * and evaluates a *trading* model version by registry id — pointing a capability
 * profile's button at it would measure a different object and file the answer
 * under this profile's name, which is worse than the dead button it replaced.
 *
 * So the controls are gone and the card says why in prose, where it is read.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import type { AgentProfile } from '@/api/agent'

vi.mock('@/api/agent', async (orig) => ({
  ...(await orig<typeof import('@/api/agent')>()),
}))

const { ProfileCard } = await import('../Models')

function profile(over: Partial<AgentProfile> = {}): AgentProfile {
  return {
    model_id: 'llama3:8b',
    provider: 'ollama',
    tier: 'degraded',
    tool_calling: 'native',
    max_steps: 6,
    is_local: true,
    installed: true,
    resident: false,
    multi_step_evidence: null,
    ...over,
  } as AgentProfile
}

describe('a measurement nothing can run is not a button waiting for its turn', () => {
  it('draws no scorecard or canary control at all', () => {
    render(<ProfileCard p={profile()} />)
    expect(screen.queryByRole('button', { name: /Run scorecard/i })).toBeNull()
    expect(screen.queryByRole('button', { name: /Run canary/i })).toBeNull()
  })

  it('says why in prose rather than in a tooltip nobody hovers', () => {
    render(<ProfileCard p={profile()} />)
    const said = screen.getByTestId('no-scorecard-runner').textContent ?? ''
    expect(said).toContain('no route serves either')
    // The condition it used to imply — that the button will work once something
    // lands — is stated as the absent thing it is.
    expect(said).toContain('AGENT-009')
  })

  it('leaves no disabled control on the card for a reader to misread', () => {
    // Any `disabled` button here would be the shape this change removed. The
    // provider cards have their own (a Connect with no key typed), so this
    // asserts on the profile card alone.
    const { container } = render(<ProfileCard p={profile()} />)
    expect(container.querySelectorAll('button[disabled]')).toHaveLength(0)
  })

  it('still renders `unmeasured` for a profile with no calibration row', () => {
    // §10's acceptance, which the deletion must not have disturbed: a default
    // in place of a measurement is the failure this page exists to avoid.
    render(<ProfileCard p={profile()} />)
    expect(screen.getByText(/unmeasured — no scorecard row exists/)).toBeInTheDocument()
  })
})
