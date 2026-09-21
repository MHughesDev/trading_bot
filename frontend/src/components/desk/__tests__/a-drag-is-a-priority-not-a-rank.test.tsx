/**
 * Reordering the agenda board (AGENT-014 §9; Set S task S-11.13).
 *
 * §9 says "drag = `user` priority event". A `user` source is not a nudge, it is
 * a **bypass**: such an item is claimed next with propensity 1.0 and never
 * scored (§4.2), so one drag that marked every card would switch the selection
 * policy off for the whole session with nothing on screen saying so.
 *
 * Which cards earn that source is therefore decided in `POST
 * …/{id}/agenda/order`, and this file pins the client to the half of the
 * contract that is left to it:
 *
 * 1. **The body is the board's whole order, every id once.** The route compares
 *    it against the board's own order, subtracts the longest run that is already
 *    in it, and marks only the remainder — so a card dragged *down* prioritises
 *    nothing, without the client working that out.
 * 2. **Never a body of one id.** The route refuses one, naming the item's own
 *    `prioritise` route as the way to say "this one matters". The client used to
 *    filter the body down to the cards that ended up earlier than they started,
 *    and one card dragged to the top of a board filters down to exactly one id —
 *    so the old client's commonest request is the one shape the route rejects.
 *    The filtering was redundant on every other path and refused on that one.
 * 3. **A board only a mouse can reorder is a board a keyboard cannot.** Telling
 *    the session what matters is not an optional part of this panel, so the move
 *    controls and the drag handlers go through the same function.
 *
 * The board was reorderable by nobody at all before any of this: `AgendaTab` had
 * no order state and `Conversation` passed no handler, so `deskApi.reorderAgenda`
 * — written weeks ago against a served route — had no caller.
 */
import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import type { AgendaItem } from '@/api/desk'
import * as agendaLib from '@/lib/desk/agenda'
import { applyOrder, dropOnto, moveInColumn, reconcileOrder } from '@/lib/desk/agenda'
import { AgendaTab } from '../RightPanel'

function item(id: string, over: Partial<AgendaItem> = {}): AgendaItem {
  return {
    item_id: id,
    question: `question ${id}`,
    workflow: 'investigate',
    sources: ['charter_seed'],
    score: null,
    state: 'queued',
    ...over,
  }
}

/** The board, rendered; returns the spy the whole-board order is posted to. */
function board(ids: string[] = ['a', 'b', 'c'], onReorder = vi.fn()) {
  render(<AgendaTab items={ids.map((id) => item(id))} onReorder={onReorder} />)
  return onReorder
}

/** The ids of the one body that was posted. */
function posted(spy: ReturnType<typeof vi.fn>): string[] {
  expect(spy).toHaveBeenCalledTimes(1)
  return spy.mock.calls[0][0] as string[]
}

describe('the whole board order is posted, and the route says what it meant', () => {
  it('sends every card, not the ones that ended up earlier', () => {
    const onReorder = board()
    fireEvent.click(screen.getByLabelText('Move up: question c'))
    // Not `['c']`. The route subtracts the run that stayed in board order, so
    // `a` and `b` in the body are how it knows `c` passed them.
    expect(posted(onReorder)).toEqual(['a', 'c', 'b'])
  })

  it('sends each id exactly once, because the route refuses a duplicate', () => {
    const onReorder = board(['a', 'b', 'c', 'd'])
    fireEvent.click(screen.getByLabelText('Move up: question d'))
    const body = posted(onReorder)
    expect(new Set(body).size).toBe(body.length)
    expect([...body].sort()).toEqual(['a', 'b', 'c', 'd'])
  })

  it('never sends a body of one id, which is the request the route rejects', () => {
    // The old client's commonest post: one card dragged to the top filtered down
    // to a single id, and `movable.len() < 2` answers that with a 400 naming
    // `POST …/{item}/prioritise`. Four ids go up instead.
    const onReorder = vi.fn()
    render(<AgendaTab items={['a', 'b', 'c', 'd'].map((id) => item(id))} onReorder={onReorder} />)
    const cards = screen.getAllByText(/^question /).map((n) => n.closest('.desk-card') as HTMLElement)
    fireEvent.dragStart(cards[3])
    fireEvent.drop(cards[0])
    expect(posted(onReorder)).toEqual(['d', 'a', 'b', 'c'])
    expect(posted(onReorder).length).toBeGreaterThan(1)
  })

  it('posts a card dragged down too, and lets the route find nothing promoted', () => {
    // A drag downwards promotes nothing — but that is the route's finding, made
    // from the board it was sent. The client filtering first turned this into
    // either a silent no-post or a body the route reads differently from the
    // board it describes.
    const onReorder = vi.fn()
    render(<AgendaTab items={['a', 'b', 'c', 'd'].map((id) => item(id))} onReorder={onReorder} />)
    const cards = screen.getAllByText(/^question /).map((n) => n.closest('.desk-card') as HTMLElement)
    fireEvent.dragStart(cards[0])
    fireEvent.drop(cards[2])
    expect(posted(onReorder)).toEqual(['b', 'c', 'a', 'd'])
  })

  it('still sends the whole order when the board comes back unchanged', () => {
    // Dropped onto itself: the arrangement did not move. The body is still the
    // board, because "did anything move?" is the same question as "what moved",
    // and the route is the one that answers it — with `prioritised: 0`.
    const onReorder = board()
    const cards = screen.getAllByText(/^question /).map((n) => n.closest('.desk-card') as HTMLElement)
    fireEvent.dragStart(cards[1])
    fireEvent.drop(cards[1])
    expect(posted(onReorder)).toEqual(['a', 'b', 'c'])
  })

  it('keeps no filter of its own for the body', () => {
    // The helper that computed "the ones that moved up" is gone rather than
    // unused: an exported filter is one a future caller reaches for, and the
    // route has refused that shape since it started subtracting the run itself.
    expect(Object.keys(agendaLib)).not.toContain('prioritisedBy')
  })
})

