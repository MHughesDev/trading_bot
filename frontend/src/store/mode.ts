import { create } from 'zustand'
import { persist } from 'zustand/middleware'

export type TradingMode = 'PAPER' | 'LIVE'

interface ModeState {
  mode: TradingMode
  setMode: (mode: TradingMode) => void
  toggleMode: () => void
}

export const useModeStore = create<ModeState>()(
  persist(
    (set, get) => ({
      mode: 'PAPER',
      setMode: (mode) => set({ mode }),
      toggleMode: () => set({ mode: get().mode === 'PAPER' ? 'LIVE' : 'PAPER' }),
    }),
    { name: 'tb-trading-mode' },
  ),
)
