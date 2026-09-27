/**
 * Four token counters printed as `0` by a table that holds no rows.
 *
 * `/api/agent/usage` is four `COALESCE(SUM(…),0)` over `llm_usage`. On this
 * platform, read 2026-09-21:
 *
 * ```
 * llm_usage            0
 * agent_runs          55      (27 with tokens_in > 0)
 * agent_sessions       0
 * SUM(agent_runs.tokens_in)   2,526,695
 * SUM(agent_runs.tokens_out)     27,512
 * ```
 *
 * So the endpoint answers four zeros for a desk that has demonstrably burned
 * two and a half million prompt tokens — because only the LLM proxy writes
 * `llm_usage` and the desk driver does not go through it, and because the
 * numbers that *were* recorded sit on `agent_runs` in columns the endpoint does
 * not read.
 *
 * `cost_usd` and `cache_hit_rate` already had branches for "nobody has this
 * number". The four counts did not: they fell through to `n.toLocaleString()`,
 * which renders `0`. A `0` in a count column is indistinguishable from a
 * measurement, so the surface asserted "nothing spent" with no evidence for it.
 *
 * What the component can honestly say is neither "0" nor "nothing spent": it is
 * that the one table this endpoint reads recorded nothing, which leaves the
 * question open rather than answering it. That caveat has to be **stated**,
 * because a reader cannot infer "unread" from a digit.
 *
 * Every test in this file fails against the code as it stood before 2026-09-21.
 */
