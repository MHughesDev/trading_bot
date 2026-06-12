// Trading mode (PAPER | LIVE) with cross-tab cascade and per-window pinning.
//
// Default behaviour: the mode is shared via localStorage, so toggling it in
// one tab cascades to every other open tab of the app (via the `storage`
// event).  A tab opened with `?mode=paper` or `?mode=live` is *pinned*: it
// keeps its own mode in sessionStorage, ignores the cascade, and never
// broadcasts — letting you run a PAPER window and a LIVE window side by side
// on different monitors.

import { create } from 'zustand'

export type TradingMode = 'PAPER' | 'LIVE'

const STORAGE_KEY = 'tb-trading-mode'
const PIN_KEY = 'tb-trading-mode-pin'

function parseMode(value: string | null | undefined): TradingMode | null {
  const up = value?.toUpperCase()
  return up === 'PAPER' || up === 'LIVE' ? up : null
}

/** Pin for this tab: `?mode=` in the URL wins, then sessionStorage (so the
 *  pin survives in-tab navigation and reloads). */
function initialPin(): TradingMode | null {
  if (typeof window === 'undefined') return null
  const fromUrl = parseMode(new URLSearchParams(window.location.search).get('mode'))
  if (fromUrl) {
    sessionStorage.setItem(PIN_KEY, fromUrl)
    return fromUrl
  }
  return parseMode(sessionStorage.getItem(PIN_KEY))
}

/** Shared (cascading) mode from localStorage.  Tolerates the legacy
 *  zustand-persist JSON shape `{"state":{"mode":...}}`. */
function sharedMode(): TradingMode {
  if (typeof window === 'undefined') return 'PAPER'
  const raw = localStorage.getItem(STORAGE_KEY)
  if (!raw) return 'PAPER'
  const plain = parseMode(raw)
  if (plain) return plain
  try {
    const legacy = JSON.parse(raw) as { state?: { mode?: string } }
    return parseMode(legacy.state?.mode) ?? 'PAPER'
  } catch {
    return 'PAPER'
  }
}

interface ModeState {
  mode: TradingMode
  /** True when this tab was opened with ?mode= and ignores the cascade. */
  pinned: boolean
  setMode: (mode: TradingMode) => void
  toggleMode: () => void
}

const pin = initialPin()

export const useModeStore = create<ModeState>()((set, get) => ({
  mode: pin ?? sharedMode(),
  pinned: pin !== null,
  setMode: (mode) => {
    if (get().pinned) {
      // Pinned tabs change only themselves.
      sessionStorage.setItem(PIN_KEY, mode)
    } else {
      // Writing localStorage fires `storage` in every other tab → cascade.
      localStorage.setItem(STORAGE_KEY, mode)
    }
    set({ mode })
  },
  toggleMode: () => {
    const { mode, setMode } = get()
    setMode(mode === 'PAPER' ? 'LIVE' : 'PAPER')
  },
}))

// Follow the cascade from other tabs (unpinned tabs only).
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (e) => {
    if (e.key !== STORAGE_KEY) return
    const state = useModeStore.getState()
    if (state.pinned) return
    const next = parseMode(e.newValue)
    if (next && next !== state.mode) useModeStore.setState({ mode: next })
  })
}
