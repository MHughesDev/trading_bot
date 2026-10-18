// The coverage census, in words (Set P P-0.2).
//
// The census is the thing to look at before believing any claim about what the
// platform can evaluate, so this file's job is to keep the page from flattering
// it. Three rules:
//
//   * **Depth is stated in words**, from the *continuous* run, not from the
//     span between first and last bar: a series with a three-year hole is not
//     five years deep.
//   * **Thinnest first.** The server sorts that way; the page re-sorts
//     defensively on the same key, so a reordered response cannot bury the
//     series that decides what can be evaluated.
//   * **An age is only an age of a row that exists.** A forward series with no
//     last-row age says it was not measured.
import type { CoverageSeries, DataCoverage, ForwardSeries } from '@/api/ops'
import { ageWords } from './monitor'

export function thinnestFirst(rows: CoverageSeries[]): CoverageSeries[] {
  return [...rows].sort(
    (a, b) =>
      a.continuous_days - b.continuous_days ||
      a.instrument.localeCompare(b.instrument) ||
      a.venue.localeCompare(b.venue) ||
      a.timeframe.localeCompare(b.timeframe),
  )
}

/** "no rows", "41 days", "3 years 2 months" — the longest continuous run. */
export function depthWords(s: Pick<CoverageSeries, 'continuous_days' | 'present_bars'>): string {
  if (s.present_bars === 0) return 'no rows'
  const d = s.continuous_days
  if (d < 1) return 'under a day of continuous history'
  if (d < 90) return `${d} ${d === 1 ? 'day' : 'days'} of continuous history`
  if (d < 730) return `${Math.round(d / 30)} months of continuous history`
  const years = Math.floor(d / 365)
  const months = Math.round((d - years * 365) / 30)
  return `${years} years${months > 0 ? ` ${months} months` : ''} of continuous history`
}

export type DepthStanding = 'strict' | 'paper' | 'short' | 'empty'

/** Which floors the series clears. `strict_v1` implies `paper_v1`. */
export function depthStanding(s: CoverageSeries): DepthStanding {
  if (s.present_bars === 0) return 'empty'
  if (s.strict_v1.satisfied) return 'strict'
  if (s.paper_v1.satisfied) return 'paper'
  return 'short'
}

export const STANDING_WORDS: Record<DepthStanding, { word: string; tone: 'pos' | 'warn' | 'neg' | 'neutral' }> = {
  strict: { word: 'meets Gate 9', tone: 'pos' },
  paper: { word: 'paper only', tone: 'warn' },
  short: { word: 'below both floors', tone: 'neg' },
  empty: { word: 'empty', tone: 'neg' },
}

/** 0..1 of the strict floor the continuous run covers. */
export function depthFraction(s: Pick<CoverageSeries, 'continuous_days'>, strictFloorDays: number): number {
  if (!Number.isFinite(strictFloorDays) || strictFloorDays <= 0) return 0
  return Math.max(0, Math.min(1, s.continuous_days / strictFloorDays))
}

/** The last row's age; a row that was never measured has no age to print. */
export function lastRowWords(secs: number | null | undefined): string {
  if (secs === null || secs === undefined) return 'not measured'
  return `${ageWords(Math.max(0, secs))} ago`
}

/** Forward series, stalest first: the one that stopped is the one to see. */
export function stalestFirst(rows: ForwardSeries[]): ForwardSeries[] {
  return [...rows].sort((a, b) => b.last_row_age_secs - a.last_row_age_secs)
}

export interface CensusSummary {
  total: number
  strict: number
  paper: number
  short: number
  empty: number
  notAccruing: number
}

export function summarise(d: DataCoverage): CensusSummary {
  const out: CensusSummary = { total: d.series.length, strict: 0, paper: 0, short: 0, empty: 0, notAccruing: 0 }
  for (const s of d.series) {
    out[depthStanding(s)] += 1
    if (!s.accruing) out.notAccruing += 1
  }
  return out
}
