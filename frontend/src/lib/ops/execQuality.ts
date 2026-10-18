// Execution quality, as the page words it (FB-RX-003).
//
// A statistic is `measured` or `unmeasured` with a reason. The one thing this
// file refuses to do is turn the second into the first: an unmeasured value
// prints as the words "not measured" and carries its reason to a tooltip, never
// as `0`, `0 bps` or an empty cell.
import type { ExecQualityRow, Measured } from '@/api/ops'

export function measuredText(m: Measured<number>, fmt: (v: number) => string): string {
  return m.status === 'measured' ? fmt(m.value) : 'not measured'
}

export function reasonOf(m: Measured<number>): string | undefined {
  return m.status === 'unmeasured' ? m.reason : undefined
}

export const fmtBps = (v: number): string => `${v.toFixed(1)} bps`
export const fmtRatio = (v: number): string => `${(v * 100).toFixed(1)}%`
export const fmtMs = (v: number): string => (v >= 1000 ? `${(v / 1000).toFixed(2)} s` : `${Math.round(v)} ms`)

/** Names a statistic no row has: the page says so once instead of per cell. */
export function unproducedStatistics(rows: ExecQualityRow[]): string[] {
  const out: string[] = []
  if (rows.length > 0 && rows.every((r) => r.slippage_bps_p50.status === 'unmeasured')) {
    out.push('slippage versus the fill model')
  }
  return out
}

/** The first reason any row gives for an unmeasured slippage, to print once. */
export function slippageReason(rows: ExecQualityRow[]): string | null {
  for (const r of rows) {
    if (r.slippage_bps_p50.status === 'unmeasured') return r.slippage_bps_p50.reason
  }
  return null
}

/** Total fills across the rows: the figure the empty state keys off. */
export function totalFills(rows: ExecQualityRow[]): number {
  return rows.reduce((n, r) => n + r.fill_count, 0)
}
