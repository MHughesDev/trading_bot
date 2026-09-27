import { useEffect, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { assetApi } from '@/lib/api'
import { getWsClient, wsBus } from '@/api/ws'
import type { Quote } from '@/components/trading/Watchlist'

/** Live last-price + session change for a set of symbols, off the 1m bar lane. */
export function useLiveQuotes(symbols: string[]): Record<string, Quote> {
  const [quotes, setQuotes] = useState<Record<string, Quote>>({})
  const panelId = useRef(`quotes-${Math.random().toString(36).slice(2, 8)}`)
  const key = symbols.join(',')

  useEffect(() => {
    if (!symbols.length) return
    const client = getWsClient()
    const id = panelId.current

    // Two lanes, on purpose. Bars give the session anchor a change % is measured
    // against; trades give the last price between minute boundaries. Subscribing
    // to bars alone means a price that only moves once a minute, which on a
    // trading screen reads as a frozen feed.
    client?.subscribe(id, [
      ...symbols.map((instrument) => ({ lane: 'market.bars.1m', instrument, max_fps: 2 })),
      ...symbols.map((instrument) => ({ lane: 'market.trades', instrument, max_fps: 4 })),
    ])

    const off = wsBus.on((msg) => {
      if (msg.type !== 'frame') return
      const f = msg as unknown as { lane: string; instrument: string; payload: Record<string, unknown> }

      if (f.lane === 'market.bars.1m') {
        const close = Number(f.payload?.close)
        const open = Number(f.payload?.open)
        if (!Number.isFinite(close)) return
        setQuotes((q) => {
          const prev = q[f.instrument]
          const anchor = prev?.prevClose ?? (Number.isFinite(open) ? open : close)
          return {
            ...q,
            [f.instrument]: {
              last: close,
              prevClose: anchor,
              changePct: anchor ? ((close - anchor) / anchor) * 100 : null,
              updatedAt: Date.now(),
            },
          }
        })
        return
      }

      if (f.lane === 'market.trades') {
        const price = Number(f.payload?.price)
        if (!Number.isFinite(price)) return
        setQuotes((q) => {
          const prev = q[f.instrument]
          const anchor = prev?.prevClose ?? price
          return {
            ...q,
            [f.instrument]: {
              last: price,
              prevClose: anchor,
              changePct: anchor ? ((price - anchor) / anchor) * 100 : null,
              updatedAt: Date.now(),
            },
          }
        })
      }
    })

    return () => {
      off()
      client?.unsubscribe(id)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key])

  return quotes
}

export function useInitializedAssets() {
  return useQuery({
    queryKey: ['initialized-assets'],
    queryFn: async () => {
      const { data } = await assetApi.initialized()
      return ((data as { assets?: InitializedAsset[] }).assets ?? []) as InitializedAsset[]
    },
    staleTime: 30_000,
  })
}

interface InitializedAsset {
  symbol: string
  asset_class: string
}
