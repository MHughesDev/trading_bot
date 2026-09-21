// Reordering the agenda board (AGENT-014 §9; AGENT-012 §2b; Set S task S-11.13).
//
// # A drag is a priority event, not a rank
//
// §9 says "drag = `user` priority event", and `POST …/{id}/agenda/order` is
// where that is decided. The route is now given **the board's whole order** and
// works out for itself which cards ended up earlier than they were: it
// subtracts the longest run of ids already in board order and marks only what
// is left, so a card dragged *down* prioritises nothing. What it marks, it marks
// with the `user` source, which clears the item's score and promotes a
// `proposed` item to `queued`. There is **no rank column anywhere** — the
// session claims `user` items next (§4.2's bypass) and scores the rest.
//
// Two consequences, and both of them are the reason this file is pure functions
// with tests rather than a `sort` inside a component:
//
// 1. **The client posts the arrangement, and the route says what it meant.**
//    The client used to filter the body down to the cards that moved up. That
//    is now both redundant and refused: the route subtracts the unmoved cards
//    itself, rejects a body of one id (an order of one says nothing about what
//    comes before what, and `POST …/{item}/prioritise` is the route for that
//    intent) and rejects duplicates. One card dragged to the top of a board
//    *is* a one-id filtered body, so the old client posted exactly the shape
//    that draws a 400. The rule lives in the route because a client that
//    filters is a discipline, and the next caller — a script, a replay, a
//    second client — does not inherit it.
// 2. **The order the human dragged into is the client's, not the server's.**
//    Nothing persists it, so the board holds it for as long as the tab is open
//    and says so on the card. A UI that silently reverted on the next poll
//    would look broken; one that claims the order was saved would be lying.
//
// # The keyboard path is not an extra
//
// A board that can only be reordered by dragging cannot be reordered by
// somebody using a keyboard at all, and "drag to prioritise" is the only way to
// tell this session what matters. `moveInColumn` is what both paths call, which
// is what keeps them from drifting apart.

/**
 * The board's working order: what was there before, minus what has gone, plus
 * what is new.
 *
 * New ids go **last** rather than being spliced into the server's position: an
 * item that appeared while the human was arranging the board has not been
 * arranged, and dropping it into the middle would silently move the cards
 * around it.
 */
export function reconcileOrder(previous: readonly string[] | null, ids: readonly string[]): string[] {
  const present = new Set(ids)
  const kept = (previous ?? []).filter((id) => present.has(id))
  const seen = new Set(kept)
  return [...kept, ...ids.filter((id) => !seen.has(id))]
}

/** `ids` sorted into `order`; anything `order` does not name keeps its place at the end. */
export function applyOrder<T>(rows: readonly T[], key: (row: T) => string, order: readonly string[]): T[] {
  const at = new Map(order.map((id, i) => [id, i]))
  return [...rows].sort((a, b) => (at.get(key(a)) ?? Number.MAX_SAFE_INTEGER) - (at.get(key(b)) ?? Number.MAX_SAFE_INTEGER))
}

/**
 * Move one card up or down **within its own column**.
 *
 * The two ids swap their positions in the whole-board order, so no card outside
 * the column moves. Returns the order unchanged when the card is already at the
 * end it is being pushed towards — the control for that direction is disabled,
 * and an order that is the board's own is one the route answers by marking
 * nothing.
 */
export function moveInColumn(
  order: readonly string[],
  columnIds: readonly string[],
  id: string,
  delta: number,
): string[] {
  const column = order.filter((x) => columnIds.includes(x))
  const from = column.indexOf(id)
  if (from < 0) return [...order]
  const to = from + delta
  if (to < 0 || to >= column.length) return [...order]
  const a = order.indexOf(column[from])
  const b = order.indexOf(column[to])
  const next = [...order]
  next[a] = column[to]
  next[b] = column[from]
  return next
}

/**
 * Drop `dragged` onto `target`'s place within the column (the pointer path).
 *
 * The dragged card takes the target's position in the whole-board order and
 * everything between them shifts by one, which is what a drag looks like.
 */
export function dropOnto(
  order: readonly string[],
  columnIds: readonly string[],
  dragged: string,
  target: string,
): string[] {
  if (dragged === target) return [...order]
  if (!columnIds.includes(dragged) || !columnIds.includes(target)) return [...order]
  const without = order.filter((x) => x !== dragged)
  const at = without.indexOf(target)
  if (at < 0) return [...order]
  const before = order.indexOf(dragged) < order.indexOf(target)
  const next = [...without]
  next.splice(before ? at + 1 : at, 0, dragged)
  return next
}
