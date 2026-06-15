import { useEffect, useRef } from 'react'
import {
  createChart, CandlestickSeries, LineSeries, HistogramSeries,
  createSeriesMarkers, LineStyle,
  type IChartApi, type ISeriesApi, type Time,
} from 'lightweight-charts'
import { useThemeStore } from '@/store/theme'
import { chartColors } from '@/lib/chartTheme'
import type { PriceLineAnnotation } from './Annotations'
import { calcEMA, calcSMA, calcBB, calcRSI, calcMACD } from '@/utils/indicators'

// ── public types (re-exported for ChartPanel) ─────────────────────────────────

export type IndicatorKind = 'ema' | 'sma' | 'bb' | 'volume' | 'rsi' | 'macd'

export interface IndicatorInstance {
  uid: string
  kind: IndicatorKind
  color?: string
  // EMA / SMA / RSI
  period?: number
  // BB
  stddev?: number
  // MACD
  fast?: number
  slow?: number
  signal?: number
}

export interface Bar {
  ts: string | number
  open: number; high: number; low: number; close: number; volume?: number
}

export interface TradeMarker {
  ts: string | number; side: 'buy' | 'sell'; price: number; qty?: number
}

// ── helpers ───────────────────────────────────────────────────────────────────

function toTime(ts: string | number): Time {
  if (typeof ts === 'number') return ts as unknown as Time
  return Math.floor(new Date(ts).getTime() / 1000) as unknown as Time
}

type AnySeries = ISeriesApi<'Line' | 'Candlestick' | 'Histogram'>

function makeChart(
  el: HTMLDivElement,
  colors: ReturnType<typeof chartColors>,
  extra?: object,
): IChartApi {
  return createChart(el, {
    layout: { background: { color: 'transparent' }, textColor: colors.text },
    grid:   { vertLines: { color: colors.grid }, horzLines: { color: colors.grid } },
    crosshair: { mode: 1 },
    rightPriceScale: { borderColor: colors.border },
    timeScale: { borderColor: colors.border, timeVisible: true, secondsVisible: false },
    ...extra,
  })
}

const SUB_H = { volume: 64, rsi: 88, macd: 110 }

// ── component ─────────────────────────────────────────────────────────────────

interface Props {
  bars: Bar[]
  indicators?: IndicatorInstance[]
  markers?: TradeMarker[]
  priceLines?: PriceLineAnnotation[]
  mainHeight?: number
}

