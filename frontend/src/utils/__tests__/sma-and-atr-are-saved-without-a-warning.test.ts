/**
 * Set E 4.1: SMA and ATR execute in the backtest engine (the single feature
 * runtime serves `sma_N` and `atr_N`; `requirements.rs` now derives them), so
 * the builder must stop telling the author they are unsupported — and must still
 * emit the feature names the engine resolves.
 */
import { describe, expect, it } from 'vitest'
import { ruleSpecToDefinition } from '@/utils/toDefinition'
import type { RuleStrategySpec } from '@/types/spec'

const spec = (kind: 'sma' | 'atr', period: number): RuleStrategySpec => ({
  name: 'indicator test',
  indicators: [
    { id: 'a', kind, period },
    { id: 'b', kind: 'ema', period: 21 },
  ],
  entry: {
    side: 'buy',
    all: [],
    any: [{ type: 'greater_than', left: 'a', right_id: 'b' }],
  },
  size: { type: 'fixed_quantity', value: 0.01 },
  exits: [],
})

describe('SMA / ATR in the builder', () => {
  it.each([
    ['sma', 20],
    ['atr', 14],
  ] as const)('%s_%i saves with no unsupported-indicator warning and the engine name', (kind, period) => {
    const r = ruleSpecToDefinition(spec(kind, period))
    expect(r.errors).toEqual([])
    expect(r.warnings.join(' ')).not.toMatch(/not yet supported/i)
    expect(JSON.stringify(r.definition)).toContain(`${kind}_${period}`)
  })
})
