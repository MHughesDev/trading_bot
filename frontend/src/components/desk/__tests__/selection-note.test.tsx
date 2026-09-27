/**
 * "Why did the model not know that?" — the selection card (AGENT-005 §28.3;
 * S-9.19).
 *
 * The most common question about an agent that did something strange, and one
 * no version of this system could previously answer. The clause was computed
 * during selection and rendered into the prompt for the records that won; for
 * the records that *lost* — the ones this question is always about — it was
 * discarded at the boundary, and the timeline note carried two lists of opaque
 * ids.
 *
 * These tests are about the half a reader is asked to trust: that the card
 * leads with what was left out, that a line without a clause is not rendered as
 * a line with an empty one, and that a capped list says it was capped.
 */
import { describe, expect, it } from 'vitest'
import { selectionLines, selectionSummary } from '@/components/desk/Cards.helpers'


const note = {
  policy_version: 'v2',
  candidates: 4,
  selected: ['r001', 'r003'],
  elided: ['r002', 'r004'],
  bound_by: ['body budget'],
  lines_omitted: 0,
  reasons: [
    { id: 'r002', selected: false, reason: 'superseded by r004', score: 0.05 },
    { id: 'r004', selected: false, reason: 'same subject, scored below r001', score: 0.49 },
    { id: 'r003', selected: true, reason: 'the correction this step is recovering from', score: 0.51 },
    { id: 'r001', selected: true, reason: 'names the id this step uses', score: 0.95 },
  ],
}

describe('selectionLines', () => {
  it('reads every record and keeps its clause', () => {
    const lines = selectionLines(note)
    expect(lines).toHaveLength(4)
    for (const l of lines) expect(l.reason.length).toBeGreaterThan(0)
    expect(lines.find((l) => l.id === 'r002')?.reason).toBe('superseded by r004')
  })

  it('drops a line with no clause rather than rendering an empty one', () => {
    // An empty row says "no reason". The truth is "this note predates the
    // field", and those are different things to tell a reviewer.
    const lines = selectionLines({ reasons: [{ id: 'r009', selected: false }] })
    expect(lines).toHaveLength(0)
  })

  it('survives a note that carries no reasons at all', () => {
    expect(selectionLines({ selected: ['r001'] })).toEqual([])
    expect(selectionLines(null)).toEqual([])
    expect(selectionLines({ reasons: 'not an array' })).toEqual([])
  })

  it('keeps a missing score as null rather than zero', () => {
    // Zero is a score. Absent is not, and rendering one as the other would put
    // a record at the bottom of the ranking for having no number.
    const [line] = selectionLines({ reasons: [{ id: 'r001', selected: true, reason: 'because' }] })
    expect(line.score).toBeNull()
  })
})

describe('selectionSummary', () => {
  it('leads with what was left out', () => {
    const s = selectionSummary(note)
    expect(s).toMatch(/^2 record\(s\) did not reach this prompt/)
    expect(s).toContain('4 candidate(s)')
    expect(s).toContain('body budget')
  })

  it('says so explicitly when nothing was left out', () => {
    // An omitted statement and an empty one look identical and mean opposite
    // things — the dossier's mandatory-section rule, one level down.
    const s = selectionSummary({ ...note, reasons: note.reasons.map((r) => ({ ...r, selected: true })) })
    expect(s).toContain('nothing was left out')
  })

  it('says when the list is capped', () => {
    const s = selectionSummary({ ...note, lines_omitted: 7 })
    expect(s).toContain('7 more line(s) are not shown')
  })

  it('does not claim a candidate count it was not given', () => {
    const { candidates, ...without } = note
    expect(candidates).toBe(4)
    expect(selectionSummary(without)).not.toContain('candidate(s)')
  })
})
