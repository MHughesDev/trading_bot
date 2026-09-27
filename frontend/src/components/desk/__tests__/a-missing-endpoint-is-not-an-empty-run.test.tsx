/**
 * The default tab of the desk read a route that is not mounted, and said the
 * run had nothing (AGENT-012 §3.5, §17; Set S tasks S-6.8, S-6.9).
 *
 * `deskApi.state` calls `/api/agent/runs/{id}/state`. That route does not exist:
 * `crates/api/src/routes/mod.rs` registers `/api/agent/runs`,
 * `/api/agent/runs/{id}`, `…/{id}/messages` and `…/{id}/cancel`, and nothing
 * else under that prefix — an unauthenticated GET against the running platform
 * answers **404**, where every mounted route answers 401. `optional()` folded
 * that 404 into `null`, and `null` was also what "this run has no snapshot yet"
 * looked like, so the State tab printed:
 *
 *   > No working state yet — it appears as soon as the run's first step lands.
 *
 * over every conversation the platform has ever had. It is a promise about a
 * step, made on a platform where no step can keep it. The Evidence tab inherited
 * it exactly, because its entries are the working state's `evidence`: it said
 * **"This run has produced no ids yet"** over runs with eighty-nine recorded
 * tool calls.
 *
 * Both sentences are claims about the run. The truth was a claim about the desk.
 *
 * What this file does **not** assert, deliberately: that the panel rebuilds the
 * state from the cards beside it. §3.5 says the State tab renders the latest
 * snapshot "not by replaying cards", and a reconstruction is a second answer
 * that can disagree with the agent's — which is how the surfaces this desk
 * replaced ended up showing a state the agent did not hold.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { EvidenceTab, StateTab } from '../RightPanel'
import { absenceCopy } from '@/components/desk/RightPanel.helpers'

describe('the State tab says why it has no state', () => {
  it('names the unmounted route rather than promising a step that will never land', () => {
    render(<StateTab state={null} absence="not_served" />)
    expect(screen.getByText(/Nothing on this platform reports a working state/)).toBeInTheDocument()
    expect(screen.getByText(/is not mounted/)).toBeInTheDocument()
    // The sentence that was there before, and that told the reader to wait.
    expect(screen.queryByText(/as soon as the run's first step lands/)).toBeNull()
  })

  it('does not blame the platform for a conversation that has not started a run', () => {
    render(<StateTab state={null} absence="no_run" />)
    expect(screen.getByText(/This conversation has no run/)).toBeInTheDocument()
    expect(screen.queryByText(/not mounted/)).toBeNull()
  })

  it('says the least when nobody told it why, rather than guessing either way', () => {
    // A caller that does not know the reason must not get a confident one. This
    // is the branch the existing state-panel test renders, and it stays vague.
    render(<StateTab state={null} />)
    expect(screen.getByText(/No working state yet/)).toBeInTheDocument()
    expect(screen.getByText(/Nothing here says why there is none/)).toBeInTheDocument()
    expect(screen.queryByText(/not mounted/)).toBeNull()
  })

  it('gives three different sentences, so none of them can be the default', () => {
    const three = [absenceCopy('not_served'), absenceCopy('no_run'), absenceCopy(undefined)]
    expect(new Set(three.map((c) => c.message)).size).toBe(3)
    expect(new Set(three.map((c) => c.detail)).size).toBe(3)
  })
})

describe('the Evidence tab does not report the run produced nothing', () => {
  it('says it has no source when the state route is the reason the list is empty', () => {
    render(<EvidenceTab entries={[]} absence="not_served" />)
    expect(screen.getByText(/Nothing reports what this run produced/)).toBeInTheDocument()
    // The claim about the run that this replaces.
    expect(screen.queryByText(/This run has produced no ids yet/)).toBeNull()
  })

  it('still claims an empty run when the caller knows the state was read', () => {
    // The branch must survive: when the snapshot arrives and carries no
    // evidence, "no ids yet" is true and is the right thing to print.
    render(<EvidenceTab entries={[]} />)
    expect(screen.getByText(/This run has produced no ids yet/)).toBeInTheDocument()
  })

  it('renders the ids when there are ids, so the absences are not the only branch', () => {
    render(
      <EvidenceTab
        entries={[{ id: 'exp_7f0c', type: 'exp', summary: 'produced by run_backtest at step 2' }]}
        resolve={() => ({ label: 'an experiment', kind: 'experiment' })}
      />,
    )
    expect(screen.getByTestId('ev-exp_7f0c')).toHaveAttribute('data-resolved', 'true')
    expect(screen.queryByText(/Nothing reports what this run produced/)).toBeNull()
  })

  it('still strikes through an id that resolves to nothing', () => {
    // §6.3's honesty check, asserted here too because this file changes the tab
    // that hosts it: a fabricated id must never render like a real one.
    render(
      <EvidenceTab entries={[{ id: 'exp_ghost', type: 'exp', summary: 'claimed' }]} resolve={() => null} />,
    )
    expect(screen.getByTestId('ev-exp_ghost')).toHaveAttribute('data-resolved', 'false')
    expect(screen.getByText('exp_ghost').tagName.toLowerCase()).toBe('s')
  })
})
