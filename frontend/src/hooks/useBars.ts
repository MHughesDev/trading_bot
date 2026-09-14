import { useEffect, useMemo, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { assetApi } from '@/lib/api'
import { usePrefs } from '@/store/prefs'
import { getWsClient, wsBus } from '@/api/ws'
import type { Bar } from '@/components/charts/MultiPaneChart'

/* =============================================================================
   Bars for one instrument at one timeframe.

   REST gives the history; the 1m lane gives the live tail. Coarser timeframes
   rely on the server aggregation, so the live splice only applies at 1m.
   Spec M1/M3: nothing animates on append.
   ============================================================================= */

export const TIMEFRAMES = [
  { label: '1m', secs: 60 },
  { label: '5m', secs: 300 },
  { label: '15m', secs: 900 },
  { label: '1h', secs: 3600 },
  { label: '4h', secs: 14400 },
  { label: '1D', secs: 86400 },
] as const

export type TimeframeSecs = (typeof TIMEFRAMES)[number]['secs']

export function timeframeLabel(secs: number): string {
  return TIMEFRAMES.find((t) => t.secs === secs)?.label ?? `${secs}s`
}

/**
 * The REST bars endpoint emits compact keys (`t/o/h/l/c/v`) to keep a
 * multi-thousand-bar response small; the live bus emits the long ones. Both
 * shapes are accepted here so neither producer has to change for the other.
 */
interface RawBar {
  ts?: string | number
  time?: string | number
  t?: string | number
  open?: string | number
  o?: string | number
  high?: string | number
  h?: string | number
  low?: string | number
  l?: string | number
  close?: string | number
  c?: string | number
  volume?: string | number
  v?: string | number
}

function normalise(rows: RawBar[]): Bar[] {
  return rows
    .map((b) => ({
      ts: (b.ts ?? b.time ?? b.t) as string | number,
      open: Number(b.open ?? b.o),
      high: Number(b.high ?? b.h),
      low: Number(b.low ?? b.l),
      close: Number(b.close ?? b.c),
      volume: (b.volume ?? b.v) === undefined ? undefined : Number(b.volume ?? b.v),
    }))
    .filter((b) => Number.isFinite(b.close) && b.ts !== undefined)
}

export interface BarsState {
  bars: Bar[]
  loading: boolean
  error: boolean
  refetch: () => void
  /** Epoch ms of the most recent update, for the staleness badge. */
  lastUpdate: number | null
}

export function useBars(instrument: string | undefined, tfSecs: number, lookbackDaysOverride?: number): BarsState {
  const prefLookback = usePrefs((s) => s.chartLookbackDays)
  const lookbackDays = lookbackDaysOverride ?? prefLookback
  const [liveBars, setLiveBars] = useState<Bar[]>([])
  const [lastUpdate, setLastUpdate] = useState<number | null>(null)
  const panelId = useRef(`bars-${Math.random().toString(36).slice(2, 8)}`)

  const rest = useQuery({
    queryKey: ['bars', instrument, tfSecs, lookbackDays],
    enabled: !!instrument,
    queryFn: async () => {
      const end = new Date().toISOString()
      const start = new Date(Date.now() - lookbackDays * 86400_000).toISOString()
      const { data } = await assetApi.chartBars(instrument!, start, end, tfSecs)
      const rows = (data as { bars?: RawBar[] })?.bars ?? (data as unknown as RawBar[])
      return normalise(Array.isArray(rows) ? rows : [])
    },
    refetchInterval: 15_000,
    staleTime: 5_000,
  })

  useEffect(() => {
    setLiveBars([])
  }, [instrument, tfSecs])

  useEffect(() => {
    if (!instrument) return
    // Only the 1m view splices a live tail; coarser frames come from the
    // server-side aggregation, which is authoritative for their boundaries.
    if (tfSecs !== 60) return
    const client = getWsClient()
    const id = panelId.current
    client?.subscribe(id, [
      { lane: 'market.bars.1m', instrument, max_fps: 4 },
      { lane: 'market.trades', instrument, max_fps: 4 },
    ])
    const off = wsBus.on((msg) => {
      if (msg.type !== 'frame') return
      const f = msg as unknown as { lane: string; instrument: string; payload: RawBar }
      if (f.instrument !== instrument) return

      if (f.lane === 'market.bars.1m') {
        const [b] = normalise([f.payload])
        if (!b) return
        setLiveBars((prev) => {
          const next = prev.filter((x) => String(x.ts) !== String(b.ts))
          next.push(b)
          return next.slice(-600)
        })
        setLastUpdate(Date.now())
        return
      }

      if (f.lane === 'market.trades') {
        // Extend the in-progress candle so the chart moves between minute
        // boundaries. The completed bar from the aggregator replaces it.
        const price = Number((f.payload as unknown as { price?: string })?.price)
        if (!Number.isFinite(price)) return
        const minute = Math.floor(Date.now() / 60000) * 60
        setLiveBars((prev) => {
          const next = [...prev]
          const i = next.findIndex((x) => Number(x.ts) === minute)
          if (i >= 0) {
            const cur = next[i]
            next[i] = {
              ...cur,
              high: Math.max(cur.high, price),
              low: Math.min(cur.low, price),
              close: price,
            }
          } else {
            next.push({ ts: minute, open: price, high: price, low: price, close: price, volume: 0 })
          }
          return next.slice(-600)
        })
        setLastUpdate(Date.now())
      }
    })
    return () => {
      off()
      client?.unsubscribe(id)
    }
  }, [instrument, tfSecs])

  const bars = useMemo(() => {
    const base = rest.data ?? []
    if (!liveBars.length) return base
    const lastRest = base.length ? Number(base[base.length - 1].ts) : 0
    const tail = liveBars.filter((b) => Number(b.ts) > lastRest)
    return tail.length ? [...base, ...tail] : base
  }, [rest.data, liveBars])

  return {
    bars,
    loading: rest.isLoading,
    error: rest.isError,
    refetch: () => void rest.refetch(),
    lastUpdate: lastUpdate ?? (rest.dataUpdatedAt || null),
  }
}
