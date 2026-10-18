/**
 * The Cost and Tools tabs were fed literals over a stream that carried the
 * answer (AGENT-012 §3.5; Set S tasks S-6.11, S-6.12).
 *
 * `Conversation.tsx` passed `CostTab calls={[]}` and `ToolsTab exposed={null}`.
 * Both panels then explained their own emptiness — "No model call has been
 * accounted yet", "The harness has not reported an exposure note" — and both
 * sentences were false: the local driver writes a `usage` note per model call
 * and an `exposure` note per step, and both were already on the conversation's
 * own event stream. They sat in `RAW_KINDS`, folded behind the details
 * disclosure, so nothing read them and nothing looked broken. That is the same
 * defect the catalogue had: a surface that explains its emptiness with a reason
 * that stopped being true tells the reader not to look.
 *
 * The second half of this file is the arithmetic the wiring must not get wrong.
 * The `usage` note carries a prompt total and **no layer split**, and the
 * platform has **no price table** (OQ-16) — so a four-column table of zeros and
 * a `$0.000` cumulative were both one careless spread away, and both are the
 * most reassuring possible wrong answer.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { costCalls, latestExposure, type WireEvent } from '@/lib/desk/events'
import { CostTab, ToolsTab } from '../RightPanel'

function usage(over: Record<string, unknown> = {}, seq = 1): WireEvent {
  return {
    seq,
    kind: 'usage',
    created_at: '2026-09-21T10:00:00Z',
    content: {
      step: 1,
      input_tokens: 800,
      output_tokens: 120,
      prompt_tokens: 1_000,
      est_prompt_tokens: 1_100,
      cache_read_tokens: 200,
      latency_ms: 4_200,
      ...over,
    },
  }
}

describe('the usage note the stream already carried', () => {
  it('reads one row per call, with the provider count and the harness estimate', () => {
    const [call] = costCalls([usage()])
    expect(call).toBeDefined()
    expect(call.step).toBe(1)
    expect(call.provider_prompt_tokens).toBe(1_000)
    expect(call.estimated_prompt_tokens).toBe(1_100)
    expect(call.output_tokens).toBe(120)
    expect(call.latency_ms).toBe(4_200)
  })

  it('leaves the layer split absent rather than inventing four zeros', () => {
    // The note records one prompt total. `{charter: 0, schemas: 0, body: 0}` is
    // a measurement nobody made, and the old table printed exactly that.
    expect(costCalls([usage()])[0].layers).toBeNull()
  })

  it('leaves the money absent, because nothing in the platform prices a call', () => {
    expect(costCalls([usage()])[0].usd_tenth_cents).toBeNull()
  })

  it('computes the cache share only when there is a prompt total to take it of', () => {
    expect(costCalls([usage()])[0].cache_hit_pct).toBeCloseTo(20)
    expect(costCalls([usage({ prompt_tokens: 0 })])[0].cache_hit_pct).toBeNull()
    expect(costCalls([usage({ cache_read_tokens: undefined })])[0].cache_hit_pct).toBeNull()
    // Zero cache reads out of a real prompt IS measured, and is 0 — not absent.
    expect(costCalls([usage({ cache_read_tokens: 0 })])[0].cache_hit_pct).toBe(0)
  })

  it('keeps a retried step as two rows rather than collapsing it', () => {
    // A step rejected by a rung and re-called is two calls, and the second is
    // the one worth finding. Keyed by step alone it would have been one row.
    const rows = costCalls([usage({}, 1), usage({}, 2)])
    expect(rows.length).toBe(2)
  })

  it('ignores every other kind on the stream', () => {
    const rows = costCalls([
      { seq: 1, kind: 'assistant', created_at: '', content: { text: 'hello' } },
      usage({}, 2),
    ])
    expect(rows.length).toBe(1)
  })
})

describe('the Cost tab tells three states apart', () => {
  it('says nobody asked when there is no run to read', () => {
    render(<CostTab calls={null} bytesPerTokenCalibrated={null} bytesPerTokenEstimate={null} />)
    expect(screen.getByText(/has not been read/)).toBeInTheDocument()
  })

  it('prices and splits a call only when its own note says so (S-6.12)', () => {
    const calls = costCalls([
      usage({ usd_tenth_cents: 250, layers: { charter: 10, schemas: 20, body: 900, output: 70 } }),
      usage({ layers: { charter: 10, schemas: 20 } }, 2),
    ])
    expect(calls[0].usd_tenth_cents).toBe(250)
    expect(calls[0].layers).toEqual({ charter: 10, schemas: 20, body: 900, output: 70 })
    // A half-split is not drawn as a whole one, and no figure is not a free call.
    expect(calls[1].layers).toBeNull()
    expect(calls[1].usd_tenth_cents).toBeNull()
    render(<CostTab calls={calls} bytesPerTokenCalibrated={null} bytesPerTokenEstimate={null} />)
    expect(screen.getAllByTestId('layer-split')[0].textContent).toBe('10 · 20 · 900 · 70')
    expect(screen.getAllByTestId('layer-split')[1].textContent).toBe('not split')
    // S-6.12's acceptance, as a stated share: 930 estimated of the provider's 1,000.
    expect(screen.getByTestId('layer-reassembly').textContent).toMatch(/930 tokens against\s+the provider's 1,000 over 1 call \(93%\)/)
    expect(screen.queryByTestId('cost-unpriced')).toBeNull()
  })

  it('says no call has been made when the run has made none', () => {
    render(<CostTab calls={[]} bytesPerTokenCalibrated={null} bytesPerTokenEstimate={null} />)
    expect(screen.getByText(/no model call yet/)).toBeInTheDocument()
  })

  it('never prints $0.000 for a run nothing priced', () => {
    render(
      <CostTab
        calls={costCalls([usage()])}
        bytesPerTokenCalibrated={null}
        bytesPerTokenEstimate={null}
      />,
    )
    expect(screen.queryByText(/\$0\.000/)).toBeNull()
    expect(screen.getByTestId('cost-unpriced').textContent).toContain('not priced')
    // …and the tokens it *did* measure are on the screen, so the assertion
    // above cannot be passing because the tab rendered nothing.
    expect(screen.getByText(/1,000 prompt · 120 output/)).toBeInTheDocument()
  })

  it('says the layer split is unbuilt rather than showing it as zero', () => {
    render(
      <CostTab
        calls={costCalls([usage()])}
        bytesPerTokenCalibrated={null}
        bytesPerTokenEstimate={null}
      />,
    )
    expect(screen.getByTestId('layers-unsplit').textContent).toContain('unbuilt, not zero')
  })

  it('shows the estimate against the provider count, signed', () => {
    render(
      <CostTab
        calls={costCalls([usage()])}
        bytesPerTokenCalibrated={null}
        bytesPerTokenEstimate={null}
      />,
    )
    // 1,100 estimated against 1,000 billed is a 100-token over-estimate, which
    // is §3.5's last line and the thing that calibrates the estimator.
    expect(screen.getByText(/1100 \(\+100\)/)).toBeInTheDocument()
  })

  it('says the cache share is unknown rather than nil when nothing reported one', () => {
    render(
      <CostTab
        calls={costCalls([usage({ prompt_tokens: 0, cache_read_tokens: undefined })])}
        bytesPerTokenCalibrated={null}
        bytesPerTokenEstimate={null}
      />,
    )
    expect(screen.getByTestId('cache-unreported').textContent).toContain('unknown rather than nil')
  })
})

describe('the exposure note says what the step saw, and why that set', () => {
  function exposure(over: Record<string, unknown> = {}, seq = 1): WireEvent {
    return {
      seq,
      kind: 'exposure',
      created_at: '2026-09-21T10:00:00Z',
      content: {
        note: 'exposure',
        namespaces: ['backtest'],
        exposed: ['run_backtest', 'get_bars'],
        budget: 8,
        pinned: ['get_bars'],
        plan_step: 'measure the edge',
        ...over,
      },
    }
  }

  it('reads the latest note, not the first', () => {
    // "exposed this step" answered with step one's exposure would be a wrong
    // answer rather than a stale one.
    const note = latestExposure([exposure({}, 1), exposure({ exposed: ['deflated_sharpe'] }, 2)])
    expect(note?.exposed).toEqual(['deflated_sharpe'])
  })

  it('is null when the run has emitted none, rather than an empty list', () => {
    expect(latestExposure([usage()])).toBeNull()
  })

  it('carries the budget, namespaces and pins that explain the set', () => {
    const note = latestExposure([exposure()])
    expect(note?.budget).toBe(8)
    expect(note?.namespaces).toEqual(['backtest'])
    expect(note?.pinned).toEqual(['get_bars'])
    expect(note?.plan_step).toBe('measure the edge')
  })

  it('renders the why-line in the Tools tab beside the list', () => {
    render(
      <ToolsTab
        mode="research"
        exposed={['run_backtest', 'get_bars']}
        exposure={latestExposure([exposure()])}
        catalogue={[]}
        searchedFor={[]}
      />,
    )
    const why = screen.getByTestId('exposure-why')
    expect(why.textContent).toContain('2 of a 8 budget')
    expect(why.textContent).toContain('backtest')
    expect(why.textContent).toContain('get_bars')
  })

  it('says an absent note means no step has run, not that nothing was shown', () => {
    render(<ToolsTab mode="research" exposed={null} catalogue={[]} searchedFor={[]} />)
    expect(screen.getByTestId('exposure-absent').textContent).toContain(
      'not that the model was shown nothing',
    )
  })
})
