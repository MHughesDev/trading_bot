import { useEffect, useRef } from 'react'
import { createChart, AreaSeries, type IChartApi } from 'lightweight-charts'
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

  const points = normalizeData(data)
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
      timeScale: { borderColor: colors.border, timeVisible: true },
      handleScroll: true,
      handleScale: true,
    })

    chartRef.current = chart

    const series = chart.addSeries(AreaSeries, {
      lineColor: colors.accent,
      topColor: 'rgba(59,130,246,0.25)',
      bottomColor: 'rgba(59,130,246,0)',
      lineWidth: 2,
    })

    if (points.length > 0) {
      const sorted = [...points].sort((a, b) => {
        const ta = typeof a.time === 'string' ? new Date(a.time).getTime() : a.time
        const tb = typeof b.time === 'string' ? new Date(b.time).getTime() : b.time
        return ta - tb
      })
      const mapped = sorted.map((p) => ({
        time: (typeof p.time === 'string' ? Math.floor(new Date(p.time).getTime() / 1000) : p.time) as number,
        value: p.value,
      }))
      series.setData(mapped as Parameters<typeof series.setData>[0])
      chart.timeScale().fitContent()
    }

    const ro = new ResizeObserver(() => {
      if (containerRef.current) {
        chart.applyOptions({ width: containerRef.current.clientWidth })
      }
    })
    ro.observe(containerRef.current)

    return () => {
      ro.disconnect()
      chart.remove()
    }
  }, [JSON.stringify(points), theme])

  return <div ref={containerRef} className="h-48 w-full" />
}
