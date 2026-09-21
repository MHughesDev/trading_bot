/**
 * A third of the searches this platform has recorded rendered as
 * "a search this panel could not read" (AGENT-012 §3.5; Set S task S-6.11).
 *
 * The harness answers `search_tools` itself and writes the outcome as a
 * **rendered string**, not a struct: `format!("{query:?} -> {}", found)`. The
 * spelling of that arrow is therefore whatever the build that wrote the row
 * used, and this platform's `agent_messages` holds two spellings — four rows
 * with the ASCII `->` today's `drive.rs` emits, and two written with `→`
 * (U+2192) by an earlier one. Old rows are never rewritten.
 *
 * `SEARCH_HIT` matched only `->`, so the Unicode rows fell to the parser's
 * "shape not recognised" branch and the Tools tab printed *a search this panel
 * could not read* over a search that was recorded perfectly well. That inverts
 * the one distinction the section exists to make: `found: []` (a search that
 * matched nothing — why a model goes round in circles naming tools it was never
 * offered) versus `found: null` (the desk failing). A parser bug that presents
 * as the run being illegible is the worst of the three outcomes, because it
 * points the reader at the agent.
 *
 * Counted against the live database on 2026-09-21: 2 of 6.
 */
import { describe, expect, it } from 'vitest'

import { toolSearches, type WireEvent } from '@/lib/desk/events'

function note(detail: string, seq = 1): WireEvent {
  return {
    seq,
    kind: 'core_tool',
    created_at: '2026-09-21T10:00:00Z',
    content: { note: 'core_tool', name: 'search_tools', detail, step: 3 },
  }
}

describe('a search note is read whichever arrow the build that wrote it used', () => {
  it('reads the ASCII arrow the current harness emits', () => {
    const [s] = toolSearches([note('"list_instruments" -> list_instruments')])
    expect(s.query).toBe('list_instruments')
    expect(s.found).toEqual(['list_instruments'])
  })

  it('reads the Unicode arrow an older build left in the table', () => {
    // The exact bytes of a row on this platform.
    const [s] = toolSearches([note('"crypto_spot_cex" → list_asset_classes')])
    expect(s.query).toBe('crypto_spot_cex')
    expect(s.found).toEqual(['list_asset_classes'])
    // Not the desk's own failure branch, which is what it used to be.
    expect(s.found).not.toBeNull()
  })

  it('reads the long arrow and the double arrow too, so the fix is not one character wide', () => {
    expect(toolSearches([note('"bars" ⟶ get_bars, list_instruments')])[0].found).toEqual([
      'get_bars',
      'list_instruments',
    ])
    expect(toolSearches([note('"bars" ⇒ get_bars')])[0].found).toEqual(['get_bars'])
  })

  it('keeps a search that matched nothing distinct from a note it could not parse', () => {
    // The two outcomes this whole section exists to tell apart. `[]` is a fact
    // about the registry; `null` is a fact about the desk.
    const [miss] = toolSearches([note('no tool matches "a tool that is not there"')])
    expect(miss.query).toBe('a tool that is not there')
    expect(miss.found).toEqual([])

    const [garbled] = toolSearches([note('something nobody has ever written')])
    expect(garbled.query).toBeNull()
    expect(garbled.found).toBeNull()
  })

  it('does not treat an arrow inside the query itself as the separator', () => {
    // The query is delimited by its quotes, not by the first arrow, so a search
    // for a phrase containing one still splits in the right place.
    const [s] = toolSearches([note('"a -> b" -> get_bars')])
    expect(s.query).toBe('a -> b')
    expect(s.found).toEqual(['get_bars'])
  })
})
