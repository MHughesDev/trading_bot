/**
 * S-6.17's acceptance, which had no test (AGENT-012 §5, §19.5; task S-6.17).
 *
 * **Acceptance:** *"opened cold, verdict / evidence / unresolved questions /
 * critique are above the fold; **verify** updates in place."* `Dossier.tsx` says in
 * its own header that those four are the first four sections and nothing above
 * them is collapsed — and nothing checked it. The order of sections in a document
 * is exactly the kind of property a later edit moves by accident: adding a
 * provenance strip or a cost line "at the top where people will see it" is a
 * one-line change that pushes the reviewer critique under the fold, and nothing
 * fails.
 *
 * "Above the fold" is not measurable in jsdom, which does no layout. What *is*
 * measurable, and is what the clause is actually protecting, is that the four
 * mandatory sections come first, in that order, ahead of every optional one, and
 * that none of them is behind a disclosure a reader has to find. A `<details>`
 * wrapped around the critique would satisfy any ordering check while hiding the
 * section, so that is asserted separately.
 *
 * The second clause is the one that would break loudly: **verify** must re-render
 * the claim's own cell rather than navigating, reloading or replacing the
 * document. It is asserted by the surrounding document still being there
 * afterwards, which is the part "in place" means.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactElement } from 'react'
import { describe, expect, it, vi } from 'vitest'

const dossier = vi.hoisted(() => vi.fn())
const verifyClaim = vi.hoisted(() => vi.fn())
vi.mock('@/api/desk', () => ({ deskApi: { dossier, verifyClaim } }))

import { DossierReader } from '../Dossier'

const BODY = {
  ticket: 'tk_1',
  verdict: 'the overnight gap does not pay after costs',
  claims: [
    {
      claim: 'the edge is 3bp before costs',
      evidence_id: 'exp_1',
      n: 412,
      se: 0.8,
      verification: 'out-of-sample replication',
      evidence_class: 'direct_stat',
      chain: [],
    },
    {
      claim: 'costs are 4bp round trip',
      evidence_id: 'exp_2',
      n: 412,
      se: 0.2,
      verification: 'fee schedule',
      evidence_class: 'direct_stat',
      chain: [],
    },
  ],
  unresolved_questions: ['does it survive a different venue?'],
  reviewer_critique: ['the window is one regime'],
  plan_as_executed: ['pull the bars', 'sweep the window'],
  funnel: [],
  cost_usd: 0,
  trials: 4,
  provenance: { harness: 'h1', profile: 'p1', window: '2024-01..2024-06', seeds: '7' },
  quoted_untrusted: [],
}

function withClient(ui: ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } })
  return render(<QueryClientProvider client={qc}>{ui}</QueryClientProvider>)
}

async function openCold() {
  dossier.mockResolvedValue({ kind: 'ok', body: BODY })
  const r = withClient(<DossierReader ticket="tk_1" />)
  await screen.findByText(BODY.verdict)
  return r
}

describe('the four mandatory sections are the ones a reader meets first', () => {
  it('puts verdict, evidence, unresolved questions and critique ahead of everything else', async () => {
    const { container } = await openCold()
    const headings = [...container.querySelectorAll('h2, h3')].map((h) => h.textContent)
    expect(headings.slice(0, 4)).toEqual(['Verdict', 'Evidence', 'Unresolved questions', 'Reviewer critique'])
    // …and the optional ones are genuinely after them, so the assertion above is
    // about position and not about a document that has only four sections.
    expect(headings.length).toBeGreaterThan(4)
    expect(headings).toContain('Cost')
    expect(headings).toContain('Provenance')
  })

  it('does not fold any of the four behind a disclosure', async () => {
    // An ordering check passes just as happily over a `<details>` that hides the
    // critique. Being first and being read are different properties.
    const { container } = await openCold()
    for (const text of [
      BODY.verdict,
      BODY.claims[0].claim,
      BODY.unresolved_questions[0],
      BODY.reviewer_critique[0],
    ]) {
      const el = screen.getByText(text, { exact: false })
      expect(el.closest('details'), text).toBeNull()
      expect(el.closest('[hidden]'), text).toBeNull()
    }
    expect(container.querySelector('.dossier')).not.toBeNull()
  })

  it('marks a missing mandatory section rather than omitting it', async () => {
    // The clause is about the section being above the fold, and a dossier that
    // simply left it out would pass an ordering check by having nothing to order.
    dossier.mockResolvedValue({
      kind: 'ok',
      body: { ...BODY, unresolved_questions: [], reviewer_critique: [] },
    })
    const { container } = withClient(<DossierReader ticket="tk_1" />)
    await screen.findByText(BODY.verdict)
    const headings = [...container.querySelectorAll('h2, h3')].map((h) => h.textContent)
    expect(headings.slice(0, 4)).toEqual(['Verdict', 'Evidence', 'Unresolved questions', 'Reviewer critique'])
    expect(screen.getAllByText('mandatory section is empty')).toHaveLength(2)
  })
})

describe('verify updates in place', () => {
  it('writes the recheck into the claim’s own row and leaves the document standing', async () => {
    await openCold()
    verifyClaim.mockResolvedValue({ ok: true, note: 're-derived: direct_stat', was: 'direct_stat', now: 'direct_stat' })
    const row = screen.getByText('the edge is 3bp before costs').closest('tr')
    expect(row).not.toBeNull()
    const fetchesBefore = dossier.mock.calls.length
    fireEvent.click(screen.getAllByRole('button', { name: 'Verify' })[0])
    await waitFor(() => expect(row?.textContent).toContain('re-derived'))
    // In place: the verdict, the other claim and the sections below are all still
    // rendered, and the document was not re-fetched to say so.
    expect(screen.getByText(BODY.verdict)).toBeInTheDocument()
    expect(screen.getByText('costs are 4bp round trip')).toBeInTheDocument()
    expect(screen.getByText(BODY.reviewer_critique[0])).toBeInTheDocument()
    expect(dossier.mock.calls.length).toBe(fetchesBefore)
  })

  it('does not write one claim’s recheck into another claim’s row', async () => {
    await openCold()
    verifyClaim.mockResolvedValue({ ok: true, note: 're-derived: direct_stat', was: 'direct_stat', now: 'direct_stat' })
    fireEvent.click(screen.getAllByRole('button', { name: 'Verify' })[0])
    await waitFor(() => expect(screen.getByText(/re-derived/)).toBeInTheDocument())
    const other = screen.getByText('costs are 4bp round trip').closest('tr')
    expect(other?.textContent).not.toContain('re-derived')
  })

  it('puts a lookup that found nothing in the row as a lookup, not as a refuted claim', async () => {
    // `verdictOfRecheck`'s three outcomes are unit-tested in
    // `a-mounted-route-is-not-an-unbuilt-one.test.tsx`; this is the one assertion
    // that file cannot make, which is that the distinction survives the trip into
    // the cell a reviewer actually reads. "does not hold" beside a claim whose id
    // merely stopped resolving reads as a refutation of the research.
    await openCold()
    verifyClaim.mockResolvedValue({
      ok: false,
      note: '`exp_1` no longer resolves on this platform, so the claim cannot be re-derived',
    })
    const row = screen.getByText('the edge is 3bp before costs').closest('tr')
    fireEvent.click(screen.getAllByRole('button', { name: 'Verify' })[0])
    await waitFor(() => expect(row?.textContent).toContain('not re-derived'))
    expect(row?.textContent).not.toContain('does not hold')
  })
})
