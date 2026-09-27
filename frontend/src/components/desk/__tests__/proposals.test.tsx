/**
 * A proposal that would change nothing must say so before it is decided.
 *
 * Two of the three proposal kinds have no approval-side writer, and a proposal
 * created before migration 0131 names no target row. Approving either records
 * the decision and performs nothing — which is fine, and is exactly what
 * `lifecycle.propose` already refuses to hide on the agent's side by answering
 * `"effect": "nothing yet"` out loud. The person deciding is owed the same
 * honesty, and owed it *before* they click rather than in the result.
 */
import { describe, expect, it } from 'vitest'

import type { Proposal } from '@/api/desk'
import { inertNotice, ringLabel } from '@/components/desk/ProposalsQueue.helpers'

function proposal(over: Partial<Proposal>): Proposal {
  return {
    proposal_id: 'p_1',
    kind: 'deploy_paper',
    ring: 2,
    envelope: 'promotion',
    dossier_ref: 'dossier_41',
    subject: 'ema_cross_v3',
    subject_id: '11111111-1111-1111-1111-111111111111',
    rationale: 'survived the battery on two venues',
    actionable: true,
    ...over,
  }
}

describe('inertNotice', () => {
  it('says nothing when approving would perform the write', () => {
    // The ordinary case explains itself; a notice here would bury the case that
    // matters.
    expect(inertNotice(proposal({}))).toBeNull()
  })

  it('warns when the kind has no approval-side writer', () => {
    const n = inertNotice(proposal({ kind: 'promote_strategy', ring: 1, actionable: false }))
    expect(n).toContain('changes nothing')
    expect(n).toContain('writer')
  })

  it('distinguishes a proposal that names no target row', () => {
    // A different repair from a missing writer: this one is a row that predates
    // the column, and conflating them would send somebody looking for a writer
    // that exists.
    const n = inertNotice(proposal({ actionable: false, subject_id: null }))
    expect(n).toContain('no target row')
    expect(n).not.toContain('writer')
  })
})

describe('ringLabel', () => {
  it('says what a ring protects rather than its number', () => {
    // "1" and "2" read as degrees of one thing. Writing a strategy and trading
    // on paper are different decisions.
    expect(ringLabel(2)).toContain('paper')
    expect(ringLabel(1)).toContain('artifacts')
    expect(ringLabel(0)).toContain('research')
  })

  it('treats anything above 2 as reaching paper', () => {
    // A ring the desk does not know must not render as the least dangerous one.
    expect(ringLabel(3)).toContain('paper')
  })
})
