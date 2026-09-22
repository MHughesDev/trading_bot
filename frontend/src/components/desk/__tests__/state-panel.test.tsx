// §19.1: a running turn is understandable from the State tab alone.
//
// "Understandable" is not directly testable, so this asserts the five things the
// acceptance names as its content — goal kind, current step, evidence count,
// budget, and what it is waiting on — are all present **without opening a card**,
// and that the panel never invents any of them when the state does not carry them.
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { WorkingState } from '@/api/desk'
import { emptyWorkingState } from '@/api/desk'
import { StateTab, waitingOn } from '../RightPanel'
import { BudgetMeter } from '../Shared'

function runningState(): WorkingState {
  return {
    ...emptyWorkingState('does the overnight gap on BTC pay after costs?'),
    kind: 'research',
    mode: 'research',
    workflow: 'falsify',
    ring: 'research',
    stage: 'run',
    acceptance: [{ text: 'a verdict against the sealed delta_practical', met: null }],
    hypothesis: {
      family_id: 'fam_gap',
      statement: 'the overnight gap mean-reverts',
      falsifier: 'the 95% CI of the edge includes zero after costs',
      role: 'primary',
      status: 'open',
    },
    plan: [
      { goal: 'pull the bars', kind: { kind: 'tool', name: 'get_bars' }, status: { status: 'done' } },
      { goal: 'sweep the window', kind: { kind: 'tool', name: 'run_sweep' }, status: { status: 'active' } },
      { goal: 'seal the verdict', kind: { kind: 'tool', name: 'finish_task' }, status: { status: 'pending' } },
    ],
    evidence: [
      { id: 'exp_1', tool: 'run_backtest', step: 2 },
      { id: 'exp_2', tool: 'run_backtest', step: 2 },
    ],
    findings: [{ id: 'f_1', headline: 'the gap is 3bp before costs', step: 2 }],
    budget: {
      steps_remaining: 6,
      steps_total: 20,
      tokens_cap: null,
      tokens_remaining: 40_000,
      usd_tenth_cents_cap: null,
      usd_tenth_cents_remaining: 12_000,
      trials_cap: null,
      trials_spent: 4,
      n_eff: 2.4,
    },
  }
}

describe('the State tab answers the five questions on its own', () => {
  it('names the goal kind, the stage, the current step, the evidence count and what it waits on', () => {
    const s = runningState()
    render(<StateTab state={s} />)
    expect(screen.getByText(s.goal)).toBeInTheDocument()
    expect(screen.getByText('research')).toBeInTheDocument() // goal kind
    expect(screen.getByText('falsify')).toBeInTheDocument() // workflow
    expect(screen.getByText('ring: research')).toBeInTheDocument()
    expect(screen.getByLabelText('lifecycle stage: run')).toBeInTheDocument()
    expect(screen.getByText(/plan · step 2 of 3/)).toBeInTheDocument()
    expect(screen.getByText('evidence · 2')).toBeInTheDocument()
    expect(screen.getByText('findings · 1')).toBeInTheDocument()
    expect(screen.getByText(/running step: sweep the window/)).toBeInTheDocument()
    expect(screen.getByText(/n_eff 2\.4/)).toBeInTheDocument()
  })

  it('says what it is waiting on in the words a person would use', () => {
    const s = runningState()
    expect(waitingOn(s, {})).toBe('running step: sweep the window')
    expect(waitingOn(s, { paused: true })).toContain('paused')
    expect(waitingOn(s, { pendingApproval: true })).toContain('approve or deny')
    expect(waitingOn(s, { jobPct: 34 })).toContain('34%')
  })

  it('invents nothing when there is no snapshot', () => {
    render(<StateTab state={null} />)
    expect(screen.getByText(/No working state yet/)).toBeInTheDocument()
  })

  it('prints the budget as text, never as colour alone', () => {
    const s = runningState()
    render(<BudgetMeter budget={s.budget} />)
    // Four bars, each with its number beside it.
    expect(screen.getAllByRole('meter').length).toBe(4)
    expect(screen.getByText(/14 \/ 20/)).toBeInTheDocument()
    expect(screen.getByText(/n_eff 2\.4/)).toBeInTheDocument()
  })

  it('says n_eff is not computed rather than showing a zero', () => {
    const s = runningState()
    s.budget.n_eff = null
    render(<BudgetMeter budget={s.budget} />)
    expect(screen.getByText(/n_eff \(not computed\)/)).toBeInTheDocument()
  })
})
