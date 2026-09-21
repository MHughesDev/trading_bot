/**
 * The Tools panel's "searched for" section was handed a literal `[]`
 * (AGENT-012 §3.5; Set S task S-6.11).
 *
 * The section existed, the heading existed, and `Conversation.tsx` passed
 * `searchedFor={[]}` — so it was hidden behind `searchedFor.length > 0` and had
 * never rendered once. The nearest real thing on the panel was the exposure
 * note's `pinned`, and labelling that as "searched for" would have been wrong in
 * the direction that matters: pinned tools are ones the model *named* and that
 * were held ahead of namespace filling, so a search that matched **nothing**
 * pins nothing and would have disappeared entirely.
 *
 * The note was on the stream the whole time. `drive.rs` answers `search_tools`
 * itself and pushes `Note::CoreTool { name: "search_tools", detail }`;
 * `local_driver.rs` appends it under kind `core_tool`. The harness's own comment
 * on `Note::Exposure` says where its worst observed behaviours started — "a tool
 * the model had just searched for losing its slot to an alphabetical sibling" —
 * which is a sentence nobody could check against this panel, because the panel
 * could not show a search.
 *
 * A search that returned nothing is the case worth finding, so `[]` and `null`
 * are kept apart: `[]` is "the registry had no match", `null` is "the desk could
 * not read the note", and only the first is a fact about the run.
 */
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { toolSearches, type WireEvent } from '@/lib/desk/events'
import { ToolsTab } from '../RightPanel'

function note(detail: string, over: Record<string, unknown> = {}): WireEvent {
  return {
    seq: 1,
    kind: 'core_tool',
    created_at: '2026-09-21T10:00:00Z',
    content: { note: 'core_tool', name: 'search_tools', detail, ...over },
  }
}

describe('a run’s tool searches are read off its own stream', () => {
  it('reads the query and what it returned out of the note', () => {
    const [s] = toolSearches([note('"realised volatility" -> get_bars, preview_signals')])
    expect(s.query).toBe('realised volatility')
    expect(s.found).toEqual(['get_bars', 'preview_signals'])
  })

  it('keeps a search that matched nothing apart from a note it could not read', () => {
    const [miss] = toolSearches([note('no tool matches "order book depth"')])
    expect(miss.query).toBe('order book depth')
    // Measured: the registry answered, with nothing.
    expect(miss.found).toEqual([])

    const [garbled] = toolSearches([note('something nobody has written yet')])
    // Unread: a fact about this panel, not about the run. It must not read as a
    // search that found nothing.
    expect(garbled.found).toBeNull()
    expect(garbled.query).toBeNull()
    // …and the note is still carried, so an unrecognised shape is legible
    // rather than dropped.
    expect(garbled.detail).toBe('something nobody has written yet')
  })

  it('ignores the other core tools, which are not searches', () => {
    const events: WireEvent[] = [
      note('"vol" -> get_bars'),
      {
        seq: 2,
        kind: 'core_tool',
        created_at: '2026-09-21T10:01:00Z',
        content: { note: 'core_tool', name: 'record_finding', detail: '"vol" -> anything' },
      },
      { seq: 3, kind: 'exposure', created_at: '2026-09-21T10:02:00Z', content: { pinned: ['run_backtest'] } },
    ]
    const found = toolSearches(events)
    expect(found).toHaveLength(1)
    expect(found[0].query).toBe('vol')
  })

  it('reads the trajectory’s spelling of the same note', () => {
    // `{core_tool, detail}` into the trajectory, `{note, name, detail}` into
    // `agent_messages`. One field name apart, and the panel would have shown
    // nothing on whichever stream it was not written for.
    const [s] = toolSearches([
      {
        seq: 1,
        kind: 'core_tool',
        created_at: '2026-09-21T10:00:00Z',
        content: { core_tool: 'search_tools', detail: '"vol" -> get_bars' },
      },
    ])
    expect(s.query).toBe('vol')
  })

  it('keeps every search, because a later one is a correction of an earlier one', () => {
    const found = toolSearches([
      note('no tool matches "orderbook"'),
      { ...note('"order book" -> get_bars'), seq: 2 },
    ])
    expect(found.map((s) => s.query)).toEqual(['orderbook', 'order book'])
  })
})

describe('the panel says what was searched for, or that nothing searched', () => {
  it('renders the query and the tools it turned up', () => {
    render(
      <ToolsTab
        mode="research"
        exposed={null}
        catalogue={[]}
        searchedFor={toolSearches([note('"realised volatility" -> get_bars')])}
      />,
    )
    const list = screen.getByTestId('tool-searches')
    expect(list.textContent).toContain('realised volatility')
    expect(list.textContent).toContain('get_bars')
    expect(screen.queryByTestId('no-tool-search')).toBeNull()
  })

  it('shows a search that matched nothing rather than dropping it', () => {
    render(
      <ToolsTab
        mode="research"
        exposed={null}
        catalogue={[]}
        searchedFor={toolSearches([note('no tool matches "order book depth"')])}
      />,
    )
    expect(screen.getByTestId('search-matched-nothing')).toBeInTheDocument()
    expect(screen.getByTestId('tool-searches').textContent).toContain('order book depth')
  })

  it('says no search is recorded, rather than hiding the section', () => {
    // The old behaviour: `searchedFor.length > 0 &&`, so an empty list rendered
    // no heading at all and the reader could not tell "it did not search" from
    // "searching is not recorded here".
    render(<ToolsTab mode="research" exposed={null} catalogue={[]} searchedFor={[]} />)
    const said = screen.getByTestId('no-tool-search').textContent ?? ''
    expect(said).toContain('search_tools')
    expect(said).toContain('has not gone looking')
    expect(screen.queryByTestId('tool-searches')).toBeNull()
  })
})
