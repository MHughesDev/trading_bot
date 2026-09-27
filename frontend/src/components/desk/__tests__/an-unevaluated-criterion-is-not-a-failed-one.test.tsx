/**
 * `met: null` means nobody has looked. It must not render as a cross, and it
 * must not render as a tick (ADR-D-25; `enum Met` in `crates/harness/src/state.rs`).
 *
 * The harness gained a third state on the wire an hour before this test was
 * written: `acceptance` is now `[{ text, met: true | false | null }]`, and
 * **`met` is `null` on every criterion of every run**, because nothing on this
 * platform evaluates an acceptance criterion. That makes the two-state rendering
 * a panel naturally falls into — ticked or not ticked — a report that every run
 * is failing everything it was asked to do, which is the loudest possible way to
 * be wrong about a run that may be going perfectly.
 *
 * The Rust type refuses to have a `bool` escape hatch for exactly this reason:
 * no `is_met()`, no `From<Met> for bool`, only a three-armed `fold`. TypeScript
 * cannot enforce that, so these are the tests that stand in for the compiler.
 *
 * Before ADR-D-25 the panel did `state.acceptance.map((a) => <li key={a}>{a}</li>)`
 * over bare strings; every test in the first block fails against that renderer,
 * and the last one fails against any renderer that reaches for `met ?? false`.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import type { WorkingState } from '@/api/desk'
import { emptyWorkingState } from '@/api/desk'
import { StateTab } from '../RightPanel'
import { acceptanceMark } from '@/components/desk/RightPanel.helpers'

function withAcceptance(acceptance: WorkingState['acceptance']): WorkingState {
  return { ...emptyWorkingState('does the overnight gap on BTC pay after costs?'), acceptance }
}

const THREE: WorkingState['acceptance'] = [
  { text: 'a verdict against the sealed delta_practical', met: null },
  { text: 'the null library was consulted', met: true },
  { text: 'the holdout was never unsealed', met: false },
]

describe('an acceptance criterion has three states and null is one of them', () => {
  it('gives an unevaluated criterion neither a tick nor a cross', () => {
    const m = acceptanceMark(null)
    expect(m.glyph).toBeNull()
    expect(m.word).toBe('not yet evaluated')
    expect(m.tone).toBe('neutral')
  })

  it('marks a measured failure as a failure, so the neutral state is not doing that job too', () => {
    expect(acceptanceMark(false).word).toBe('NOT met')
    expect(acceptanceMark(false).tone).toBe('neg')
    expect(acceptanceMark(true).word).toBe('met')
    expect(acceptanceMark(true).tone).toBe('pos')
  })

  it('reads the three states differently, so none of them is the default', () => {
    const words = [acceptanceMark(null), acceptanceMark(true), acceptanceMark(false)].map((m) => m.word)
    expect(new Set(words).size).toBe(3)
    const keys = [acceptanceMark(null), acceptanceMark(true), acceptanceMark(false)].map((m) => m.key)
    expect(new Set(keys).size).toBe(3)
  })

  it('says "not yet evaluated" in the words the harness card uses, so the two surfaces agree', () => {
    // `Met::mark()` in state.rs: "met" / "NOT met" / "not yet evaluated".
    expect(acceptanceMark(null).word).toBe('not yet evaluated')
  })
})

describe('the State tab renders the criterion, not the object it now arrives in', () => {
  it('prints the text of each criterion', () => {
    render(<StateTab state={withAcceptance(THREE)} />)
    for (const c of THREE) expect(screen.getByText(new RegExp(c.text))).toBeInTheDocument()
  })

  it('labels each row with its own state', () => {
    const { container } = render(<StateTab state={withAcceptance(THREE)} />)
    const rows = Array.from(container.querySelectorAll('li[data-met]'))
    expect(rows.map((r) => r.getAttribute('data-met'))).toEqual(['unevaluated', 'met', 'not-met'])
  })

  it('never draws a cross or a tick beside a criterion nobody evaluated', () => {
    const { container } = render(<StateTab state={withAcceptance([THREE[0]])} />)
    const row = container.querySelector('li[data-met="unevaluated"]')
    expect(row).not.toBeNull()
    expect(row?.textContent).toContain('not yet evaluated')
    expect(row?.textContent).not.toContain('✕')
    expect(row?.textContent).not.toContain('✓')
  })

  it('reports nothing as failed on a run where every criterion is unevaluated', () => {
    // The state of every run on this platform today.
    const all = THREE.map((c) => ({ ...c, met: null }))
    const { container } = render(<StateTab state={withAcceptance(all)} />)
    expect(container.querySelectorAll('li[data-met="not-met"]').length).toBe(0)
    expect(container.querySelectorAll('li[data-met="met"]').length).toBe(0)
    expect(container.querySelectorAll('li[data-met="unevaluated"]').length).toBe(3)
    expect(container.textContent).not.toContain('NOT met')
  })

  it('still says so plainly when a goal derived no criteria at all', () => {
    render(<StateTab state={withAcceptance([])} />)
    expect(screen.getByText(/No acceptance criteria were derived/)).toBeInTheDocument()
  })
})
