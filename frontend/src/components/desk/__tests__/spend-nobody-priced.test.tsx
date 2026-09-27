/**
 * The Spend table printed three zeros that nobody had measured (AGENT-012 §12;
 * Set S task S-6.21).
 *
 * `Object.entries(u).map(…)` rendered `/api/agent/usage` verbatim, and that
 * response fills three fields with a value meaning "nobody has this number":
 *
 * * `cost_usd` is `COALESCE(SUM(cost_usd),0)` and there is **no price table** in
 *   this platform (OQ-16). The Cost tab, six tabs away in the same app, already
 *   says "not priced… this is not $0" — and this page printed a cost of 0 next
 *   to a token count in the hundreds of thousands, on the surface an operator
 *   opens to answer "what is this costing me".
 * * `cache_hit_rate` is `0.0` when nothing billable has been sent, so an
 *   unmeasured rate rendered as a measured zero — and it is a fraction printed
 *   bare in a column of counts, so `0.35` read as thirty-five of something.
 * * `dollars_per_verdict` is `null` when no verdict was filed, and
 *   `String(null)` put the literal word **null** in the table.
 *
 * This is the platform's one non-negotiable — an absent number is never rendered
 * as zero — on the page whose numbers are money.
 */
import { describe, expect, it } from 'vitest'
import { usageRows } from '@/components/desk/Operations.helpers'


/** The response shape `get_usage` builds. */
function usage(over: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    input_tokens: 512_000,
    output_tokens: 48_000,
    cache_read: 128_000,
    cache_write: 4_000,
    cost_usd: 0,
    cache_hit_rate: 0.2,
    verdicts: 0,
    dollars_per_verdict: null,
    ...over,
  }
}

function value(rows: ReturnType<typeof usageRows>, key: string): string {
  const hit = rows?.find((r) => r.key === key)
  // A lookup that missed would make every assertion below vacuous.
  expect(hit, `no row for ${key}`).toBeDefined()
  return hit?.value ?? ''
}

describe('a run nobody priced does not render as a free one', () => {
  it('says the spend is unpriced when tokens were burned and the cost is zero', () => {
    const said = value(usageRows(usage()), 'cost_usd')
    expect(said).toContain('not priced')
    expect(said).toContain('not zero')
    expect(said).not.toBe('0')
  })

  it('still prints a real cost when the platform ever has one', () => {
    expect(value(usageRows(usage({ cost_usd: 12.5 })), 'cost_usd')).toBe('$12.5000')
  })

  it('leaves a genuine zero alone when nothing was spent at all', () => {
    // No tokens and no cost is a desk that has not run anything, and $0.0000 is
    // the true answer there. The caveat is for the case where the tokens exist.
    expect(
      value(usageRows(usage({ input_tokens: 0, output_tokens: 0, cache_read: 0, cost_usd: 0 })), 'cost_usd'),
    ).toBe('$0.0000')
  })
})

describe('an unmeasured rate is not a measured zero', () => {
  it('says the cache rate is not measured when nothing billable was sent', () => {
    const rows = usageRows(usage({ input_tokens: 0, cache_read: 0, cache_hit_rate: 0 }))
    expect(value(rows, 'cache_hit_rate')).toContain('not measured')
  })

  it('prints a measured rate as a percentage, not as a bare fraction', () => {
    // `0.2` in a column of token counts is a number a reader has no way to
    // scale.
    expect(value(usageRows(usage()), 'cache_hit_rate')).toBe('20.0%')
  })

  it('prints a real zero rate as 0.0%, which is a measurement', () => {
    expect(value(usageRows(usage({ cache_hit_rate: 0 })), 'cache_hit_rate')).toBe('0.0%')
  })
})

describe('cost per verdict', () => {
  it('does not put the word null in the table', () => {
    const said = value(usageRows(usage()), 'dollars_per_verdict')
    expect(said).not.toContain('null')
    expect(said).toContain('no verdict has been filed')
  })

  it('prints the figure when there is one', () => {
    expect(value(usageRows(usage({ verdicts: 4, dollars_per_verdict: 3 })), 'dollars_per_verdict')).toBe(
      '$3.0000',
    )
  })
})

describe('the table reports what it was given and nothing else', () => {
  it('returns null when nothing answered, which is not an empty table', () => {
    expect(usageRows(null)).toBeNull()
    expect(usageRows(undefined)).toBeNull()
  })

  it('labels the keys in words and formats the counts', () => {
    const rows = usageRows(usage())
    expect(rows?.find((r) => r.key === 'input_tokens')?.label).toBe('prompt tokens')
    expect(value(rows, 'input_tokens')).toBe((512_000).toLocaleString())
  })

  it('passes an unknown key through rather than dropping it', () => {
    // A field the endpoint grows later must appear rather than vanish, however
    // ugly its label is.
    const rows = usageRows(usage({ something_new: 7 }))
    expect(value(rows, 'something_new')).toBe('7')
  })
})
