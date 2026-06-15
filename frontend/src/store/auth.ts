import { create } from 'zustand'
import { authApi, clearStoredToken, setStoredToken, type User } from '@/lib/api'
import { initWsClient, destroyWsClient } from '@/api/ws'

interface AuthState {
  user: User | null
  loading: boolean
  initialized: boolean
  login: (email: string, password: string) => Promise<void>
  logout: () => Promise<void>
  fetchMe: () => Promise<void>
}

export const useAuthStore = create<AuthState>((set) => ({
  user: null,
  loading: false,
  initialized: false,

  fetchMe: async () => {
    try {
      const { data } = await authApi.me()
      set({ user: data, initialized: true })
    } catch {
      set({ user: null, initialized: true })
    }
  },

  login: async (email, password) => {
    set({ loading: true })
    try {
      const { data } = await authApi.login(email, password)
      setStoredToken(data.token)
      initWsClient(data.token)
      set({ user: data.user, loading: false })
    } catch (err) {
      set({ loading: false })
      throw err
    }
  },

  logout: async () => {
    try { await authApi.logout() } catch { /* ignore */ }
    clearStoredToken()
    destroyWsClient()
    set({ user: null })
    window.location.href = '/login'
  },
}))
