import { create } from 'zustand'
import { persist } from 'zustand/middleware'

interface WatchlistState {
  symbols: string[]
  add: (symbol: string) => void
  remove: (symbol: string) => void
  toggle: (symbol: string) => void
  has: (symbol: string) => boolean
}

export const useWatchlistStore = create<WatchlistState>()(
  persist(
    (set, get) => ({
      symbols: [],
      add: (symbol) =>
        set((s) => ({ symbols: [...new Set([...s.symbols, symbol])] })),
      remove: (symbol) =>
        set((s) => ({ symbols: s.symbols.filter((x) => x !== symbol) })),
      toggle: (symbol) => {
        if (get().has(symbol)) get().remove(symbol)
        else get().add(symbol)
      },
      has: (symbol) => get().symbols.includes(symbol),
    }),
    { name: 'tb-watchlist' }
  )
)
