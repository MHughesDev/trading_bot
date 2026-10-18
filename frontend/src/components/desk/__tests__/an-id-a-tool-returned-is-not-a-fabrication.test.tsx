/**
 * S-6.9 — every id resolves on click; an unresolvable id is struck through.
 *
 * The drivers store a tool result as
 * `{ name, is_error, content: { summary, ids: { key: id }, n, se, detail, handle } }`.
 * The resolver read `detail.ids` and `detail.handle` — the outer object — where
 * neither has ever been, so every id a tool returned rendered struck through as
 * "not found": the fabrication mark, on ids the platform produced itself. And
 * the Evidence tab listed only the working state, whose route is not mounted, so
 * it listed nothing at all.
 */
import { describe, expect, it } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import { toCards, type WireEvent } from '@/lib/desk/events'
import { evidenceEntries, streamEvidence, storedResult } from '@/lib/desk/evidence'
import { makeResolver } from '../Conversation.helpers'
import { EvidenceTab } from '../RightPanel'

const result = (seq: number, content: unknown, name = 'run_backtest'): WireEvent => ({
  seq,
  kind: 'tool_result',
  created_at: '2026-10-03T10:00:00Z',
  content: { name, is_error: false, content: content as Record<string, unknown> },
})

const stream: WireEvent[] = [
  { seq: 1, kind: 'assistant', created_at: '2026-10-03T10:00:00Z', content: { text: 'testing the gap' } },
  { seq: 2, kind: 'tool_call', created_at: '2026-10-03T10:00:01Z', content: { name: 'run_backtest' } },
  result(3, {
    summary: 'Sharpe 0.41 over 812 trades, after costs',
    ids: { backtest_id: 'bt_7f0c2a', reservation_id: 'tkt_99a1b' },
    n: 812,
    se: 0.12,
    detail: { sharpe: 0.41 },
  }),
  // The cloud driver's spelling: the ToolResult as a JSON string.
  result(4, JSON.stringify({ summary: 'study done', ids: { study_id: 'study_31c' }, handle: 'h_0042x' }), 'run_study'),
  // A free-text reply is not a ToolResult and yields no ids.
  result(5, 'plain words with exp_fake123 in them' as unknown, 'search_tools'),
]

describe('the ids a tool returned are read where the drivers write them', () => {
  const cards = toCards(stream)

  it('reads content.ids one level down, object or string', () => {
    const ev = streamEvidence(cards)
    expect(ev.map((e) => e.id).sort()).toEqual(['bt_7f0c2a', 'h_0042x', 'study_31c', 'tkt_99a1b'])
    const bt = ev.find((e) => e.id === 'bt_7f0c2a')!
    expect(bt).toMatchObject({ key: 'backtest_id', type: 'backtest', tool: 'run_backtest', n: 812, se: 0.12 })
    expect(bt.summary).toMatch(/Sharpe 0.41/)
  })

  it('does not invent ids from a free-text reply', () => {
    expect(storedResult({ content: 'plain words with exp_fake123' })).toBeNull()
    expect(streamEvidence(cards).some((e) => e.id === 'exp_fake123')).toBe(false)
  })

  it('the resolver now resolves what the tool returned, and still not what it did not', () => {
    const resolve = makeResolver(null, cards)
    expect(resolve('bt_7f0c2a')).toMatchObject({ kind: 'tool result' })
    expect(resolve('h_0042x')).toMatchObject({ kind: 'handle' })
    expect(resolve('exp_fake123')).toBeNull()
  })
})

describe('the Evidence tab opens the stored result in place', () => {
  const cards = toCards(stream)
  const resolve = makeResolver(null, cards)
  const entries = evidenceEntries(null, streamEvidence(cards))

  it('lists the stream ids even though the working state route answers nothing', () => {
    render(<EvidenceTab entries={entries} resolve={resolve} absence="not_served" />)
    expect(screen.queryByText(/Nothing reports what this run produced/)).toBeNull()
    for (const id of ['bt_7f0c2a', 'tkt_99a1b', 'study_31c', 'h_0042x']) {
      expect(screen.getByTestId(`ev-${id}`)).toHaveAttribute('data-resolved', 'true')
      expect(screen.getByTestId(`ev-${id}`)).toHaveAttribute('data-openable', 'true')
    }
  })

  it('every listed id opens its result on click', () => {
    render(<EvidenceTab entries={entries} resolve={resolve} />)
    for (const e of entries) {
      // The first chip with this id is the list's own; a sibling chip inside an
      // open viewer carries the same test id.
      fireEvent.click(screen.getAllByTestId(`ev-${e.id}`)[0])
      expect(screen.getByTestId(`ev-view-${e.id}`)).toBeInTheDocument()
      fireEvent.click(screen.getAllByTestId(`ev-${e.id}`)[0])
      expect(screen.queryByTestId(`ev-view-${e.id}`)).toBeNull()
    }
    // The viewer shows what was stored, n included.
    fireEvent.click(screen.getByTestId('ev-bt_7f0c2a'))
    const view = screen.getByTestId('ev-view-bt_7f0c2a')
    expect(view.textContent).toMatch(/Sharpe 0.41 over 812 trades/)
    expect(view.textContent).toMatch(/n 812/)
  })

  it('an injected unknown id is struck through with "not found"', () => {
    render(
      <EvidenceTab
        entries={[...entries, { id: 'exp_ghost', type: 'exp', summary: 'claimed by the model' }]}
        resolve={resolve}
      />,
    )
    const chip = screen.getByTestId('ev-exp_ghost')
    expect(chip).toHaveAttribute('data-resolved', 'false')
    expect(screen.getByText('exp_ghost').tagName.toLowerCase()).toBe('s')
  })

  it('an id another surface asked for that no result returned says so', () => {
    render(<EvidenceTab entries={entries} resolve={resolve} focusedId="exp_ghost" />)
    expect(screen.getByTestId('ev-not-listed').textContent).toMatch(/not found/)
  })

  it('an id another surface asked for that is listed opens directly', () => {
    render(<EvidenceTab entries={entries} resolve={resolve} focusedId="study_31c" />)
    expect(screen.getByTestId('ev-view-study_31c')).toBeInTheDocument()
  })
})
