import { describe, expect, it } from 'vitest'
import type { Verdict } from '@/api/modelsStudio'
import { EMPTY_FORM, buildDeclaration, stepOfField, type DeclarationForm } from '../declaration'
import { blockingRules, ruleBlocks, scoreColumns, scoreText, verdictHeadline } from '../verdict'

const filled = (over: Partial<DeclarationForm> = {}): DeclarationForm => ({
  ...EMPTY_FORM,
  instruments: 'BTC-USD, ETH-USD',
  start: '2026-01-01',
  end: '2026-06-01',
  features: 'ema(close,21)\nrsi(close,14)',
  horizon: '12',
  weights: 'uniqueness',
  trainDays: '90',
  testDays: '14',
  refitDays: '14',
  ...over,
})

describe('buildDeclaration', () => {
  it('builds the typed shape and never carries purge or embargo', () => {
    const { declaration, problems } = buildDeclaration(filled())
    expect(problems).toEqual([])
    expect(declaration?.window).toEqual({ start: '2026-01-01T00:00:00Z', end: '2026-06-01T23:59:59Z' })
    expect(declaration?.target).toEqual({ kind: 'forward_return', horizon_bars: 12, log: true })
    expect(JSON.stringify(declaration)).not.toMatch(/purge|embargo/i)
  })

  it('keeps commas inside a feature expression', () => {
    expect(buildDeclaration(filled()).declaration?.features).toEqual(['ema(close,21)', 'rsi(close,14)'])
  })

  it('has no default weighting', () => {
    const r = buildDeclaration(filled({ weights: '' }))
    expect(r.declaration).toBeNull()
    expect(r.problems.map((p) => p.field)).toContain('weights')
  })

  it('names every field it could not parse', () => {
    const r = buildDeclaration(filled({ instruments: '', start: '', horizon: 'abc', trainDays: '' }))
    expect(r.problems.map((p) => p.field).sort()).toEqual(['cv.train_days', 'target', 'universe', 'window.start'])
  })

  it('requires a sequence window only for a family that needs one', () => {
    expect(buildDeclaration(filled()).declaration?.sequence).toBeNull()
    const seq = buildDeclaration(filled({ seqWindow: '32' }), { needsSequence: true })
    expect(seq.declaration?.sequence).toEqual({ window: 32, stride: 1 })
    expect(buildDeclaration(filled(), { needsSequence: true }).problems.map((p) => p.field)).toContain('sequence.window')
  })

  it('builds a triple barrier and a time-decay weighting', () => {
    const r = buildDeclaration(filled({ targetKind: 'triple_barrier', ptSigma: '2', slSigma: '1', maxBars: '30', weights: 'time_decay', halfLife: '45' }))
    expect(r.declaration?.target).toEqual({ kind: 'triple_barrier', pt_sigma: 2, sl_sigma: 1, max_bars: 30, path: 'high_low' })
    expect(r.declaration?.weights).toEqual({ kind: 'time_decay', half_life_days: 45 })
  })

  it('maps a server field to the step that owns it', () => {
    expect(stepOfField('window.start')).toBe('universe')
    expect(stepOfField('transforms.lags')).toBe('features')
    expect(stepOfField('weights')).toBe('target')
    expect(stepOfField('cv.k')).toBe('cv')
    expect(stepOfField('anything_else')).toBe('review')
  })
})

const rule = (status: string, applies = true) => ({ rule: 'vol_forecast', applies, status, reason: '', measured: {} }) as never
const verdict = (rules: unknown[], allowed: boolean): Verdict =>
  ({ model_id: 'm', version: 1, allowed, rules, baselines: [], complexity: null }) as Verdict

describe('the verdict rules', () => {
  it('blocks on fail and on not_computable, and on nothing else', () => {
    expect(ruleBlocks(rule('fail'))).toBe(true)
    expect(ruleBlocks(rule('not_computable'))).toBe(true)
    expect(ruleBlocks(rule('pass'))).toBe(false)
    expect(ruleBlocks(rule('not_applicable', false))).toBe(false)
    // a rule that does not apply never blocks, whatever its status says
    expect(ruleBlocks(rule('fail', false))).toBe(false)
  })

  it('is allowed only when the server says so and nothing blocks', () => {
    expect(verdictHeadline(verdict([rule('pass')], true)).tone).toBe('pos')
    expect(verdictHeadline(verdict([rule('not_computable')], true)).tone).toBe('neg')
    expect(verdictHeadline(verdict([rule('pass')], false)).tone).toBe('warn')
    expect(blockingRules(verdict([rule('fail'), rule('pass')], false))).toHaveLength(1)
  })

  it('prints a score only when it is a finite number', () => {
    expect(scoreText(0.123456)).toBe('0.1235')
    expect(scoreText(Number.NaN)).toBe('not reported')
    expect(scoreText(undefined)).toBe('not reported')
    expect(scoreText(null)).toBe('not reported')
  })

  it('unions score columns in first-seen order', () => {
    expect(scoreColumns([{ scores: { a: 1, b: 2 } }, { scores: { b: 1, c: 3 } }])).toEqual(['a', 'b', 'c'])
  })
})
