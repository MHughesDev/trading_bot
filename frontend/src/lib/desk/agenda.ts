// Reordering the agenda board (AGENT-014 §9; AGENT-012 §2b; Set S task S-11.13).
//
// # A drag is a priority event, not a rank
//
// §9 says "drag = `user` priority event". `POST …/{id}/agenda/order` honours
// that literally: it puts the `user` source on every item id it is sent, clears
// that item's score and promotes a `proposed` item to `queued`. There is **no
// rank column anywhere** — the session claims `user` items next (§4.2's bypass)
// and scores the rest.
//
// Two consequences, and both of them are the reason this file is pure functions
// with tests rather than a `sort` inside a component:
//
// 1. **The client must post the items the human moved up, never the whole
//    board.** The route marks everything it is sent, so posting the board would
//    mark every question on it a priority — which tells the session exactly as
//    much as marking none of them. `prioritisedBy` is that short list.
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
 * end it is being pushed towards — a no-op reorder must not post anything,
 * because posting it would mark the item `user` for a move that did not happen.
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

/**
 * The ids to post: everything that ends up **earlier** than it started, in its
 * new order.
 *
 * Only the ones that moved up, because the route reads the list as "these
 * matter". A card that moved down did not become a priority, and the card that
 * displaced it did.
 */
export function prioritisedBy(before: readonly string[], after: readonly string[]): string[] {
  const was = new Map(before.map((id, i) => [id, i]))
  return after.filter((id, i) => {
    const old = was.get(id)
    return old !== undefined && i < old
  })
}
