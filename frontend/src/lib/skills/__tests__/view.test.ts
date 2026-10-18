import { describe, expect, it } from 'vitest'
import type { Verification } from '@/api/skills'
import {
  admitControl,
  canQuarantine,
  noticeWords,
  openProposals,
  proposalTone,
  similarityWords,
  statusTone,
  stepRows,
  usageWords,
} from '../view'

const v = (over: Partial<Verification> = {}): Verification => ({
  proposal_id: 'p1',
  steps: { lint: true, procedure: true, triggers: true, coexistence: true, replay: true, reviewer: true },
  required: ['lint', 'procedure', 'triggers', 'coexistence', 'replay', 'reviewer'],
  first_unmet: null,
  admissible: true,
  ...over,
})

describe('admit is offered only for a proposal this view has verified admissible', () => {
  it('is off before anything has been verified, and says why', () => {
    const c = admitControl('submitted', null)
    expect(c.enabled).toBe(false)
    expect(c.why).toMatch(/verify first/)
  })

  it('is off while a step is owed, naming the step', () => {
    const c = admitControl('submitted', v({ admissible: false, first_unmet: 'reviewer', steps: { lint: true } }))
    expect(c).toEqual({ enabled: false, why: 'step `reviewer` has not passed' })
  })

  it('is on for an admissible verification of an undecided proposal', () => {
    expect(admitControl('submitted', v()).enabled).toBe(true)
    expect(admitControl('verifying', v()).enabled).toBe(true)
  })

  it('is never on for a proposal already decided, whatever the verification says', () => {
    expect(admitControl('admitted', v()).enabled).toBe(false)
    expect(admitControl('rejected', v()).enabled).toBe(false)
  })
})

describe('the verification steps', () => {
  it('are listed in the registry order, and one never run is unrun, not failed', () => {
    const rows = stepRows(v({ steps: { lint: true, procedure: false } }))
    expect(rows.map((r) => r.step)).toEqual(['lint', 'procedure', 'triggers', 'coexistence', 'replay', 'reviewer'])
    expect(rows[0].state).toBe('passed')
    expect(rows[1].state).toBe('failed')
    expect(rows[2].state).toBe('unrun')
  })
})

describe('words and tones', () => {
  it('shows an unrecognised standing as itself in a warning tone', () => {
    expect(statusTone('quarantined')).toBe('neg')
    expect(statusTone('mystery')).toBe('warn')
    expect(proposalTone('mystery')).toBe('warn')
  })

  it('only a skill still standing can be quarantined', () => {
    expect(canQuarantine({ skill_id: 's', status: 'admitted' })).toBe(true)
    expect(canQuarantine({ skill_id: 's', status: 'provisional' })).toBe(true)
    expect(canQuarantine({ skill_id: 's', status: 'quarantined' })).toBe(false)
    expect(canQuarantine({ skill_id: null, status: 'admitted' })).toBe(false)
  })

  it('the queue is the undecided proposals', () => {
    const ps = [{ status: 'submitted' }, { status: 'admitted' }, { status: 'verifying' }, { status: 'rejected' }]
    expect(openProposals(ps).map((p) => p.status)).toEqual(['submitted', 'verifying'])
  })

  it('a skill nobody was exposed to has no usage, not a zero one', () => {
    expect(usageWords({ exposures: 0, activations: 0, balance: 0 })).toMatch(/never exposed/)
    expect(usageWords({ exposures: 20, activations: 5, balance: 3 })).toBe(
      '5 of 20 exposures activated (25%), balance +3',
    )
    expect(similarityWords(null)).toBe('not measured')
    expect(similarityWords(0.82)).toMatch(/82%/)
    expect(noticeWords({ notice_id: 1, skill: 'x', kind: 'quarantined', reason: 'it leaked a ticker' })).toBe(
      'x was quarantined: it leaked a ticker',
    )
  })
})
