/**
 * Reordering the agenda board (AGENT-014 §9; Set S task S-11.13).
 *
 * §9 says "drag = `user` priority event", and `POST …/{id}/agenda/order` takes
 * that literally: it puts the `user` source on **every id it is sent**, clears
 * that item's score and promotes `proposed` to `queued`. There is no rank
 * column anywhere.
 *
 * Two defects follow from reading that route carelessly, and both are here:
 *
 * 1. **Posting the whole board.** The obvious client sends the arrangement it
 *    is showing. That would mark every question on the agenda a `user` item,
 *    and a `user` item bypasses scoring entirely (§4.2) — so telling the
 *    session that all twelve questions are the priority tells it exactly as
 *    much as telling it none of them is. The scorer would be switched off by a
 *    drag.
 * 2. **A board only a mouse can reorder.** Telling the session what matters is
 *    not an optional part of this panel, and a drag handler alone puts it out
 *    of reach of anybody using a keyboard.
 *
 * The board was also reorderable by nobody at all before this: `AgendaTab` had
 * no order state and `Conversation` passed no handler, so `deskApi.reorderAgenda`
 * — written weeks ago against a served route — had no caller.
 */
import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import type { AgendaItem } from '@/api/desk'
import { applyOrder, dropOnto, moveInColumn, prioritisedBy, reconcileOrder } from '@/lib/desk/agenda'
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

describe('only the cards that moved up are posted', () => {
  it('names the ones that end up earlier than they started, in their new order', () => {
    expect(prioritisedBy(['a', 'b', 'c'], ['c', 'a', 'b'])).toEqual(['c'])
    expect(prioritisedBy(['a', 'b', 'c', 'd'], ['c', 'd', 'a', 'b'])).toEqual(['c', 'd'])
  })

  it('posts nothing for an arrangement that did not change', () => {
    expect(prioritisedBy(['a', 'b', 'c'], ['a', 'b', 'c'])).toEqual([])
  })

  it('never posts the whole board, which would make every item a user item', () => {
    const before = ['a', 'b', 'c', 'd', 'e']
    const after = moveInColumn(before, before, 'e', -1)
    expect(prioritisedBy(before, after)).toEqual(['e'])
    expect(prioritisedBy(before, after).length).toBeLessThan(before.length)
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
  function board(onPrioritise = vi.fn()) {
    render(<AgendaTab items={[item('a'), item('b'), item('c')]} onPrioritise={onPrioritise} />)
    return onPrioritise
  }

  it('offers a move control on every card of a claimable column', () => {
    board()
    expect(screen.getByLabelText('Move up: question b')).toBeInTheDocument()
    expect(screen.getByLabelText('Move down: question b')).toBeInTheDocument()
  })

  it('posts the moved card and nothing else', () => {
    const onPrioritise = board()
    fireEvent.click(screen.getByLabelText('Move up: question c'))
    expect(onPrioritise).toHaveBeenCalledTimes(1)
    expect(onPrioritise).toHaveBeenCalledWith(['c'])
  })

  it('shows the new arrangement, because nothing on the server remembers it', () => {
    board()
    fireEvent.click(screen.getByLabelText('Move up: question c'))
    const questions = screen.getAllByText(/^question /).map((n) => n.textContent)
    expect(questions).toEqual(['question a', 'question c', 'question b'])
  })

  it('disables the control that would push a card off the end, and posts nothing', () => {
    const onPrioritise = board()
    const top = screen.getByLabelText('Move up: question a') as HTMLButtonElement
    expect(top.disabled).toBe(true)
    fireEvent.click(top)
    expect(onPrioritise).not.toHaveBeenCalled()
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

describe('the pointer path posts the same short list as the keyboard one', () => {
  it('drops one card onto another and posts only what moved up', () => {
    const onPrioritise = vi.fn()
    render(<AgendaTab items={[item('a'), item('b'), item('c')]} onPrioritise={onPrioritise} />)
    const cards = screen.getAllByText(/^question /).map((n) => n.closest('.desk-card') as HTMLElement)
    expect(cards.length).toBe(3)
    fireEvent.dragStart(cards[2])
    fireEvent.drop(cards[0])
    expect(onPrioritise).toHaveBeenCalledWith(['c'])
  })
})
