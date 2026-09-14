import { create } from 'zustand'
import {
  applyTheme,
  emitThemeChange,
  persistDensity,
  persistTheme,
  resolveDensity,
  resolveTheme,
  type Density,
  type Theme,
} from '@/lib/theme'

export type { Theme, Density }

interface ThemeState {
  theme: Theme
  density: Density
  setTheme: (theme: Theme) => void
  toggleTheme: () => void
  setDensity: (density: Density) => void
  toggleDensity: () => void
}

const initialTheme = resolveTheme()
const initialDensity = resolveDensity(initialTheme)

export const useThemeStore = create<ThemeState>((set, get) => ({
  theme: initialTheme,
  density: initialDensity,

  setTheme: (theme) => {
    if (theme === get().theme) return
    persistTheme(theme)
    applyTheme(theme, get().density, true)
    set({ theme })
    emitThemeChange(theme, get().density)
  },

  toggleTheme: () => get().setTheme(get().theme === 'terminal' ? 'paper' : 'terminal'),

  setDensity: (density) => {
    if (density === get().density) return
    persistDensity(density)
    applyTheme(get().theme, density, false)
    set({ density })
    emitThemeChange(get().theme, density)
  },

  toggleDensity: () =>
    get().setDensity(get().density === 'compact' ? 'comfortable' : 'compact'),
}))

// Boot: the inline script in index.html already painted the right attributes;
// this re-asserts them so a stale SSR/HMR document can never drift.
applyTheme(initialTheme, initialDensity, false)
