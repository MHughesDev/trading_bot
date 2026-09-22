/**
 * The dossier struck through every id it was never given a way to check
 * (AGENT-012 §5, §6.3; Set S tasks S-6.9, S-6.17).
 *
 * `EvidenceChip` opened with `const hit = resolve ? resolve(id) : null`, and
 * then treated a falsy `hit` as the answer **no**: struck through, the words
 * "not found", and a tooltip asserting *"This id resolves to nothing the
 * platform holds."* Two inputs, one output — a resolver that looked and found
 * nothing, and no resolver at all.
 *
 * That was live on the worst surface in the app. `pages/DeskPage.tsx` renders
 * `<DossierReader ticket={ticket} />` with **no `resolve` prop**, so on the
 * dossier page — the document whose entire claim is that its evidence is
 * checkable — every evidence id in the table and every funnel ticket rendered
 * struck through and labelled not found. A reviewer reading that discards sound
 * research, which is the exact inversion of what the strike-through is for: it
 * exists to catch a *fabricated* id, and a mark that fires on every id catches
 * nothing and libels everything.
 *
 * Two surfaces, not one. `DeskPage.tsx:218` renders `<ApprovalsInbox />` with
 * no `resolve` either, so the evidence a person is being asked to approve on —
 * the global inbox, away from any conversation — was struck through as not
 * found as well. Both call sites are reached through this one component, which
 * is where the fix belongs.
 *
 * The repair is a third state rather than a resolver, because the obvious
 * resolver is the wrong one. Resolving a dossier's ids from the dossier's own
 * `claims` is the circularity `makeResolver`'s header already refuses for
 * cards — "would make every id resolve and the check would assert nothing" —
 * and it would hide precisely the fabrication this chip exists to expose. The
 * dossier page has one genuine check of whether an id still resolves, and it is
 * the **Verify** button in the same row, which re-derives the claim against the
 * platform as it stands. So the chip stops answering a question nobody asked it
 * and says nobody asked.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { EvidenceChip } from '../Shared'

describe('a chip with no way to check does not claim the id is missing', () => {
  it('says nobody checked, rather than that the platform holds nothing', () => {
    render(<EvidenceChip id="exp_7f0c" />)
    const chip = screen.getByTestId('ev-exp_7f0c')
    expect(chip.getAttribute('data-resolved')).toBe('unchecked')
    expect(chip.textContent).toContain('not checked')
    expect(chip.textContent).not.toContain('not found')
  })

  it('does not strike the id through, because a strike is the answer “no”', () => {
    const { container } = render(<EvidenceChip id="exp_7f0c" />)
    expect(container.querySelector('s')).toBeNull()
    expect(screen.getByTestId('ev-exp_7f0c').className).not.toContain('missing')
  })

  it('keeps the id legible, since it is the thing a reviewer copies out', () => {
    render(<EvidenceChip id="exp_7f0c" />)
    expect(screen.getByTestId('ev-exp_7f0c').textContent).toContain('exp_7f0c')
  })

  it('still strikes through an id a resolver looked for and did not find', () => {
    // The honesty mark has to survive the fix, or this change has traded a
    // false negative for a blind spot. A resolver that answers `null` is a
    // lookup with a result.
    render(<EvidenceChip id="exp_ghost" resolve={() => null} />)
    const chip = screen.getByTestId('ev-exp_ghost')
    expect(chip.getAttribute('data-resolved')).toBe('false')
    expect(chip.textContent).toContain('not found')
  })

  it('gives the three states three different renderings', () => {
    const { container: unchecked } = render(<EvidenceChip id="a" />)
    const { container: missing } = render(<EvidenceChip id="b" resolve={() => null} />)
    const { container: found } = render(
      <EvidenceChip id="c" resolve={() => ({ label: 'an experiment', kind: 'experiment' })} />,
    )
    const states = [unchecked, missing, found].map(
      (c) => c.querySelector('[data-resolved]')?.getAttribute('data-resolved'),
    )
    expect(new Set(states).size).toBe(3)
  })

  it('is not clickable, because there is nothing it could be trusted to open', () => {
    // A `<button>` here would invite the click that an unverified id must not
    // get — the resolved chip is the one that opens things.
    const { container } = render(<EvidenceChip id="exp_7f0c" onOpen={() => undefined} />)
    expect(container.querySelector('button')).toBeNull()
  })
})
