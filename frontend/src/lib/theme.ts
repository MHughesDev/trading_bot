/* =============================================================================
   theme.ts — spec §8.3
   Theme and density resolution + persistence.

   The contract: ONE attribute on <html> selects the theme, one selects density.
   No JS colour objects, no per-component branching, no re-render required.
   ============================================================================= */

export type Theme = 'paper' | 'terminal'
export type Density = 'comfortable' | 'compact'

export const THEME_KEY = 'meridian-theme'
export const DENSITY_KEY = 'meridian-density'
/** Pre-Meridian key. Read once so existing users keep their light/dark choice. */
const LEGACY_THEME_KEY = 'tb-theme'

export const THEME_LABEL: Record<Theme, string> = {
  paper: 'Paper',
  terminal: 'Terminal',
}

export const THEME_DESCRIPTION: Record<Theme, string> = {
  paper: 'Light. Warm ivory, serif display, generous. For planning and reading.',
  terminal: 'Dark. Blue-black, cyan, monospaced numerals. For execution.',
}

/** Density each theme opens with the first time it is used (spec §2.3.4). */
export const DEFAULT_DENSITY: Record<Theme, Density> = {
  paper: 'comfortable',
  terminal: 'compact',
}

function isTheme(v: unknown): v is Theme {
  return v === 'paper' || v === 'terminal'
}
function isDensity(v: unknown): v is Density {
  return v === 'comfortable' || v === 'compact'
}

export function resolveTheme(): Theme {
  try {
    const stored = localStorage.getItem(THEME_KEY)
    if (isTheme(stored)) return stored
    const legacy = localStorage.getItem(LEGACY_THEME_KEY)
    if (legacy === 'light') return 'paper'
    if (legacy === 'dark') return 'terminal'
  } catch {
    /* private mode / blocked storage — fall through to the system preference */
  }
  if (typeof window !== 'undefined' && window.matchMedia?.('(prefers-color-scheme: light)').matches) {
    return 'paper'
  }
  return 'terminal'
}

export function resolveDensity(theme: Theme): Density {
  try {
    const stored = localStorage.getItem(DENSITY_KEY)
    if (isDensity(stored)) return stored
  } catch {
    /* ignore */
  }
  return DEFAULT_DENSITY[theme]
}

/**
 * Apply theme + density to <html>. Paint only — nothing here causes a reflow.
 * `animate` briefly enables a colour transition so a deliberate toggle reads as
 * a change rather than a flicker; boot never animates.
 */
export function applyTheme(theme: Theme, density: Density, animate = false): void {
  const el = document.documentElement
  if (animate) {
    el.setAttribute('data-theme-switching', '')
    window.setTimeout(() => el.removeAttribute('data-theme-switching'), 200)
  }
  el.setAttribute('data-theme', theme)
  el.setAttribute('data-density', density)
  // Keep the legacy hook alive for any stylesheet still scoped to `.dark`.
  el.classList.toggle('dark', theme === 'terminal')
}

export function persistTheme(theme: Theme): void {
  try {
    localStorage.setItem(THEME_KEY, theme)
    localStorage.setItem(LEGACY_THEME_KEY, theme === 'terminal' ? 'dark' : 'light')
  } catch {
    /* ignore */
  }
}

export function persistDensity(density: Density): void {
  try {
    localStorage.setItem(DENSITY_KEY, density)
  } catch {
    /* ignore */
  }
}

/* -----------------------------------------------------------------------------
   Reading resolved token values (for canvas-based chart libraries that cannot
   consume CSS variables). Always read at paint time, never cache across themes.
   -------------------------------------------------------------------------- */

export function cssVar(name: string, fallback = ''): string {
  if (typeof window === 'undefined') return fallback
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim()
  return v || fallback
}

/** Fired after every theme/density change so canvas charts can repaint. */
export const THEME_CHANGE_EVENT = 'meridian:themechange'

export function emitThemeChange(theme: Theme, density: Density): void {
  window.dispatchEvent(new CustomEvent(THEME_CHANGE_EVENT, { detail: { theme, density } }))
}
