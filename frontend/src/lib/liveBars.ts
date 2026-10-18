// Merging the live tail onto the REST history of a chart (FB-RX-002).
//
// REST gives the closed bars; the gateway's `market.bars.1m` lane delivers a
// bar when its minute closes and `market.trades` delivers every print in
// between. The chart has to show one series built from the two, and the rules
// for that live here as pure functions so a scripted message sequence can be
// asserted against them (the vitest the task asks for):
//
//   * a live bar **newer** than the last history bar is appended;
//   * a live bar with the **same** timestamp as the last history bar replaces
//     it — that bar was still forming when REST answered, and history is stale
//     for it;
//   * a live bar **older** than the last history bar is dropped: closed history
//     is authoritative, and a late frame must not rewrite it;
//   * two live bars with one timestamp are one bar, the later wins;
//   * a trade print folds into the forming candle (high/low/close), seeded from
//     whichever side already holds that minute so the open and volume are not
//     replaced by the print's own price.
import type { Bar } from '@/components/charts/MultiPaneChart'

const t = (b: Bar): number => Number(b.ts)

/** One bar per timestamp, oldest first, the last write winning. */
function dedupe(bars: Bar[]): Bar[] {
  const byTs = new Map<number, Bar>()
  for (const b of bars) byTs.set(t(b), b)
  return [...byTs.values()].sort((a, b) => t(a) - t(b))
}

export function mergeLiveBars(history: Bar[], live: Bar[]): Bar[] {
  if (live.length === 0) return history
  const lastTs = history.length > 0 ? t(history[history.length - 1]) : -Infinity
  const tail = dedupe(live).filter((b) => t(b) >= lastTs)
  if (tail.length === 0) return history
  // The first tail bar may be the forming bar history already holds.
  const keep = tail[0] && t(tail[0]) === lastTs ? history.slice(0, -1) : history
  return [...keep, ...tail]
}

/** Closed-bar frame: replace the bar with that timestamp, or add it. Capped. */
export function upsertLive(prev: Bar[], bar: Bar, cap = 600): Bar[] {
  return dedupe([...prev, bar]).slice(-cap)
}

/**
 * Fold one trade print into the candle for `minuteTs`.
 *
 * `history` is consulted only to seed a candle the live list does not yet hold:
 * when REST already returned the forming bar, the first print must extend *that*
 * bar, not start a new one at the print's price.
 */
export function foldTrade(prev: Bar[], history: Bar[], price: number, minuteTs: number, cap = 600): Bar[] {
  if (!Number.isFinite(price)) return prev
  const own = prev.find((b) => t(b) === minuteTs)
  const seed = own ?? history.find((b) => t(b) === minuteTs)
  const next: Bar = seed
    ? { ...seed, high: Math.max(seed.high, price), low: Math.min(seed.low, price), close: price }
    : { ts: minuteTs, open: price, high: price, low: price, close: price, volume: 0 }
  return upsertLive(prev, next, cap)
}
