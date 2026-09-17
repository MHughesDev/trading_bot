/**
 * An open question and a closed one must not look the same.
 *
 * `/api/memory/{bucket}` served the generic half of an entry and none of
 * AGENT-007 §2.2's right-hand column, so the "Open questions" tab rendered a
 * question as a finding: no `blocking`, no `status`, no `why_it_matters`. A
 * question somebody had already answered sat in the list looking exactly like
 * one nobody had, which is the state the tab exists to make visible.
 *
 * These test `questionFields` rather than the rendered DOM, because the defect
 * was in what reached the component: a renderer test would have passed on rows
 * that carried nothing.
 */
import { describe, expect, it } from 'vitest'

import { questionFields, type MemoryRow } from '../MemoryBrowser'

function row(extra: Record<string, unknown> | undefined): MemoryRow {
  return {
    id: 'q_1',
    subject: 'BTC-USD',
    claim: 'does the funding leg survive costs',
    extra,
  }
}

describe('questionFields', () => {
  it('reads an open question', () => {
    const q = questionFields(
      row({
        question: 'does the funding leg survive costs',
        why_it_matters: 'the carry thesis rests on it',
        blocking: true,
        value: 'decisive',
        status: 'open',
      }),
    )
    expect(q).not.toBeNull()
    expect(q?.blocking).toBe(true)
    expect(q?.value).toBe('decisive')
    expect(q?.why_it_matters).toBe('the carry thesis rests on it')
    expect(q?.closedBy).toBeUndefined()
  })

  it('reads a closed question and names who closed it', () => {
    const q = questionFields(
      row({
        why_it_matters: 'the carry thesis rests on it',
        blocking: true,
        value: 'decisive',
        status: { closed: { by: 'finding_9', how: 'answered' } },
      }),
    )
    // The distinction the tab exists for.
    expect(q?.closedBy).toBe('finding_9')
    expect(q?.closedHow).toBe('answered')
  })

  it('is null for an entry that is not a question, so other tabs stay generic', () => {
    expect(questionFields(row(undefined))).toBeNull()
    expect(questionFields(row({}))).toBeNull()
    // A negative result carries its own kind-specific field and no question
    // fields; it must not be rendered as a question with everything missing.
    expect(questionFields(row({ failure_class: 'weak_effect' }))).toBeNull()
  })

  it('defaults `blocking` to false on a malformed row', () => {
    // The one place a wrong default is expensive: a question that claimed to
    // block on nothing but a missing field would lead the next run's brief.
    expect(questionFields(row({ why_it_matters: 'x' }))?.blocking).toBe(false)
    expect(questionFields(row({ why_it_matters: 'x', blocking: 'yes' }))?.blocking).toBe(false)
  })

  it('reports unstated stakes as absent rather than as an empty string', () => {
    // The run-end write records questions whose stakes were never stated, and a
    // reader deciding what to spend a run on has to tell those from the ones
    // somebody asked deliberately.
    const q = questionFields(row({ blocking: false, value: 'curious', status: 'open' }))
    expect(q).not.toBeNull()
    expect(q?.why_it_matters).toBeUndefined()
  })
})