export function MultiPaneChart({
  bars,
  indicators = [],
  markers = [],
  priceLines = [],
  mainHeight = 300,
}: Props) {
  const theme = useThemeStore((s) => s.theme)

  // container divs
  const mainRef = useRef<HTMLDivElement>(null)
  const volRef  = useRef<HTMLDivElement>(null)
  const rsiRef  = useRef<HTMLDivElement>(null)
  const macdRef = useRef<HTMLDivElement>(null)

  // chart instances
  const mainChart = useRef<IChartApi | null>(null)
  const volChart  = useRef<IChartApi | null>(null)
  const rsiChart  = useRef<IChartApi | null>(null)
  const macdChart = useRef<IChartApi | null>(null)

  // series
  const candleSeries  = useRef<ISeriesApi<'Candlestick'> | null>(null)
  const overlayMap    = useRef<Map<string, AnySeries[]>>(new Map())  // uid → series[]
  const priceLineMap  = useRef<Map<string, unknown>>(new Map())
  const volSeriesRef  = useRef<ISeriesApi<'Histogram'> | null>(null)
  const rsiSeriesRef  = useRef<AnySeries[]>([])
  const macdSeriesRef = useRef<AnySeries[]>([])
  const lastBarCount  = useRef(0)
  const syncing       = useRef(false)

  // ── chart lifecycle (recreate on theme change) ────────────────────────────
  useEffect(() => {
    if (!mainRef.current) return
    const colors = chartColors()

    const mc = makeChart(mainRef.current, colors)
    mainChart.current = mc
    const cs = mc.addSeries(CandlestickSeries, {
      upColor: colors.pnlUp, downColor: colors.pnlDown,
      borderUpColor: colors.pnlUp, borderDownColor: colors.pnlDown,
      wickUpColor:   colors.pnlUp, wickDownColor:   colors.pnlDown,
    })
    candleSeries.current = cs

    const subOpts = {
      timeScale: { visible: false, borderColor: colors.border },
      leftPriceScale: { visible: false },
      handleScroll: false,
      handleScale:  false,
    }
    if (volRef.current)  volChart.current  = makeChart(volRef.current,  colors, subOpts)
    if (rsiRef.current)  rsiChart.current  = makeChart(rsiRef.current,  colors, subOpts)
    if (macdRef.current) macdChart.current = makeChart(macdRef.current, colors, subOpts)

    // sync sub-panes to main time scale (time-based, not index-based, so that
    // sub-pane series with fewer bars — e.g. MACD needing 35 warm-up bars —
    // stay aligned with the candles by timestamp rather than bar index)
    const subs = [volChart, rsiChart, macdChart]
      .map((r) => r.current).filter(Boolean) as IChartApi[]

    mc.timeScale().subscribeVisibleTimeRangeChange((range) => {
      if (syncing.current || !range) return
      syncing.current = true
      subs.forEach((c) => { try { c.timeScale().setVisibleRange(range) } catch { /* no data yet */ } })
      syncing.current = false
    })

    const ro = new ResizeObserver(() => {
      if (mainRef.current) mc.applyOptions({ width: mainRef.current.clientWidth })
      ;[volRef, rsiRef, macdRef].forEach((r, i) => {
        if (r.current) subs[i]?.applyOptions({ width: r.current.clientWidth })
      })
    })
    ro.observe(mainRef.current)
    lastBarCount.current = 0

    return () => {
      ro.disconnect()
      mc.remove();         mainChart.current = null; candleSeries.current = null
      volChart.current?.remove();  volChart.current  = null; volSeriesRef.current  = null
      rsiChart.current?.remove();  rsiChart.current  = null; rsiSeriesRef.current  = []
      macdChart.current?.remove(); macdChart.current = null; macdSeriesRef.current = []
      overlayMap.current.clear()
      lastBarCount.current = 0
    }
  }, [theme])

  // ── data + indicators ─────────────────────────────────────────────────────
  useEffect(() => {
    if (!candleSeries.current || !mainChart.current || bars.length === 0) return
    const colors = chartColors()

    const sorted = [...bars].sort((a, b) => {
      const ta = typeof a.ts === 'number' ? a.ts : new Date(a.ts).getTime()
      const tb = typeof b.ts === 'number' ? b.ts : new Date(b.ts).getTime()
      return ta - tb
    })

    const times  = sorted.map((b) => toTime(b.ts))
    const closes = sorted.map((b) => b.close)
    const candleData = sorted.map((b) => ({
      time: toTime(b.ts), open: b.open, high: b.high, low: b.low, close: b.close,
    }))

    // smart incremental vs full-load
    const prev = lastBarCount.current
    const full = prev === 0 || Math.abs(candleData.length - prev) > 2
    lastBarCount.current = candleData.length

    if (full) {
      candleSeries.current.setData(candleData)
      if (candleData.length > 0) {
        const ts = mainChart.current.timeScale()
        ts.setVisibleRange({
          from: candleData[Math.max(0, candleData.length - 120)].time,
          to:   candleData[candleData.length - 1].time,
        })
      }
    } else {
      const last = candleData[candleData.length - 1]
      if (last) candleSeries.current.update(last)
    }

    // markers
    if (markers.length > 0) {
      createSeriesMarkers(candleSeries.current, markers.map((m) => ({
        time: toTime(m.ts),
        position: m.side === 'buy' ? ('belowBar' as const) : ('aboveBar' as const),
        color: m.side === 'buy' ? colors.pnlUp : colors.pnlDown,
        shape: m.side === 'buy' ? ('arrowUp' as const) : ('arrowDown' as const),
        text:  `${m.side.toUpperCase()}${m.qty ? ` ${m.qty}` : ''}`,
      })))
    }

    // helper: build [{time, value}] dropping NaN
    const validPoints = (vals: number[]) =>
      vals
        .map((v, i) => (isNaN(v) ? null : { time: times[i], value: v }))
        .filter(Boolean) as { time: Time; value: number }[]

    // ── clear stale overlay series ─────────────────────────────────────────
    const activeUids = new Set(indicators.map((i) => i.uid))
    for (const [uid, series] of overlayMap.current) {
      if (!activeUids.has(uid)) {
        series.forEach((s) => {
          try { mainChart.current?.removeSeries(s as ISeriesApi<'Line'>) } catch { /* ok */ }
        })
        overlayMap.current.delete(uid)
      }
    }

    // ── overlay indicators ─────────────────────────────────────────────────
    const thin = (color: string) => ({
      color,
      lineWidth: 1 as const,
      lastValueVisible: false,
      priceLineVisible: false,
    })

    for (const inst of indicators.filter((i) => ['ema', 'sma', 'bb'].includes(i.kind))) {
      if (overlayMap.current.has(inst.uid)) continue  // already added — just let data update handle it

      const color = inst.color ?? '#94a3b8'

      if (inst.kind === 'bb') {
        const { upper, middle, lower } = calcBB(closes, inst.period ?? 20, inst.stddev ?? 2)
        const addBand = (vals: number[], ls = LineStyle.Solid) => {
          const s = mainChart.current!.addSeries(LineSeries, { ...thin(color), lineStyle: ls })
          s.setData(validPoints(vals))
          return s as AnySeries
        }
        overlayMap.current.set(inst.uid, [
          addBand(upper, LineStyle.Dashed),
          addBand(middle),
          addBand(lower, LineStyle.Dashed),
        ])
      } else {
        const vals = inst.kind === 'ema'
          ? calcEMA(closes, inst.period ?? 20)
          : calcSMA(closes, inst.period ?? 20)
        const s = mainChart.current!.addSeries(LineSeries, { ...thin(color) })
        s.setData(validPoints(vals))
        overlayMap.current.set(inst.uid, [s as AnySeries])
      }
    }

    // also refresh data for already-existing overlay series if this was a full reload
    if (full) {
      for (const [uid, series] of overlayMap.current) {
        const inst = indicators.find((i) => i.uid === uid)
        if (!inst) continue
        if (inst.kind === 'bb') {
          const { upper, middle, lower } = calcBB(closes, inst.period ?? 20, inst.stddev ?? 2)
          const bands = [upper, middle, lower]
          series.forEach((s, i) => (s as ISeriesApi<'Line'>).setData(validPoints(bands[i])))
        } else {
          const vals = inst.kind === 'ema'
            ? calcEMA(closes, inst.period ?? 20)
            : calcSMA(closes, inst.period ?? 20)
          series[0] && (series[0] as ISeriesApi<'Line'>).setData(validPoints(vals))
        }
      }
    }

    // ── VOLUME sub-pane ────────────────────────────────────────────────────
    if (volChart.current) {
      if (volSeriesRef.current) {
        try { volChart.current.removeSeries(volSeriesRef.current) } catch { /* ok */ }
        volSeriesRef.current = null
      }
      const volInsts = indicators.filter((i) => i.kind === 'volume')
      if (volInsts.length > 0) {
        const vs = volChart.current.addSeries(HistogramSeries, {
          color: colors.pnlUp,
          priceFormat: { type: 'volume' },
          lastValueVisible: false,
          priceLineVisible: false,
        })
        vs.setData(sorted.map((b) => ({
          time:  toTime(b.ts),
          value: b.volume ?? 0,
          color: b.close >= b.open ? `${colors.pnlUp}99` : `${colors.pnlDown}99`,
        })))
        volSeriesRef.current = vs
      }
    }

    // ── RSI sub-pane ───────────────────────────────────────────────────────
    if (rsiChart.current) {
      rsiSeriesRef.current.forEach((s) => {
        try { rsiChart.current!.removeSeries(s as ISeriesApi<'Line'>) } catch { /* ok */ }
      })
      rsiSeriesRef.current = []
      const rsiInsts = indicators.filter((i) => i.kind === 'rsi')
      const rsiColors = ['#a78bfa', '#34d399', '#f59e0b', '#f87171']
      for (const [idx, inst] of rsiInsts.entries()) {
        const rsiVals = calcRSI(closes, inst.period ?? 14)
        const color = rsiColors[idx % rsiColors.length]
        const rs = rsiChart.current.addSeries(LineSeries, {
          color, lineWidth: 1, lastValueVisible: true, priceLineVisible: false,
        })
        rs.setData(validPoints(rsiVals))
        if (idx === 0) {
          rs.createPriceLine({ price: 70, color: '#ef444466', lineWidth: 1, lineStyle: LineStyle.Dashed, axisLabelVisible: true, title: '' })
          rs.createPriceLine({ price: 30, color: '#22c55e66', lineWidth: 1, lineStyle: LineStyle.Dashed, axisLabelVisible: true, title: '' })
        }
        rsiSeriesRef.current.push(rs as AnySeries)
      }
    }

    // ── MACD sub-pane ──────────────────────────────────────────────────────
    if (macdChart.current) {
      macdSeriesRef.current.forEach((s) => {
        try { macdChart.current!.removeSeries(s as ISeriesApi<'Line'>) } catch { /* ok */ }
      })
      macdSeriesRef.current = []
      const macdInst = indicators.find((i) => i.kind === 'macd')
      if (macdInst) {
        const { macdLine, signalLine, histogram } = calcMACD(
          closes,
          macdInst.fast   ?? 12,
          macdInst.slow   ?? 26,
          macdInst.signal ?? 9,
        )
        const hist = macdChart.current.addSeries(HistogramSeries, {
          color: '#3b82f6', lastValueVisible: false, priceLineVisible: false,
        })
        hist.setData(
          histogram.map((v, i) => isNaN(v) ? null : {
            time: times[i], value: v,
            color: v >= 0 ? '#22c55e99' : '#ef444499',
          }).filter(Boolean) as { time: Time; value: number; color: string }[],
        )
        const ml = macdChart.current.addSeries(LineSeries, {
          color: '#3b82f6', lineWidth: 1, lastValueVisible: false, priceLineVisible: false,
        })
        ml.setData(validPoints(macdLine))
        const sl = macdChart.current.addSeries(LineSeries, {
          color: '#f59e0b', lineWidth: 1, lastValueVisible: false, priceLineVisible: false,
        })
        sl.setData(validPoints(signalLine))
        macdSeriesRef.current = [hist as AnySeries, ml as AnySeries, sl as AnySeries]
      }
    }

    // Sync sub-pane chart heights to match their CSS containers.
    // lightweight-charts v5 doesn't auto-detect height when the container
    // starts at 0px and grows, so we push the correct height explicitly.
    const hasVol  = indicators.some((i) => i.kind === 'volume')
    const hasRsi  = indicators.some((i) => i.kind === 'rsi')
    const hasMacd = indicators.some((i) => i.kind === 'macd')
    volChart.current?.applyOptions({ height: hasVol  ? SUB_H.volume : 0 })
    rsiChart.current?.applyOptions({ height: hasRsi  ? SUB_H.rsi   : 0 })
    macdChart.current?.applyOptions({ height: hasMacd ? SUB_H.macd  : 0 })

    // sync sub-pane time ranges after data load
    const range = mainChart.current.timeScale().getVisibleRange()
    if (range) {
      ;[volChart, rsiChart, macdChart].forEach((r) => {
        try { r.current?.timeScale().setVisibleRange(range) } catch { /* no data yet */ }
      })
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bars, markers, indicators])

  // ── price lines ───────────────────────────────────────────────────────────
  useEffect(() => {
    if (!candleSeries.current) return
    const series = candleSeries.current
    const existing = priceLineMap.current
    const incoming = new Map(priceLines.map((pl) => [pl.id, pl]))
    type PL      = Parameters<typeof series.removePriceLine>[0]
    type PLApply = { applyOptions: (o: object) => void }
    for (const [id, pl] of existing) {
      if (!incoming.has(id)) { series.removePriceLine(pl as PL); existing.delete(id) }
    }
    const lsMap = { dashed: LineStyle.Dashed, solid: LineStyle.Solid, dotted: LineStyle.Dotted }
    for (const [id, pl] of incoming) {
      const opts = { price: pl.price, color: pl.color, lineWidth: 1 as const, lineStyle: lsMap[pl.lineStyle ?? 'dashed'], axisLabelVisible: true, title: pl.label ?? '' }
      if (existing.has(id)) { (existing.get(id) as PLApply).applyOptions(opts) }
      else { existing.set(id, series.createPriceLine(opts)) }
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [priceLines])

  const hasVol  = indicators.some((i) => i.kind === 'volume')
  const hasRsi  = indicators.some((i) => i.kind === 'rsi')
  const hasMacd = indicators.some((i) => i.kind === 'macd')

  return (
    <div style={{ display: 'flex', flexDirection: 'column', width: '100%' }}>
      <div ref={mainRef} style={{ width: '100%', height: mainHeight }} />
      <div
        ref={volRef}
        style={{
          width: '100%', overflow: 'hidden',
          height: hasVol ? SUB_H.volume : 0,
          borderTop: hasVol ? '1px solid var(--tb-border)' : 'none',
        }}
      />
      <div
        ref={rsiRef}
        style={{
          width: '100%', overflow: 'hidden',
          height: hasRsi ? SUB_H.rsi : 0,
          borderTop: hasRsi ? '1px solid var(--tb-border)' : 'none',
        }}
      />
      <div
        ref={macdRef}
        style={{
          width: '100%', overflow: 'hidden',
          height: hasMacd ? SUB_H.macd : 0,
          borderTop: hasMacd ? '1px solid var(--tb-border)' : 'none',
        }}
      />
    </div>
  )
}