describe('the order the board holds', () => {
  it('keeps the arrangement, drops what has gone and puts new items last', () => {
    expect(reconcileOrder(['c', 'a', 'b'], ['a', 'b', 'c', 'd'])).toEqual(['c', 'a', 'b', 'd'])
    expect(reconcileOrder(['c', 'a', 'b'], ['a', 'b'])).toEqual(['a', 'b'])
    expect(reconcileOrder(null, ['a', 'b'])).toEqual(['a', 'b'])
  })

  it('moves a card within its own column and leaves every other column alone', () => {
    const order = ['a', 'x', 'b', 'c']
    expect(moveInColumn(order, ['a', 'b', 'c'], 'c', -1)).toEqual(['a', 'x', 'c', 'b'])
    // `x` is in another column and does not move.
    expect(moveInColumn(order, ['a', 'b', 'c'], 'c', -1)[1]).toBe('x')
  })

  it('refuses to move a card off either end', () => {
    expect(moveInColumn(['a', 'b'], ['a', 'b'], 'a', -1)).toEqual(['a', 'b'])
    expect(moveInColumn(['a', 'b'], ['a', 'b'], 'b', 1)).toEqual(['a', 'b'])
  })

  it('drops a card onto another one the way a drag looks', () => {
    expect(dropOnto(['a', 'b', 'c'], ['a', 'b', 'c'], 'c', 'a')).toEqual(['c', 'a', 'b'])
    expect(dropOnto(['a', 'b', 'c'], ['a', 'b', 'c'], 'a', 'c')).toEqual(['b', 'c', 'a'])
    expect(dropOnto(['a', 'b', 'c'], ['a', 'b', 'c'], 'a', 'a')).toEqual(['a', 'b', 'c'])
  })

  it('sorts the cards by that order and keeps anything it does not name', () => {
    const rows = [item('a'), item('b'), item('c')]
    expect(applyOrder(rows, (r) => r.item_id, ['c', 'a']).map((r) => r.item_id)).toEqual(['c', 'a', 'b'])
  })
})

describe('the board can be reordered from a keyboard, not only by dragging', () => {
  it('offers a move control on every card of a claimable column', () => {
    board()
    expect(screen.getByLabelText('Move up: question b')).toBeInTheDocument()
    expect(screen.getByLabelText('Move down: question b')).toBeInTheDocument()
  })

  it('shows the new arrangement, because nothing on the server remembers it', () => {
    board()
    fireEvent.click(screen.getByLabelText('Move up: question c'))
    const questions = screen.getAllByText(/^question /).map((n) => n.textContent)
    expect(questions).toEqual(['question a', 'question c', 'question b'])
  })

  it('disables the control that would push a card off the end, and posts nothing', () => {
    const onReorder = board()
    const top = screen.getByLabelText('Move up: question a') as HTMLButtonElement
    expect(top.disabled).toBe(true)
    fireEvent.click(top)
    expect(onReorder).not.toHaveBeenCalled()
  })

  it('does not offer to reorder a column whose items are already claimed or finished', () => {
    render(<AgendaTab items={[item('d', { state: 'done' }), item('e', { state: 'done' })]} />)
    expect(screen.queryByLabelText('Move up: question e')).toBeNull()
  })

  it('says what a move does, because the board keeps no ranking of its own', () => {
    board()
    const note = screen.getByTestId('reorder-note').textContent ?? ''
    expect(note).toContain('marks it yours')
    expect(note).toContain('keeps no ranking')
  })
})
