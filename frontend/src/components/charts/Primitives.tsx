import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { compactMoney, money, pct, shortDate, signedPct } from '@/lib/format'

/* =============================================================================
   Hand-rolled SVG chart primitives — spec §2.8.

   Rules honoured here:
     - every colour is a --chart-* token; no library brings its own palette
     - text in and around a chart wears TEXT tokens, never the series colour
     - crosshair + tooltip on every line/area chart
     - a table view toggle exposing the same data
     - horizontal gridlines only by default
     - no dual axis, ever
     - no animation on data append
   ============================================================================= */

export interface Pt {
  t: number
  v: number
  /** Optional annotation, e.g. a deposit marker. */
  mark?: { label: string }
}

/* -----------------------------------------------------------------------------
   Sparkline — no axes, no interaction. Used inside the hero equity tile.
   -------------------------------------------------------------------------- */
export function Sparkline({
  points,
  width = 108,
  height = 28,
  stroke = 'var(--chart-equity)',
  className,
  ariaLabel,
}: {
  points: number[]
  width?: number
  height?: number
  stroke?: string
  className?: string
  ariaLabel?: string
}) {
  if (points.length < 2) return null
  const min = Math.min(...points)
  const max = Math.max(...points)
  const span = max - min || 1
  const d = points
    .map((p, i) => {
      const x = (i / (points.length - 1)) * width
      const y = height - ((p - min) / span) * height
      return `${i === 0 ? 'M' : 'L'}${x.toFixed(2)} ${y.toFixed(2)}`
    })
    .join(' ')

  return (
    <svg
      className={className}
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={ariaLabel ?? 'Trend sparkline'}
      style={{ overflow: 'visible' }}
    >
      <path d={d} fill="none" stroke={stroke} strokeWidth={1.5} strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  )
}

/* -----------------------------------------------------------------------------
   Area / line chart with crosshair, tooltip, deposit markers and a table view.
   -------------------------------------------------------------------------- */

export interface AreaChartProps {
  points: Pt[]
  currency?: string
  height?: number
  /** Series label shown in the tooltip and the table caption. */
  label?: string
  /** Formats the value in the axis and tooltip. */
  format?: (v: number) => string
  color?: string
  areaFrom?: string
  areaTo?: string
  className?: string
  ariaLabel?: string
  /** Show the y axis on the right, as the mockups do. */
  yTicks?: number
}

