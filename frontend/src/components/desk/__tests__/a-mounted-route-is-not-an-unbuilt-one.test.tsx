/**
 * The dossier reader told every reader the platform had not built the route it
 * had just called (AGENT-012 §5; Set S tasks S-6.17, S-8.7).
 *
 * `DossierReader` sent `/api/dossiers/{ticket}` through `optional()`, which
 * turns a 404 into `null`, and printed one sentence over every `null`:
 *
 *   > /api/dossiers/{ticket} is not built yet (AGENT-012 §17).
 *
 * That was true when it was typed. It is false now, and verifying it took one
 * request: against the running platform on :7080, `GET /api/dossiers/T1`
 * answers **401**, `…/comments` answers 401, and `POST …/verify/{claim}`
 * answers 401 — where an unmounted path answers 404. `crates/api/src/routes/
 * mod.rs` registers all three, and `routes/dossiers.rs` opens with a header
 * about this exact caller having had no callee.
 *
 * The route's own 404 means something else entirely, and it takes the trouble
 * to say so: a dossier is compiled at the end of the run that produced it, so a
 * ticket with none had a claim its evidence did not support, or no critique —
 * and the run's events say which. `optional()` was throwing that away and
 * replacing it with a sentence about the platform being unfinished, which sends
 * the reader to the wrong place entirely.
 *
 * So the read has three answers now, and this file holds them apart.
 */
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import type { DossierRead } from '@/api/desk'

const dossier = vi.fn()
const dossierComments = vi.fn(async () => ({ comments: [] }))
vi.mock('@/api/desk', () => ({
  deskApi: {
    dossier: (t: string) => dossier(t),
    dossierComments: () => dossierComments(),
    addDossierComment: vi.fn(),
    verifyClaim: vi.fn(),
  },
}))

const { DossierReader } = await import('../Dossier')
    const { dossierCostLine, sampleCell, verdictOfRecheck } = await import('../Dossier.helpers')

function body(over: Record<string, unknown> = {}) {
  return {
    ticket: 'T-9',
    verdict: 'two of three claims are direct statistics',
    claims: [],
    plan_as_executed: [],
    funnel: [],
    unresolved_questions: ['whether the window generalises'],
    reviewer_critique: ['the holdout was looked at twice'],
    cost_usd: 0,
    trials: 12,
    provenance: { harness: 'h1', profile: 'p1', window: 'w1', seeds: 's1' },
    quoted_untrusted: [],
    ...over,
  }
}

async function readAs(read: DossierRead) {
  dossier.mockResolvedValueOnce(read)
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const r = render(
    <QueryClientProvider client={qc}>
      <DossierReader ticket="T-9" />
    </QueryClientProvider>,
  )
  await screen.findByText(/./)
  return r
}

describe('a ticket with no dossier is not a platform with no route', () => {
  it('gives the route’s own explanation instead of claiming the route is unbuilt', async () => {
    await readAs({
      kind: 'none',
      fix: 'a dossier is compiled at the end of the run that produced it. A run that ended without one had a claim its evidence did not support, or no critique — the run’s events say which',
    })
    expect(await screen.findByText(/No dossier was compiled for T-9/)).toBeInTheDocument()
    expect(screen.getByText(/the run’s events say which/)).toBeInTheDocument()
    // The sentence that was there before, and that sent the reader to look for
    // a feature rather than for their run.
    expect(screen.queryByText(/is not built yet/)).toBeNull()
  })

  it('still names the absence when the route sent no reason with its 404', async () => {
    await readAs({ kind: 'none', fix: null })
    expect(await screen.findByText(/No dossier was compiled for T-9/)).toBeInTheDocument()
    expect(screen.getByText(/gave no further reason/)).toBeInTheDocument()
  })

  it('keeps a failed request apart from a ticket that has no document', async () => {
    await readAs({ kind: 'unreadable', message: 'dossier_unreadable: connection reset' })
    expect(await screen.findByText(/could not be read/)).toBeInTheDocument()
    expect(screen.getByText(/connection reset/)).toBeInTheDocument()
    expect(screen.queryByText(/No dossier was compiled/)).toBeNull()
  })

  it('renders the document when the route answered with one', async () => {
    await readAs({ kind: 'found', body: body() })
    expect(await screen.findByText(/two of three claims are direct statistics/)).toBeInTheDocument()
  })
})

