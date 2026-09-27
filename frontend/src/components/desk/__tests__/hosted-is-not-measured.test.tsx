/**
 * Every hosted profile was declared measured for being hosted (AGENT-012 §10;
 * Set S task S-6.20).
 *
 * The task's acceptance is one sentence — "a profile with no calibration row
 * renders `unmeasured` rather than a default" — and the page passed
 * `measured={measured || !p.is_local}` to the tier badge. So the badge on a
 * frontier profile with no scorecard row said nothing, while three lines below
 * it the same card printed "calibration: unmeasured — no scorecard row exists
 * for this profile".
 *
 * A card that contradicts itself is not a draw: the badge is what a person reads
 * at a glance, and `unmeasured` is a warning that only means anything if it
 * appears when it is true. Whether a model is hosted is an argument about
 * whether it *needs* measuring; it is not evidence that it was.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { AgentProfile } from '@/api/agent'
import { ProfileCard } from '../Models'
// One copy, in `Shared.helpers`. There were three before the sweep.
import { profileIsMeasured } from '../Shared.helpers'

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

describe('a profile is measured when it has a calibration row and not otherwise', () => {
  it('does not call a hosted profile measured', () => {
    expect(profileIsMeasured(profile())).toBe(false)
  })

  it('does not call a local one measured either, which was already right', () => {
    expect(profileIsMeasured(profile({ is_local: true }))).toBe(false)
  })

  it('calls a profile with a scorecard row measured, wherever it runs', () => {
    const evidence = {
      eval: 'multi_step_v3',
      device: '1080 Ti',
      trials: 20,
      clean: 19,
      recorded: '2026-09-01',
    }
    expect(profileIsMeasured(profile({ multi_step_evidence: evidence }))).toBe(true)
    expect(profileIsMeasured(profile({ is_local: true, multi_step_evidence: evidence }))).toBe(true)
  })
})

describe('the badge and the calibration line say the same thing', () => {
  it('warns unmeasured on a hosted profile with no scorecard row', () => {
    render(<ProfileCard p={profile()} />)
    expect(screen.getByText('unmeasured')).toBeInTheDocument()
    // The line the badge used to disagree with, still there.
    expect(screen.getByText(/no scorecard row exists for this profile/)).toBeInTheDocument()
  })

  it('drops the warning once there is a row to point at', () => {
    render(
      <ProfileCard
        p={profile({
          multi_step_evidence: {
            eval: 'multi_step_v3',
            device: 'hosted',
            trials: 20,
            clean: 20,
            recorded: '2026-09-01',
          },
        })}
      />,
    )
    expect(screen.queryByText('unmeasured')).toBeNull()
    // …and the card rendered, so the absence above is the badge being right
    // rather than nothing having drawn.
    expect(screen.getByText('claude-opus-5')).toBeInTheDocument()
    expect(screen.getByText(/20\/20 clean/)).toBeInTheDocument()
  })
})
