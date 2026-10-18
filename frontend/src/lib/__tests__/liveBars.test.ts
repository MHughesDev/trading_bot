// FB-RX-002: a scripted message sequence merged onto REST history.
import { describe, expect, it } from 'vitest'
import { foldTrade, mergeLiveBars, upsertLive } from '@/lib/liveBars'
import type { Bar } from '@/components/charts/MultiPaneChart'

const bar = (ts: number, o: number, h: number, l: number, c: number, volume = 1): Bar => ({
  ts,
  open: o,
  high: h,
  low: l,
  close: c,
  volume,
})

const history: Bar[] = [bar(60, 1, 2, 1, 2), bar(120, 2, 3, 2, 3), bar(180, 3, 4, 3, 4)]

describe('the live tail merged onto history', () => {
  it('appends a bar that closes after the last history bar', () => {
    const live = upsertLive([], bar(240, 4, 5, 4, 5))
    expect(mergeLiveBars(history, live).map((b) => b.ts)).toEqual([60, 120, 180, 240])
  })

  it('replaces the last history bar when it was still forming', () => {
    const live = upsertLive([], bar(180, 3, 6, 3, 6, 9))
    const out = mergeLiveBars(history, live)
    expect(out.map((b) => b.ts)).toEqual([60, 120, 180])
    expect(out[2].high).toBe(6)
    expect(out[2].volume).toBe(9)
  })

  it('never rewrites a closed history bar from a late frame', () => {
    const live = upsertLive([], bar(120, 9, 9, 9, 9))
    expect(mergeLiveBars(history, live)).toEqual(history)
  })

  it('treats two frames for one minute as one bar, the later winning', () => {
    let live = upsertLive([], bar(240, 4, 5, 4, 5))
    live = upsertLive(live, bar(240, 4, 7, 4, 7))
    const out = mergeLiveBars(history, live)
    expect(out.filter((b) => b.ts === 240)).toHaveLength(1)
    expect(out[out.length - 1].high).toBe(7)
  })

  it('with no history, shows the live bars alone in order', () => {
    let live = upsertLive([], bar(240, 1, 1, 1, 1))
    live = upsertLive(live, bar(180, 1, 1, 1, 1))
    expect(mergeLiveBars([], live).map((b) => b.ts)).toEqual([180, 240])
  })

  it('is the history itself when nothing live has arrived', () => {
    expect(mergeLiveBars(history, [])).toBe(history)
  })
})

describe('a scripted trades-then-close sequence', () => {
  it('extends the forming candle, then lets the closed bar replace it', () => {
    let live: Bar[] = []
    // Three prints inside minute 240, then the aggregator's closed bar.
    live = foldTrade(live, history, 10, 240)
    live = foldTrade(live, history, 12, 240)
    live = foldTrade(live, history, 9, 240)
    let out = mergeLiveBars(history, live)
    expect(out[out.length - 1]).toMatchObject({ ts: 240, open: 10, high: 12, low: 9, close: 9 })

    live = upsertLive(live, bar(240, 10, 12, 9, 11, 42))
    out = mergeLiveBars(history, live)
    expect(out[out.length - 1]).toMatchObject({ ts: 240, close: 11, volume: 42 })
    // The next minute's first print starts a new candle after it.
    live = foldTrade(live, history, 11.5, 300)
    out = mergeLiveBars(history, live)
    expect(out.map((b) => b.ts)).toEqual([60, 120, 180, 240, 300])
  })

  it('seeds the first print from the forming bar REST already returned', () => {
    // History's last bar (180) is the minute still open: the print extends it
    // and keeps its open and volume rather than starting over at the print.
    const live = foldTrade([], history, 5, 180)
    const out = mergeLiveBars(history, live)
    expect(out).toHaveLength(3)
    expect(out[2]).toMatchObject({ ts: 180, open: 3, high: 5, low: 3, close: 5, volume: 1 })
  })

  it('ignores a print that is not a number', () => {
    const live = foldTrade([], history, Number.NaN, 240)
    expect(live).toEqual([])
  })
})
