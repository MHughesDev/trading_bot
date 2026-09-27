/**
 * The better sentence about the cache hit rate had become unreachable, because
 * the server changed which value it uses to mean "nobody has this number".
 *
 * `usageRows` had two readings for `cache_hit_rate`:
 *
 * * `n === null` → **"not reported"**, checked first;
 * * `billablePrompt === 0` → **"not measured — no prompt has been billed yet"**.
 *
 * The second is the one worth printing: it says *why* there is no rate, and it
 * says it from evidence in the same payload. It was written when
 * `/api/agent/usage` answered `0.0` for an unbilled desk. It does not any more.
 * `usage_body` in `crates/api/src/routes/agent_sessions.rs` now reads:
 *
 * ```rust
 * // A rate over nothing billable is not a rate of zero.
 * "cache_hit_rate": if billable_prompt > 0 { Some(...) } else { None },
 * ```
 *
 * So the backend fixed its own half — and in doing so routed every unbilled
 * response into the `n === null` branch above it, where the desk printed the
 * vaguer "not reported" and the sentence that explains it could no longer be
 * reached by any response the platform is capable of producing. Two honest
 * changes, one on each side, that together were a regression.
 *
 * The fix is the order: `billablePrompt === 0` is the *evidence* for the
 * sentence, so it is the condition; `n === null` with billable tokens behind it
 * is some other absence and keeps the vaguer word.
 *
 * WHICH OF THESE FAILED BEFORE. Checked by running them against the previous
 * `usageRows`, not assumed: three of the five failed. The two marked
 * `[held before]` are here so the reorder cannot quietly swallow the readings
 * it had to step around.
 */
import { describe, expect, it } from 'vitest'
import { usageRows } from '@/components/desk/Operations.helpers'


/** A response with real traffic on it, so `unrecorded` is false throughout. */
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

function value(raw: Record<string, unknown>): string {
  const hit = usageRows(raw)?.find((r) => r.key === 'cache_hit_rate')
  expect(hit, 'no row for cache_hit_rate').toBeDefined()
  return hit?.value ?? ''
}

describe('an absent cache hit rate is explained, not just noted', () => {
  it('[was failing] says why there is no rate when the server sends null for an unbilled desk', () => {
    // The shape the platform actually produces now: nothing billable, so the
    // rate is `null` rather than `0.0`. Output tokens are non-zero, so this is
    // not the empty-table case — llm_usage has rows, they just carry no
    // billable prompt.
    const said = value(usage({ input_tokens: 0, cache_read: 0, cache_hit_rate: null }))
    expect(said).toBe('not measured — no prompt has been billed yet')
    expect(said).not.toBe('not reported')
  })

  it('[was failing] keeps saying so through the old zero, which older rows still carry', () => {
    // `llm_usage` rows written before the server changed still hold `0.0`, and
    // both spellings of the same fact must read the same on the page.
    expect(value(usage({ input_tokens: 0, cache_read: 0, cache_hit_rate: 0 }))).toBe(
      value(usage({ input_tokens: 0, cache_read: 0, cache_hit_rate: null })),
    )
  })

  it('[held before] a null with billable prompt behind it is some other absence', () => {
    // Not "no prompt has been billed" — half a million were. Something else
    // failed to report, and the desk must not name a cause it did not observe.
    expect(value(usage({ cache_hit_rate: null }))).toBe('not reported')
  })

  it('[held before] a measured rate is still a percentage, not a bare fraction', () => {
    expect(value(usage())).toBe('20.0%')
  })

  it('[was failing] an empty llm_usage outranks both, because nothing was read at all', () => {
    // With no rows, "no prompt has been billed" is a claim about billing that
    // nothing supports — only that no row says so. That branch stays first.
    //
    // This one was a surprise. The `n === null` short-circuit did not only
    // hide the "not billed" sentence: it sat above the `unrecorded` check too,
    // so on the live platform — where `llm_usage` has zero rows *and* the rate
    // now arrives as `null` — the table printed a bare "not reported" where the
    // page has a whole paragraph about the one source it reads holding nothing.
    // Two sentences were unreachable behind that line, not one.
    const said = value(
      usage({ input_tokens: 0, output_tokens: 0, cache_read: 0, cache_write: 0, cache_hit_rate: null }),
    )
    expect(said).toContain('llm_usage')
    expect(said).not.toContain('billed')
  })
})
