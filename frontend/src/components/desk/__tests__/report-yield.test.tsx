/**
 * What a run produced, on the card that reports it (AGENT-013 §10; S-10.13,
 * S-10.15) — and the dossier it compiled being reachable from the conversation
 * that produced it (S-8.7).
 *
 * Two gaps, both of the same shape: the platform computed the thing and the
 * last hop to the reader was missing. The yield columns have been on
 * `agent_runs` since S-10.13 and the Report card showed a headline and a button;
 * the `dossier` event had no entry in `KIND_MAP`, so a compiled dossier fell to
 * `raw`, was folded away, and the "Open dossier" button — which reads
 * `detail.ticket` — could never appear.
 */
import { describe, expect, it } from 'vitest'

import { toCard } from '@/lib/desk/events'
import { yieldLine } from '@/components/desk/Cards.helpers'

describe('yieldLine', () => {
  it('names each count rather than scoring them', () => {
    const line = yieldLine({
      findings_established: 1,
      findings_suggestive: 2,
      questions_opened: 3,
      trials_spent: 12,
    })
    expect(line).toContain('1 established finding')
    expect(line).toContain('2 suggestive findings')
    expect(line).toContain('3 questions opened')
    expect(line).toContain('12 trial(s)')
    // A weighted total would let three questions and no findings read as
    // productive. There is no total.
    expect(line).not.toMatch(/score|total/i)
  })

  it('singularises, because "1 questions opened" reads as a bug', () => {
    expect(yieldLine({ questions_opened: 1, trials_spent: 0 })).toContain('1 question opened')
    expect(yieldLine({ questions_opened: 2, trials_spent: 0 })).toContain('2 questions opened')
  })

  it('says barren when a run spent trials and produced nothing', () => {
    // The one number on the card that measures whether the work was worth
    // doing, rather than whether the harness worked.
    const line = yieldLine({ trials_spent: 9 })
    expect(line).toContain('barren')
    expect(line).toContain('9 trial(s)')
  })

  it('distinguishes producing nothing from never being measured', () => {
    // Zero trials and nothing produced is a real, complete answer.
    expect(yieldLine({ trials_spent: 0 })).toContain('no trial spent')
    // No trial count at all is a run recorded before these columns existed.
    // Printing "0 findings" for it would state a measurement nobody made.
    expect(yieldLine({ summary: 'done' })).toBeNull()
    expect(yieldLine(null)).toBeNull()
  })

  it('omits a count of zero rather than listing it', () => {
    const line = yieldLine({ findings_established: 2, negatives: 0, proposals: 0, trials_spent: 4 })
    expect(line).toContain('2 established findings')
    expect(line).not.toContain('negative')
    expect(line).not.toContain('proposal')
  })
})

describe('the dossier event reaches the reader', () => {
  const ev = (kind: string, content: Record<string, unknown>) => ({
    seq: 1,
    kind,
    content,
    created_at: '2026-09-21T00:00:00Z',
  })

  it('maps a dossier event to the report card instead of folding it away', () => {
    const card = toCard(ev('dossier', { ticket: 'run-1' }), 3)
    expect(card.kind).toBe('report')
    expect(card.raw).toBeUndefined()
    expect(card.detail?.ticket).toBe('run-1')
  })

  it('says why there is no dossier rather than showing nothing', () => {
    // A dossier refuses to compile when a claim is not supported by its
    // evidence, or when no critic ran. That is a finding about the run, and an
    // absent button says none of it.
    const card = toCard(ev('dossier', { not_compiled: 'a claim in this report is not supported' }), 3)
    expect(card.kind).toBe('report')
    expect(card.headline).toContain('no dossier')
    expect(card.headline).toContain('not supported')
  })

  it('keeps a real summary when the event carries one', () => {
    const card = toCard(ev('report', { summary: 'funding leg survives costs' }), 3)
    expect(card.headline).toBe('funding leg survives costs')
  })
})