/**
 * `crates/api/src/dossier.rs` writes `"cost_usd": produced.cost_usd`, which is
 * `local_driver.rs:938 → self.yielded.cost_usd` — a field of
 * `harness::lifecycle::yield::Yield` that nothing in the workspace assigns. It
 * arrives at its `Default` on every dossier the platform compiles, and
 * `research_run_worker.rs` passes a literal `Some(0.0)` twice besides.
 *
 * The reader printed `$${d.cost_usd.toFixed(2)}`, so the document a reviewer
 * opens to judge whether research was worth its cost said **$0.00** — while the
 * Spend table and the Cost tab in the same app already say "not priced… this is
 * not zero", and `session_driver.rs` says out loud that there is no per-run USD
 * on this platform.
 */
describe('a dossier nobody priced is not a run that was free', () => {
  it('refuses to print $0.00 for a figure the platform never records', () => {
    const said = dossierCostLine(0, 12)
    expect(said).toContain('Not priced')
    expect(said).toContain('not a run that was free')
    expect(said).not.toContain('$0.00')
    expect(said).toContain('12 trials')
  })

  it('prints a real cost the day the platform has one', () => {
    expect(dossierCostLine(4.2, 12)).toBe('$4.20 · 12 trials')
  })

  it('does not turn a missing field into a measured zero, or throw on it', () => {
    // `undefined.toFixed(2)` is a white screen, and `Math.round(null)` is 0 —
    // the two ways this shape has shipped before.
    expect(dossierCostLine(undefined, 12)).toContain('not recorded')
    expect(dossierCostLine(null, undefined)).toContain('trials not recorded')
    expect(dossierCostLine(Number.NaN, 12)).toContain('not recorded')
  })

  it('carries the caveat into the page, not just the helper', async () => {
    await readAs({ kind: 'found', body: body() })
    expect((await screen.findByTestId('dossier-cost')).textContent).toContain('Not priced')
  })
})

describe('an absent sample size is not an unknown one rendered as a word', () => {
  it('names both halves when the document has neither', () => {
    expect(sampleCell(null, null)).toBe('(n unknown) / (se unknown)')
  })

  it('does not let an absent field through as the literal undefined', () => {
    // `c.n === null ? … : c.n` passed `undefined` straight to React, and
    // `c.se ?? '—'` printed an em dash that reads as a measured nothing.
    expect(sampleCell(undefined, undefined)).not.toContain('undefined')
    expect(sampleCell(undefined, undefined)).toBe('(n unknown) / (se unknown)')
  })

  it('prints a genuine zero, which is a measurement', () => {
    expect(sampleCell(0, 0)).toBe('0 / 0')
  })
})

/**
 * `POST …/verify/{claim}` answers 404 with a body — "there is no dossier with
 * that ticket", "this dossier has no claim citing `x`" — and answers 200 with
 * `was`/`now` when it genuinely re-derived. `optional()` folded the 404 into
 * `null` and the row said *"the recheck endpoint did not answer"* about a route
 * that had answered precisely; and every non-`ok` reply, including "this
 * evidence id no longer resolves", printed as **does not hold**, which reads as
 * a refutation of the research rather than a fact about the lookup.
 */
describe('a lookup that found nothing is not a claim that failed', () => {
  it('reports a real re-derivation as one, with the route’s own comparison', () => {
    const said = verdictOfRecheck({
      ok: false,
      note: 'this claim no longer holds. The dossier records it as direct_stat',
      was: 'direct_stat',
      now: 'unverified',
    })
    expect(said).toContain('does not hold')
    expect(said).toContain('direct_stat')
  })

  it('does not report an unresolvable id as a failed claim', () => {
    const said = verdictOfRecheck({
      ok: false,
      note: '`exp_7f0c` no longer resolves on this platform, so the claim cannot be re-derived',
    })
    expect(said).toContain('not re-derived')
    expect(said).not.toContain('does not hold')
  })

  it('says the request failed, not the claim, when nothing came back', () => {
    const said = verdictOfRecheck({
      ok: false,
      note: 'the recheck request did not come back (no response) — this says nothing about the claim',
    })
    expect(said).toContain('not re-derived')
    expect(said).toContain('says nothing about the claim')
  })

  it('reports a claim that still holds', () => {
    const said = verdictOfRecheck({ ok: true, note: 're-derived: direct_stat', was: 'direct_stat', now: 'direct_stat' })
    expect(said).toContain('holds')
    expect(said).not.toContain('not re-derived')
  })
})
