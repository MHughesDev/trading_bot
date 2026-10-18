/**
 * FB-RX-002: the terminal chart's hook merges a scripted live message sequence
 * onto REST history, through the real `wsBus` and the real merge.
 *
 * The REST history ends on a minute that was still forming when it was read.
 * The sequence is the one production sends: prints on `market.trades`, then the
 * aggregator's closed bar on `market.bars.1m`, then the next minute's first
 * print. What the chart receives must replace the forming bar, never duplicate
 * it, and append the next one.
 */
import { act, renderHook, waitFor } from '@testing-library/react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'

import { wsBus } from '@/api/ws'
import { useBars } from '@/hooks/useBars'

vi.mock('@/lib/api', async (orig) => {
  const real = await orig<typeof import('@/lib/api')>()
  return {
    ...real,
    assetApi: {
      ...real.assetApi,
      chartBars: async () => ({
        data: {
          bars: [
            { t: 60, o: '1', h: '2', l: '1', c: '2', v: '5' },
            { t: 120, o: '2', h: '3', l: '2', c: '3', v: '5' },
          ],
        },
      }),
    },
  }
})

function wrapper({ children }: { children: ReactNode }) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>
}

const frame = (lane: string, payload: Record<string, unknown>, instrument = 'BTC-USD') =>
  ({ type: 'frame', sub_id: 's', lane, instrument, payload }) as never

describe('the live tail on the terminal chart', () => {
  it('replaces the forming bar, then appends the next one', async () => {
    const { result } = renderHook(() => useBars('BTC-USD', 60, 1), { wrapper })
    await waitFor(() => expect(result.current.bars).toHaveLength(2))

    // The closed bar for the minute REST still held as forming.
    act(() => wsBus.emit(frame('market.bars.1m', { ts: 120, open: 2, high: 9, low: 2, close: 8, volume: 77 })))
    await waitFor(() => expect(result.current.bars[1].high).toBe(9))
    expect(result.current.bars.map((b) => Number(b.ts))).toEqual([60, 120])
    expect(result.current.bars[1].volume).toBe(77)

    // The next minute closes: appended, not merged into anything.
    act(() => wsBus.emit(frame('market.bars.1m', { ts: 180, open: 8, high: 8, low: 7, close: 7, volume: 3 })))
    await waitFor(() => expect(result.current.bars).toHaveLength(3))
    expect(Number(result.current.bars[2].ts)).toBe(180)

    // A frame for another instrument is not this chart's.
    act(() => wsBus.emit(frame('market.bars.1m', { ts: 240, open: 1, high: 1, low: 1, close: 1 }, 'ETH-USD')))
    expect(result.current.bars).toHaveLength(3)
  })
})
