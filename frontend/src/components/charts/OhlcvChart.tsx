import { useEffect, useRef } from 'react'
import {
  createChart, CandlestickSeries, createSeriesMarkers,
  type IChartApi, type ISeriesApi, type CandlestickData, type Time,
} from 'lightweight-charts'
import { useThemeStore } from '@/store/theme'
import { chartColors } from '@/lib/chartTheme'

export interface Bar {
  ts: string | number
  open: number
  high: number
  low: number
  close: number
  volume?: number
}

export interface TradeMarker {
  ts: string | number
  side: 'buy' | 'sell'
  price: number
  qty?: number
}

interface Props {
  bars: Bar[]
  markers?: TradeMarker[]
  height?: number
}

function toTime(ts: string | number): Time {
  if (typeof ts === 'number') return ts as unknown as Time
  return Math.floor(new Date(ts).getTime() / 1000) as unknown as Time
}

export function OhlcvChart({ bars, markers = [], height = 360 }: Props) {
  const containerRef = useRef<HTMLDivElement>(null)
  const chartRef = useRef<IChartApi | null>(null)
  const candleRef = useRef<ISeriesApi<'Candlestick'> | null>(null)
  const theme = useThemeStore((s) => s.theme)

  useEffect(() => {
    if (!containerRef.current) return

    const colors = chartColors()

    const chart = createChart(containerRef.current, {
      layout: {
        background: { color: 'transparent' },
        textColor: colors.text,
      },
      grid: {
        vertLines: { color: colors.grid },
        horzLines: { color: colors.grid },
      },
      crosshair: { mode: 1 },
      rightPriceScale: { borderColor: colors.border },
      timeScale: { borderColor: colors.border, timeVisible: true, secondsVisible: false },
    })
    chartRef.current = chart

    const candle = chart.addSeries(CandlestickSeries, {
      upColor: colors.pnlUp,
      downColor: colors.pnlDown,
      borderUpColor: colors.pnlUp,
      borderDownColor: colors.pnlDown,
      wickUpColor: colors.pnlUp,
      wickDownColor: colors.pnlDown,
    })
    candleRef.current = candle

    const ro = new ResizeObserver(() => {
      if (containerRef.current) {
        chart.applyOptions({ width: containerRef.current.clientWidth })
      }
    })
    ro.observe(containerRef.current)

    return () => {
      ro.disconnect()
      chart.remove()
      candleRef.current = null
      chartRef.current = null
    }
  }, [theme])

  useEffect(() => {
    if (!candleRef.current || bars.length === 0) return

    const sorted = [...bars].sort((a, b) => {
      const ta = typeof a.ts === 'number' ? a.ts : new Date(a.ts).getTime()
      const tb = typeof b.ts === 'number' ? b.ts : new Date(b.ts).getTime()
      return ta - tb
    })

    const data: CandlestickData[] = sorted.map((b) => ({
      time: toTime(b.ts),
      open: b.open,
      high: b.high,
      low: b.low,
      close: b.close,
    }))
    candleRef.current.setData(data)

    if (markers.length > 0) {
      const colors = chartColors()
      const seriesMarkers = markers.map((m) => ({
        time: toTime(m.ts),
        position: m.side === 'buy' ? ('belowBar' as const) : ('aboveBar' as const),
        color: m.side === 'buy' ? colors.pnlUp : colors.pnlDown,
        shape: m.side === 'buy' ? ('arrowUp' as const) : ('arrowDown' as const),
        text: `${m.side.toUpperCase()}${m.qty ? ` ${m.qty}` : ''}`,
      }))
      createSeriesMarkers(candleRef.current, seriesMarkers)
    }

    chartRef.current?.timeScale().fitContent()
  }, [JSON.stringify(bars), JSON.stringify(markers), theme])

  return <div ref={containerRef} style={{ height }} className="w-full" />
}
