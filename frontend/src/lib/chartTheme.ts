/* =============================================================================
   chartTheme.ts — spec §2.8.1
   Canvas-based chart libraries cannot read CSS variables, so this module
   resolves the --chart-* tokens at paint time and hands them over as values.

   Rules:
     - This is the ONLY bridge between the token layer and a chart library.
     - Always call at paint time, never cache across a theme change.
     - No colour value is ever written here. Every entry resolves a token.
   ============================================================================= */

import { cssVar } from './theme'

export interface ChartPalette {
  surface: string
  grid: string
  gridMajor: string
  axis: string
  crosshair: string
  up: string
  upLine: string
  down: string
  downLine: string
  volume: string
  last: string
  tagBg: string
  tagFg: string
  maFast: string
  maSlow: string
  equity: string
  areaFrom: string
  areaTo: string
  /** Fixed categorical Instrument scale. Slot order NEVER changes (V7). */
  series: string[]
  /** Sequential (magnitude) ramp — one hue, light to dark. */
  sequential: string[]
  /** Diverging (polarity) ramp — two hues with a neutral midpoint. */
  diverging: string[]
  /** Text tokens — chart text NEVER wears a series colour (§2.8.4). */
  fg: string
  fgSecondary: string
  fgTertiary: string
  line: string
  hairline: string
  accent: string
  pos: string
  neg: string
}

export function chartPalette(): ChartPalette {
  return {
    surface: cssVar('--chart-surface'),
    grid: cssVar('--chart-grid'),
    gridMajor: cssVar('--chart-grid-major'),
    axis: cssVar('--chart-axis'),
    crosshair: cssVar('--chart-crosshair'),
    up: cssVar('--chart-up'),
    upLine: cssVar('--chart-up-line'),
    down: cssVar('--chart-down'),
    downLine: cssVar('--chart-down-line'),
    volume: cssVar('--chart-volume'),
    last: cssVar('--chart-last'),
    tagBg: cssVar('--chart-tag-bg'),
    tagFg: cssVar('--chart-tag-fg'),
    maFast: cssVar('--chart-ma-fast'),
    maSlow: cssVar('--chart-ma-slow'),
    equity: cssVar('--chart-equity'),
    areaFrom: cssVar('--chart-area-from'),
    areaTo: cssVar('--chart-area-to'),
    series: [1, 2, 3, 4, 5, 6, 7, 8].map((i) => cssVar(`--chart-${i}`)),
    sequential: [1, 2, 3, 4, 5].map((i) => cssVar(`--chart-seq-${i}`)),
    diverging: ['n2', 'n1', '0', 'p1', 'p2'].map((k) => cssVar(`--chart-div-${k}`)),
    fg: cssVar('--fg-primary'),
    fgSecondary: cssVar('--fg-secondary'),
    fgTertiary: cssVar('--fg-tertiary'),
    line: cssVar('--line-default'),
    hairline: cssVar('--line-hairline'),
    accent: cssVar('--bg-accent'),
    pos: cssVar('--fg-pos'),
    neg: cssVar('--fg-neg'),
  }
}

/**
 * Back-compat shape for pre-Meridian chart call sites.
 * @deprecated consume `chartPalette()` instead.
 */
export function chartColors() {
  const p = chartPalette()
  return {
    text: p.fgSecondary,
    grid: p.grid,
    border: p.hairline,
    accent: p.accent,
    pnlUp: p.up,
    pnlDown: p.down,
  }
}

/**
 * Interpolate a value in [-1, 1] onto the diverging ramp — used by monthly
 * returns heat maps (§4.4). Never use the categorical scale for magnitude.
 */
export function divergingColor(t: number, palette = chartPalette()): string {
  const clamped = Math.max(-1, Math.min(1, t))
  const [n2, n1, mid, p1, p2] = palette.diverging
  if (clamped <= -0.66) return n2
  if (clamped <= -0.22) return n1
  if (clamped < 0.22) return mid
  if (clamped < 0.66) return p1
  return p2
}

/** Magnitude in [0, 1] onto the sequential ramp. */
export function sequentialColor(t: number, palette = chartPalette()): string {
  const clamped = Math.max(0, Math.min(1, t))
  const ramp = palette.sequential
  const i = Math.min(ramp.length - 1, Math.floor(clamped * ramp.length))
  return ramp[i]
}