export function AreaChart({
  points,
  currency = 'USD',
  height = 260,
  label = 'Equity',
  format,
  color = 'var(--chart-equity)',
  areaFrom = 'var(--chart-area-from)',
  areaTo = 'var(--chart-area-to)',
  className,
  ariaLabel,
  yTicks = 5,
}: AreaChartProps) {
  const gid = useId().replace(/:/g, '')
  const wrapRef = useRef<HTMLDivElement>(null)
  const [w, setW] = useState(720)
  const [hoverIdx, setHoverIdx] = useState<number | null>(null)

  useEffect(() => {
    const el = wrapRef.current
    if (!el) return
    const ro = new ResizeObserver(([e]) => setW(Math.max(240, e.contentRect.width)))
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  const padL = 8
  const padR = 62
  const padT = 12
  const padB = 26
  const innerW = Math.max(10, w - padL - padR)
  const innerH = Math.max(10, height - padT - padB)

  const fmt = format ?? ((v: number) => compactMoney(v, currency))

  const geom = useMemo(() => {
    if (points.length < 2) return null
    const vs = points.map((p) => p.v)
    let min = Math.min(...vs)
    let max = Math.max(...vs)
    const pad = (max - min) * 0.12 || Math.abs(max) * 0.02 || 1
    min -= pad
    max += pad
    const span = max - min || 1
    const x = (i: number) => padL + (i / (points.length - 1)) * innerW
    const y = (v: number) => padT + innerH - ((v - min) / span) * innerH
    const line = points.map((p, i) => `${i === 0 ? 'M' : 'L'}${x(i).toFixed(2)} ${y(p.v).toFixed(2)}`).join(' ')
    const area = `${line} L${x(points.length - 1).toFixed(2)} ${(padT + innerH).toFixed(2)} L${x(0).toFixed(2)} ${(padT + innerH).toFixed(2)} Z`
    const ticks = Array.from({ length: yTicks }, (_, i) => min + (span * (i + 0.5)) / yTicks)
    return { x, y, line, area, min, max, ticks }
  }, [points, innerW, innerH, yTicks])

  if (!geom) return null

  const hover = hoverIdx !== null ? points[hoverIdx] : null

  function onMove(e: React.MouseEvent<SVGSVGElement>) {
    const rect = e.currentTarget.getBoundingClientRect()
    const rel = e.clientX - rect.left - padL
    const idx = Math.round((rel / innerW) * (points.length - 1))
    setHoverIdx(Math.max(0, Math.min(points.length - 1, idx)))
  }

  const first = points[0]
  const last = points[points.length - 1]
  const change = first.v !== 0 ? ((last.v - first.v) / Math.abs(first.v)) * 100 : 0

  return (
    <div ref={wrapRef} className={cn('col-flex', className)} style={{ position: 'relative', width: '100%' }}>
      <svg
        width={w}
        height={height}
        role="img"
        aria-label={
          ariaLabel ??
          `${label} from ${shortDate(first.t * 1000)} to ${shortDate(last.t * 1000)}, ${fmt(first.v)} to ${fmt(last.v)}, ${signedPct(change)}`
        }
        onMouseMove={onMove}
        onMouseLeave={() => setHoverIdx(null)}
        style={{ display: 'block', cursor: 'crosshair' }}
      >
        <defs>
          <linearGradient id={`g-${gid}`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={areaFrom} />
            <stop offset="100%" stopColor={areaTo} />
          </linearGradient>
        </defs>

        {/* horizontal gridlines only (V5) */}
        {geom.ticks.map((t, i) => (
          <g key={i}>
            <line
              x1={padL}
              x2={padL + innerW}
              y1={geom.y(t)}
              y2={geom.y(t)}
              stroke="var(--chart-grid)"
              strokeWidth={1}
            />
            <text
              x={padL + innerW + 8}
              y={geom.y(t) + 3}
              fontSize={10}
              fill="var(--chart-axis)"
              className="num"
            >
              {fmt(t)}
            </text>
          </g>
        ))}

        <path d={geom.area} fill={`url(#g-${gid})`} />
        <path d={geom.line} fill="none" stroke={color} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />

        {/* deposit / external cash-movement markers, labelled */}
        {points.map((p, i) =>
          p.mark ? (
            <g key={`m-${i}`}>
              <line
                x1={geom.x(i)}
                x2={geom.x(i)}
                y1={padT}
                y2={padT + innerH}
                stroke="var(--chart-grid-major)"
                strokeDasharray="3 3"
              />
              <circle cx={geom.x(i)} cy={geom.y(p.v)} r={3.5} fill="var(--chart-surface)" stroke={color} strokeWidth={1.5} />
              <text x={geom.x(i) + 6} y={padT + 12} fontSize={10} fill="var(--fg-tertiary)">
                {p.mark.label}
              </text>
            </g>
          ) : null,
        )}

        {/* last point */}
        <circle cx={geom.x(points.length - 1)} cy={geom.y(last.v)} r={3.5} fill={color} />

        {/* crosshair */}
        {hoverIdx !== null && hover && (
          <g>
            <line
              x1={geom.x(hoverIdx)}
              x2={geom.x(hoverIdx)}
              y1={padT}
              y2={padT + innerH}
              stroke="var(--chart-crosshair)"
              strokeWidth={1}
              strokeDasharray="2 2"
            />
            <circle cx={geom.x(hoverIdx)} cy={geom.y(hover.v)} r={4} fill="var(--chart-surface)" stroke={color} strokeWidth={2} />
          </g>
        )}

        {/* x axis */}
        <text x={padL} y={height - 8} fontSize={10} fill="var(--chart-axis)">
          {shortDate(first.t * 1000)}
        </text>
        <text x={padL + innerW} y={height - 8} fontSize={10} fill="var(--chart-axis)" textAnchor="end">
          {shortDate(last.t * 1000)}
        </text>
      </svg>

      {hoverIdx !== null && hover && (
        <div
          className="tooltip"
          style={{
            position: 'absolute',
            left: Math.min(Math.max(0, geom.x(hoverIdx) - 60), w - 150),
            top: 6,
            pointerEvents: 'none',
          }}
        >
          <div className="lbl">{shortDate(hover.t * 1000)}</div>
          <div className="num" style={{ fontSize: 'var(--t-13)', fontWeight: 'var(--w-semibold)' }}>
            {money(hover.v, currency)}
          </div>
          {hover.mark && <div className="lbl">{hover.mark.label}</div>}
        </div>
      )}
    </div>
  )
}

/* -----------------------------------------------------------------------------
   Stacked allocation bar. Segments read from ONE array in ONE fixed slot order.
   -------------------------------------------------------------------------- */
export interface AllocSegment {
  key: string
  label: string
  pct: number
  value: number
  color: string
}

export function AllocationBar({ segments, className }: { segments: AllocSegment[]; className?: string }) {
  const total = segments.reduce((s, x) => s + x.pct, 0) || 1
  return (
    <div
      className={cn('allocbar', className)}
      role="img"
      aria-label={`Allocation: ${segments.map((s) => `${s.label} ${pct(s.pct)}`).join(', ')}`}
    >
      {segments
        .filter((s) => s.pct > 0)
        .map((s) => (
          <i key={s.key} style={{ width: `${(s.pct / total) * 100}%`, background: s.color }} title={`${s.label} ${pct(s.pct)}`} />
        ))}
    </div>
  )
}

/** The allocation cell's mini bar — scaled to the LARGEST class, not to 100%. */
export function MiniBar({ pct: p, max, color, width = 56 }: { pct: number; max: number; color: string; width?: number }) {
  const w = max > 0 ? Math.max(0, Math.min(100, (p / max) * 100)) : 0
  return (
    <span className="minibar" style={{ width, display: 'inline-block' }} aria-hidden>
      <i style={{ width: `${w}%`, background: color }} />
    </span>
  )
}

/** A labelled progress meter, e.g. margin utilisation against its limit. */
export function Meter({
  value,
  max,
  tone = 'accent',
  className,
  ariaLabel,
}: {
  value: number
  max: number
  tone?: 'accent' | 'pos' | 'warn' | 'neg'
  className?: string
  ariaLabel?: string
}) {
  const p = max > 0 ? Math.max(0, Math.min(100, (value / max) * 100)) : 0
  return (
    <div
      className={cn('meter', className)}
      role="progressbar"
      aria-valuenow={Math.round(p)}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-label={ariaLabel}
    >
      <i className={tone === 'accent' ? undefined : tone} style={{ width: `${p}%` }} />
    </div>
  )
}

/* -----------------------------------------------------------------------------
   Table view — every chart ships one, keyboard reachable (§2.8.4).
   -------------------------------------------------------------------------- */
export function ChartTableView({
  points,
  currency = 'USD',
  caption,
}: {
  points: Pt[]
  currency?: string
  caption: string
}) {
  return (
    <div className="tblwrap" style={{ maxHeight: 260 }}>
      <table className="tbl dense">
        <caption className="sr-only">{caption}</caption>
        <thead>
          <tr>
            <th scope="col" className="l">Date</th>
            <th scope="col">Value</th>
            <th scope="col">Change</th>
          </tr>
        </thead>
        <tbody>
          {points.map((p, i) => {
            const prev = i > 0 ? points[i - 1].v : p.v
            const ch = prev !== 0 ? ((p.v - prev) / Math.abs(prev)) * 100 : 0
            return (
              <tr key={p.t}>
                <td className="l">{shortDate(p.t * 1000)}</td>
                <td className="n">{money(p.v, currency)}</td>
                <td className={cn('n', ch > 0 && 'pos', ch < 0 && 'neg')}>{signedPct(ch)}</td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

/* -----------------------------------------------------------------------------
   Legend — always present for >= 2 series (§2.8.4).
   -------------------------------------------------------------------------- */
export function Legend({
  items,
  className,
}: {
  items: { key: string; label: ReactNode; color: string; value?: ReactNode; meta?: ReactNode }[]
  className?: string
}) {
  return (
    <div className={cn('col-flex', className)}>
      {items.map((it) => (
        <div
          key={it.key}
          style={{
            display: 'grid',
            gridTemplateColumns: '10px 1fr auto auto',
            gap: 'var(--s-3)',
            alignItems: 'center',
            height: 26,
            fontSize: 'var(--t-12)',
          }}
        >
          <i className="swatch lg" style={{ background: it.color }} aria-hidden />
          <span className="truncate-1">{it.label}</span>
          <span className="num" style={{ fontWeight: 'var(--w-semibold)', width: 58, textAlign: 'right' }}>
            {it.value}
          </span>
          <span className="num mut" style={{ width: 92, textAlign: 'right' }}>
            {it.meta}
          </span>
        </div>
      ))}
    </div>
  )
}

/* -----------------------------------------------------------------------------
   Bar chart — monthly returns etc. Uses the DIVERGING ramp, never categorical.
   -------------------------------------------------------------------------- */
export function BarChart({
  bars,
  height = 160,
  format = (v: number) => signedPct(v),
  className,
  ariaLabel,
}: {
  bars: { key: string; label: string; value: number }[]
  height?: number
  format?: (v: number) => string
  className?: string
  ariaLabel?: string
}) {
  const max = Math.max(1, ...bars.map((b) => Math.abs(b.value)))
  return (
    <div className={cn('row', className)} style={{ alignItems: 'flex-end', gap: 4, height, width: '100%' }} role="img" aria-label={ariaLabel}>
      {bars.map((b) => {
        const h = (Math.abs(b.value) / max) * (height - 28)
        const up = b.value >= 0
        return (
          <div key={b.key} className="col-flex" style={{ flex: 1, alignItems: 'center', gap: 4, minWidth: 0 }} title={`${b.label}: ${format(b.value)}`}>
            <div style={{ flex: 1, display: 'flex', alignItems: 'flex-end', width: '100%' }}>
              <div
                style={{
                  width: '100%',
                  height: Math.max(2, h),
                  background: up ? 'var(--chart-div-p2)' : 'var(--chart-div-n2)',
                  borderRadius: 'var(--r-xs)',
                }}
              />
            </div>
            <span className="lbl truncate-1" style={{ fontSize: 9, maxWidth: '100%' }}>
              {b.label}
            </span>
          </div>
        )
      })}
    </div>
  )
}
