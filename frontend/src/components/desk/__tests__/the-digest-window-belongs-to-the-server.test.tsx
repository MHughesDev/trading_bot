/**
 * "Since you were away" filtered the server's timestamps against the browser's
 * clock, and the line it dropped first was the spend (AGENT-012 §2.2a, §19;
 * Set S task S-6.24).
 *
 * `linesInGap(data.lines, lastVisit, now)` compared every `at` — all of them
 * stamped by the server — against `localStorage`'s marker and the browser's
 * `Date.now()`. Two clocks, and the difference between them was subtracted from
 * the digest with no trace.
 *
 * It bites hardest on exactly one line. `crates/api/src/routes/desk_digest.rs`
 * stamps the budget line at `until = Utc::now()`, which is by construction the
 * newest timestamp in the response — so a server clock even a second ahead of
 * the browser's put it past `now` and the filter removed it. The operator then
 * read every other line and no spend figure, which is a window of spending
 * rendered as a window of silence. That is the route's own stated
 * non-negotiable — *"An absent measurement is never a zero"*, written in the
 * `Budget` type's doc comment — undone on the way to the screen.
 *
 * The server sends the window it covered. Filtering against that window cannot
 * subtract anything the server put in it.
 *
 * # The four-way budget truth table, as the panel has to render it
 *
 * `read_budget` returns four things and `budget_line` turns only two of them
 * into a line:
 *
 * * `Priced` — a dollar figure, the only branch that has one;
 * * `TokensOnly` — tokens in and out, saying in so many words that the dollars
 *   are absent rather than zero;
 * * `Unmeasured` — **no line at all**, deliberately. Runs were active and
 *   nothing recorded what they cost, and the only trace of that anywhere in the
 *   response is a row in `kinds_not_covered`;
 * * `Idle` — no line and no gap, because no run was active and silence is then
 *   the true report.
 *
 * So the `budget` gap row is not decoration. Under `Unmeasured` it is the whole
 * message, and it used to be joined into a paragraph with three other reasons
 * and set in `.sub`.
 */
import { render, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { describe, expect, it } from 'vitest'

import type { AwayDigest } from '@/api/desk'
import { AwayDigestPanel, digestWindow } from '../AwayDigest'

const NOW = Date.parse('2026-09-20T18:00:00Z')
const LAST_VISIT = NOW - 8 * 60 * 60 * 1000

/** The budget line as the route builds it: stamped at the server's `until`. */
function budgetLine(at: number) {
  return {
    id: 'budget',
    at: new Date(at).toISOString(),
    kind: 'budget' as const,
    text: '412000 tokens in and 38000 out over 3 run(s) that finished while you were away. No priced model call was recorded for this window, so what it cost in dollars is absent rather than zero',
    href: '/desk/usage',
  }
}

function digest(over: Partial<AwayDigest> = {}): AwayDigest {
  return {
    since: new Date(LAST_VISIT).toISOString(),
    // The server is four seconds ahead of this browser. Nothing exotic: an
    // unsynchronised container clock does this all day.
    until: new Date(NOW + 4000).toISOString(),
    lines: [budgetLine(NOW + 4000)],
    kinds_not_covered: [],
    ...over,
  }
}

function panel(data: AwayDigest) {
  return render(
    <MemoryRouter>
      <AwayDigestPanel data={data} lastVisit={LAST_VISIT} now={NOW} />
    </MemoryRouter>,
  )
}

describe('the digest covers the window the server says it covered', () => {
  it('keeps the spend line when the server’s clock is ahead of the browser’s', () => {
    panel(digest())
    expect(screen.getByText(/absent rather than zero/)).toBeInTheDocument()
  })

  it('does not vanish entirely over a window in which money was spent', () => {
    // The failure the line above prevents, and it is worse than a wrong
    // sentence. With the budget line filtered out by the clock and nothing else
    // in the digest, `hasSomethingToSay` saw zero lines and zero gaps and the
    // panel returned `null` — so the operator came back to a spending window
    // and was shown no panel at all, which they read as "nothing happened".
    //
    // Asserting `queryByTestId('digest-quiet')` is null would pass on that
    // empty render and prove nothing; this asserts the panel is there.
    const { container } = panel(digest())
    expect(container.textContent).not.toBe('')
    expect(screen.getByText('Since you were away')).toBeInTheDocument()
    expect(screen.queryByTestId('digest-quiet')).toBeNull()
  })

  it('takes the window from the response, and the browser’s marker only without one', () => {
    expect(digestWindow(digest(), LAST_VISIT, NOW)).toEqual([LAST_VISIT, NOW + 4000])
    // A response that carried no window at all, which nothing should send but
    // the panel must survive.
    const none = { since: '', until: '' } as Pick<AwayDigest, 'since' | 'until'>
    expect(digestWindow(none, LAST_VISIT, NOW)).toEqual([LAST_VISIT, NOW])
  })

  it('still drops a line older than the window, which is the rule’s other half', () => {
    const stale = digest({
      lines: [budgetLine(NOW + 4000), { ...budgetLine(LAST_VISIT - 1000), id: 'old' }],
    })
    panel(stale)
    // One rendering of the budget text, not two.
    expect(screen.getAllByText(/absent rather than zero/)).toHaveLength(1)
  })
})

describe('an unmeasured spend is a named absence, not a missing line', () => {
  it('renders the budget gap as its own row rather than inside a joined paragraph', () => {
    // `Budget::Unmeasured` emits no line, so this row is the only trace of it.
    panel(
      digest({
        lines: [],
        kinds_not_covered: [
          {
            kind: 'budget',
            why: '3 run(s) were active while you were away and nothing recorded what they cost — no priced model call, and no token counter on the runs themselves. That is an absent measurement and not a spend of zero, so no figure is shown.',
          },
        ],
      }),
    )
    const row = screen.getByTestId('digest-gap-budget')
    expect(row.textContent).toContain('absent measurement and not a spend of zero')
    expect(row.tagName.toLowerCase()).toBe('li')
  })

  it('says the uncovered kinds are what it could not see, not what did not happen', () => {
    panel(digest({ lines: [], kinds_not_covered: [{ kind: 'monitor', why: 'the log did not answer' }] }))
    expect(screen.getByTestId('digest-blind-spots').textContent).toContain('not what did not happen')
  })

  it('stays silent about spend when the server said nothing was active', () => {
    // `Idle`: no line and no gap. Silence is the true report, and inventing a
    // "$0 spent" here would be the same defect pointing the other way.
    const { container } = panel(digest({ lines: [], kinds_not_covered: [] }))
    expect(container.textContent).toBe('')
  })
})
