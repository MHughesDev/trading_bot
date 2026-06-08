import { create } from 'zustand'
import { authApi, type User } from '@/lib/api'

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
      await authApi.login(email, password)
      const { data } = await authApi.me()
      set({ user: data, loading: false })
    } catch (err) {
      set({ loading: false })
      throw err
    }
  },

  logout: async () => {
    await authApi.logout()
    set({ user: null })
    window.location.href = '/login'
  },
}))