import { render, screen, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { describe, expect, it, vi } from 'vitest'

import { Usage } from '../Operations'
import { nothingRecordedInLlmUsage, usageRows } from '@/components/desk/Operations.helpers'

const usageCall = vi.hoisted(() => vi.fn())
// The Spend route renders the barren panel beside the table; it is not under
// test here, so it is given the shape it expects and left alone.
const barren = vi.hoisted(() =>
  vi.fn(() => Promise.resolve({ rows: [], days: 30, unresolved: [] })),
)
vi.mock('@/api/desk', () => ({ deskApi: { usage: usageCall, barren } }))

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

/**
 * Exactly what the live platform returns for this desk today.
 *
 * `cache_hit_rate` was `0` here and is now `null`, because the server stopped
 * sending a zero for a rate nobody measured: `usage_body` sends `None` unless
 * `billable_prompt > 0`. The fixture claims to be the live shape, so it has to
 * follow the live shape — a fixture that has drifted is a test asserting
 * against a response the platform can no longer produce.
 */
function emptyTable(over: Record<string, unknown> = {}): Record<string, unknown> {
  return usage({
    input_tokens: 0,
    output_tokens: 0,
    cache_read: 0,
    cache_write: 0,
    cost_usd: 0,
    cache_hit_rate: null,
    ...over,
  })
}

function value(rows: ReturnType<typeof usageRows>, key: string): string {
  const hit = rows?.find((r) => r.key === key)
  expect(hit, `no row for ${key}`).toBeDefined()
  return hit?.value ?? ''
}

const COUNTERS = ['input_tokens', 'output_tokens', 'cache_read', 'cache_write']

describe('a token count of zero from an empty table is not a token count of zero', () => {
  it('prints no bare zero for any of the four counters when the table recorded nothing', () => {
    const rows = usageRows(emptyTable())
    for (const key of COUNTERS) {
      const said = value(rows, key)
      expect(said, key).not.toBe('0')
      expect(said, key).not.toBe((0).toLocaleString())
    }
  })

  it('says which table came back empty rather than just "not recorded"', () => {
    // "not recorded" on its own is as unreadable as "0": it does not say what
    // was asked, so it cannot say what is still open. The reading that matters
    // is *which* table holds nothing, because that is the difference between a
    // desk that spent nothing and a desk nothing was watching.
    const said = value(usageRows(emptyTable()), 'input_tokens')
    expect(said).toContain('not recorded')
    expect(said).toContain('llm_usage')
    expect(said).toContain('Not a measured zero')
  })

  it('does not claim the desk spent nothing', () => {
    for (const key of [...COUNTERS, 'cost_usd']) {
      const said = value(usageRows(emptyTable()), key).toLowerCase()
      expect(said, key).not.toContain('nothing spent')
      expect(said, key).not.toContain('no tokens')
    }
  })
})

describe('a real zero beside a real number is still printed as a number', () => {
  it('prints a measured zero as 0 when the table recorded something else', () => {
    // The caveat is for an empty table, not for a counter that genuinely came
    // back at nought. A `cache_write` of 0 next to 512,000 prompt tokens is a
    // measurement, and rendering it as "not recorded" would be the same defect
    // pointed the other way.
    expect(value(usageRows(usage({ cache_write: 0 })), 'cache_write')).toBe((0).toLocaleString())
  })

  it('still prints the counts when they are there', () => {
    const rows = usageRows(usage())
    expect(value(rows, 'input_tokens')).toBe((512_000).toLocaleString())
    expect(value(rows, 'cache_write')).toBe((4_000).toLocaleString())
  })

  it('needs all four counters at zero, not any one of them', () => {
    expect(nothingRecordedInLlmUsage(usage())).toBe(false)
    expect(nothingRecordedInLlmUsage(usage({ input_tokens: 0 }))).toBe(false)
    expect(nothingRecordedInLlmUsage(emptyTable())).toBe(true)
  })

  it('treats a missing counter as unreported rather than as an empty table', () => {
    // A field the endpoint stops sending is not evidence that the table is
    // empty, and must not be read as such.
    expect(nothingRecordedInLlmUsage(usage({ input_tokens: undefined }))).toBe(false)
    expect(nothingRecordedInLlmUsage(null)).toBe(false)
    expect(value(usageRows(usage({ cache_write: null })), 'cache_write')).toBe('not reported')
  })
})

describe('the cost and the rate derived from an empty table are unread too', () => {
  it('does not print $0.0000 when nothing was recorded to price', () => {
    const said = value(usageRows(emptyTable()), 'cost_usd')
    expect(said).not.toBe('$0.0000')
    expect(said).toContain('llm_usage')
  })

  it('does not claim a prompt was or was not billed when no row says either', () => {
    const said = value(usageRows(emptyTable()), 'cache_hit_rate')
    expect(said).not.toContain('billed')
    expect(said).not.toBe('0.0%')
  })

  it('leaves the verdict rows alone, which come from a different table', () => {
    // `verdicts` counts `agent_sessions`, not `llm_usage`. An empty
    // `llm_usage` says nothing about it, so its existing reading must survive.
    expect(value(usageRows(emptyTable()), 'dollars_per_verdict')).toContain('no verdict has been filed')
    expect(value(usageRows(emptyTable()), 'verdicts')).toBe((0).toLocaleString())
  })
})

function renderUsage(payload: Record<string, unknown>) {
  usageCall.mockResolvedValue({ data: payload })
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <Usage />
    </QueryClientProvider>,
  )
}

describe('the caveat is stated on the surface, not left to be inferred', () => {
  it('names the table, its one writer, and where the run totals actually live', async () => {
    renderUsage(emptyTable())
    const note = await screen.findByTestId('usage-unrecorded-note')
    const said = note.textContent ?? ''
    // The three things a reader needs to know what is still open: which table
    // was read, why it can be empty, and what the zeros are not.
    expect(said).toContain('llm_usage')
    expect(said).toContain('LLM proxy')
    expect(said).toContain('agent_runs')
    expect(said).toContain('unread, not as nil')
  })

  it('does not print the caveat when the table did record something', async () => {
    renderUsage(usage())
    await waitFor(() => expect(screen.getByText((512_000).toLocaleString())).toBeInTheDocument())
    expect(screen.queryByTestId('usage-unrecorded-note')).toBeNull()
  })

  it('does not print the caveat instead of the "did not answer" empty state', async () => {
    // A request that failed and a table that is empty are different facts, and
    // the surface already had a sentence for the first one.
    usageCall.mockRejectedValue(new Error('boom'))
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <QueryClientProvider client={qc}>
        <Usage />
      </QueryClientProvider>,
    )
    await waitFor(() => expect(screen.getByText(/did not answer/)).toBeInTheDocument())
    expect(screen.queryByTestId('usage-unrecorded-note')).toBeNull()
  })
})
