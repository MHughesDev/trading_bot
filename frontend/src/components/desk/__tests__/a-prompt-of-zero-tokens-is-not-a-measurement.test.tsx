/**
 * A provider that reported no usage was rendered as a run that cost nothing
 * (AGENT-012 §3.5, principle 11; Set S task S-6.12).
 *
 * `Usage::prompt_tokens()` is `input + cache_read + cache_creation`, and all
 * three default to `0` when the provider's response carries no usage block — a
 * local backend, a proxy that strips it, an older harness build. `costCalls`
 * read `num(c.prompt_tokens) ?? num(c.input_tokens)`, and `??` fires on `null`,
 * not on `0`. So a note whose `prompt_tokens` was zero never reached the
 * fallback: the provider column printed a confident **0**, and the "cumulative"
 * line summed those zeros into a prompt total of nought — over calls whose
 * `input_tokens` on this platform's own 428 recorded `usage` rows run to eleven
 * and a half thousand each.
 *
 * No model call has an empty prompt. Every one of them carries a system block.
 * A zero there is a provider that did not say, and the panel must treat it as
 * absent — an absent measurement is never a zero, and this is the direction that
 * matters most, because a zero cost is the most reassuring wrong number this
 * panel can print.
 *
 * The asymmetry is deliberate and is asserted below: a call whose **output** was
 * zero tokens is a real thing — a refusal, an immediate stop — so a zero there
 * is a number somebody could have measured and must survive.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { costCalls, type WireEvent } from '@/lib/desk/events'
import { CostTab } from '../RightPanel'

/** The shape this platform's own rows actually have: no prompt total at all. */
function liveUsage(over: Record<string, unknown> = {}, seq = 1): WireEvent {
  return {
    seq,
    kind: 'usage',
    created_at: '2026-09-21T10:00:00Z',
    content: { step: 31, latency_ms: 1379, input_tokens: 11_528, output_tokens: 49, ...over },
  }
}

describe('a zero prompt total is read as unreported, not as zero', () => {
  it('falls back to the input count when the provider sent a zero prompt total', () => {
    const [call] = costCalls([liveUsage({ prompt_tokens: 0 })])
    expect(call.provider_prompt_tokens).toBe(11_528)
  })

  it('still falls back when the field is missing altogether', () => {
    expect(costCalls([liveUsage()])[0].provider_prompt_tokens).toBe(11_528)
  })

  it('reports nothing when both counters are zero, rather than reporting zero', () => {
    const [call] = costCalls([liveUsage({ input_tokens: 0, prompt_tokens: 0 })])
    expect(call.provider_prompt_tokens).toBeNull()
  })

  it('keeps a measured zero output, because a call can return nothing', () => {
    const [call] = costCalls([liveUsage({ output_tokens: 0 })])
    expect(call.output_tokens).toBe(0)
  })

  it('prefers the provider total over the input count when both are real', () => {
    // `prompt_tokens` includes the cached part and `input_tokens` does not, so
    // the larger, provider-reported figure is the one the column is labelled
    // with.
    const [call] = costCalls([liveUsage({ prompt_tokens: 12_000 })])
    expect(call.provider_prompt_tokens).toBe(12_000)
  })
})

describe('the cache share is taken of the provider total and of nothing else', () => {
  it('does not divide a cache read by the input count', () => {
    // `input_tokens` is the prompt *minus* what the cache served, so 200/800
    // reads as 25% where the provider's own 1,000 makes it 20%. The fallback is
    // good enough for a column; it is not good enough to be a denominator.
    const [call] = costCalls([liveUsage({ input_tokens: 800, cache_read_tokens: 200 })])
    expect(call.cache_hit_pct).toBeNull()
  })

  it('computes it when the provider did report a prompt total', () => {
    const [call] = costCalls([
      liveUsage({ input_tokens: 800, prompt_tokens: 1_000, cache_read_tokens: 200 }),
    ])
    expect(call.cache_hit_pct).toBeCloseTo(20)
  })
})

describe('the cumulative line does not sum unreported prompts into a total', () => {
  it('says the prompt was not reported rather than printing nought', () => {
    render(
      <CostTab
        calls={costCalls([liveUsage({ input_tokens: 0, prompt_tokens: 0 })])}
        bytesPerTokenCalibrated={null}
        bytesPerTokenEstimate={null}
      />,
    )
    expect(screen.getByText(/prompt not reported/)).toBeInTheDocument()
  })

  it('prints the real total when the platform did report one', () => {
    render(
      <CostTab
        calls={costCalls([liveUsage({ prompt_tokens: 0 })])}
        bytesPerTokenCalibrated={null}
        bytesPerTokenEstimate={null}
      />,
    )
    expect(screen.getByText(/11,528 prompt/)).toBeInTheDocument()
  })
})

describe('bytes per token says it was never recorded, not that it measured nothing', () => {
  it('does not print "unmeasured" twice over a ratio nothing publishes', () => {
    render(<CostTab calls={costCalls([liveUsage()])} bytesPerTokenCalibrated={null} bytesPerTokenEstimate={null} />)
    expect(screen.getByTestId('bytes-per-token-unrecorded')).toHaveTextContent(/not recorded/)
    expect(screen.queryByText(/calibrated unmeasured/)).toBeNull()
  })

  it('prints the figures if anything ever serves them', () => {
    render(<CostTab calls={costCalls([liveUsage()])} bytesPerTokenCalibrated={3.9} bytesPerTokenEstimate={4} />)
    expect(screen.getByText(/calibrated 3\.9 · harness estimate 4/)).toBeInTheDocument()
    expect(screen.queryByTestId('bytes-per-token-unrecorded')).toBeNull()
  })
})
