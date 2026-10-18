import type { UsedByRef } from './studio.helpers'

/**
 * A trend line with no axes: drawn from observed values only. Fewer than two
 * points is not a trend, so it renders a labelled dash instead of a flat line
 * that would read as "unchanged".
 */
export function Sparkline({
  points,
  label,
  width = 120,
  height = 28,
}: {
  points: number[]
  label: string
  width?: number
  height?: number
}) {
  if (points.length < 2) {
    return (
      <span className="mut" style={{ fontSize: 'var(--t-11)' }} aria-label={`${label}: no history yet`}>
        no history yet
      </span>
    )
  }
  const min = Math.min(...points)
  const max = Math.max(...points)
  const span = max - min || 1
  const pad = 2
  const step = (width - pad * 2) / (points.length - 1)
  const coords = points.map((p, i) => [pad + i * step, pad + (1 - (p - min) / span) * (height - pad * 2)])
  const d = coords.map(([x, y], i) => `${i === 0 ? 'M' : 'L'}${x.toFixed(1)} ${y.toFixed(1)}`).join(' ')
  const [lx, ly] = coords[coords.length - 1]
  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={`${label} across ${points.length} versions`}
    >
      <path d={d} fill="none" stroke="var(--bg-accent)" strokeWidth={1.5} strokeLinejoin="round" />
      <circle cx={lx} cy={ly} r={2.5} fill="var(--bg-accent)" />
    </svg>
  )
}

/** Strategy chips. `null` = the used-by read has not answered; `[]` = answered, none. */
export function UsedByChips({ usedBy, max = 3 }: { usedBy: UsedByRef[] | null; max?: number }) {
  if (usedBy === null) {
    return <span className="mut" style={{ fontSize: 'var(--t-11)' }}>used by: …</span>
  }
  if (usedBy.length === 0) {
    return <span className="mut" style={{ fontSize: 'var(--t-11)' }}>not used by any strategy</span>
  }
  const shown = usedBy.slice(0, max)
  const extra = usedBy.length - shown.length
  return (
    <span style={{ display: 'inline-flex', gap: 4, flexWrap: 'wrap', alignItems: 'center' }}>
      <span className="mut" style={{ fontSize: 'var(--t-11)' }}>used by</span>
      {shown.map((s) => (
        <span key={s.id} className="chip">
          {s.name}
        </span>
      ))}
      {extra > 0 && <span className="chip">+{extra}</span>}
    </span>
  )
}
