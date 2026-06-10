import { useEffect, useRef } from 'react'
import { createChart, AreaSeries, type IChartApi, type ISeriesApi } from 'lightweight-charts'
import { useThemeStore } from '@/store/theme'
import { chartColors } from '@/lib/chartTheme'

interface PnlPoint {
  time: string | number
  value: number
}

interface Props {
  data?: PnlPoint[] | { series?: PnlPoint[]; buckets?: PnlPoint[] } | null
}

function normalizeData(data: Props['data']): PnlPoint[] {
  if (!data) return []
  if (Array.isArray(data)) return data
  if ('series' in data && Array.isArray(data.series)) return data.series
  if ('buckets' in data && Array.isArray(data.buckets)) return data.buckets
  return []
}

export function PnlChart({ data }: Props) {
  const containerRef = useRef<HTMLDivElement>(null)
  const chartRef = useRef<IChartApi | null>(null)
  // Keep a ref to the series so data updates don't require chart teardown (L-9).
  const seriesRef = useRef<ISeriesApi<'Area'> | null>(null)

  const points = normalizeData(data)
  const theme = useThemeStore((s) => s.theme)

  // Init effect — runs only when theme changes, not on every data update (L-9).
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
      timeScale: { borderColor: colors.border, timeVisible: true },
      handleScroll: true,
      handleScale: true,
    })

    chartRef.current = chart

    seriesRef.current = chart.addSeries(AreaSeries, {
      lineColor: colors.accent,
      topColor: 'rgba(59,130,246,0.25)',
      bottomColor: 'rgba(59,130,246,0)',
      lineWidth: 2,
    })

    const ro = new ResizeObserver(() => {
      if (containerRef.current) {
        chart.applyOptions({ width: containerRef.current.clientWidth })
      }
    })
    ro.observe(containerRef.current)

    return () => {
      ro.disconnect()
      chart.remove()
      chartRef.current = null
      seriesRef.current = null
    }
  }, [theme])

  // Data update effect — runs when data changes, does NOT recreate the chart (L-9).
  // Uses the stable points reference instead of JSON.stringify (L-8).
  useEffect(() => {
    if (!seriesRef.current || points.length === 0) return

    const sorted = [...points].sort((a, b) => {
      const ta = typeof a.time === 'string' ? new Date(a.time).getTime() : a.time
      const tb = typeof b.time === 'string' ? new Date(b.time).getTime() : b.time
      return ta - tb
    })
    const mapped = sorted.map((p) => ({
      time: (typeof p.time === 'string' ? Math.floor(new Date(p.time).getTime() / 1000) : p.time) as number,
      value: p.value,
    }))
    seriesRef.current.setData(mapped as Parameters<typeof seriesRef.current.setData>[0])
    chartRef.current?.timeScale().fitContent()
  }, [points])

  return <div ref={containerRef} className="h-48 w-full" />
}
