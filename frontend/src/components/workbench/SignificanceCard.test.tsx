// INV-3: the significance card renders p ⊕ null ⊕ trial count inseparably, or an
// explicit empty state — there is no path that renders a bare p-value.
import { describe, it, expect } from 'vitest'
import { render, screen } from '@testing-library/react'
import { SignificanceCard } from './SignificanceCard'
import type { SignificanceView } from '@/api/experiments'

const full: SignificanceView = {
  p_value: 0.012,
  null_id: 'null:abc123',
  null_kind: 'block_permutation',
  preserves: ['marginal return distribution'],
  destroys: ['signal-return dependence'],
  trial_count_at_eval: 1320,
  raw_p_value: 0.004,
  deflated_sharpe: 0.97,
  pbo: 0.1,
  corroborators_agree: true,
}

describe('SignificanceCard (INV-3)', () => {
  it('renders the empty state and no p-value when significance is null', () => {
    render(<SignificanceCard significance={null} />)
    expect(screen.getByText(/not yet significance-tested/i)).toBeInTheDocument()
    // No numeric p-value is shown anywhere in the empty state.
    expect(screen.queryByText(/^0\.\d+$/)).not.toBeInTheDocument()
  })

  it('renders the p-value only alongside its null and trial count', () => {
    render(<SignificanceCard significance={full} />)
    // The p-value is present...
    expect(screen.getByText('0.0120')).toBeInTheDocument()
    // ...and so are its inseparable companions: the null and the trial count.
    expect(screen.getByText('block_permutation')).toBeInTheDocument()
    expect(screen.getByText('1,320')).toBeInTheDocument()
    expect(screen.getByText(/null:abc123/)).toBeInTheDocument()
    // The null's hypothesis travels with it.
    expect(screen.getByText(/signal-return dependence/)).toBeInTheDocument()
  })

  it('shows an investigate badge when corroborators disagree', () => {
    render(<SignificanceCard significance={{ ...full, corroborators_agree: false }} />)
    expect(screen.getByText(/investigate/i)).toBeInTheDocument()
  })
})
