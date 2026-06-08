import { useState, useCallback } from 'react'

interface ToastItem {
  id: string
  title: string
  description?: string
  variant?: 'default' | 'success' | 'error'
  open: boolean
}

let globalToast: ((t: Omit<ToastItem, 'id' | 'open'>) => void) | null = null

export function registerToast(fn: (t: Omit<ToastItem, 'id' | 'open'>) => void) {
  globalToast = fn
}

export function toast(t: Omit<ToastItem, 'id' | 'open'>) {
  globalToast?.(t)
}

export function useToastState() {
  const [toasts, setToasts] = useState<ToastItem[]>([])

  const addToast = useCallback((t: Omit<ToastItem, 'id' | 'open'>) => {
    const id = Math.random().toString(36).slice(2)
    setToasts((prev) => [...prev, { ...t, id, open: true }])
    setTimeout(() => {
      setToasts((prev) => prev.filter((x) => x.id !== id))
    }, 4000)
  }, [])

  const dismissToast = useCallback((id: string) => {
    setToasts((prev) => prev.filter((x) => x.id !== id))
  }, [])

  return { toasts, addToast, dismissToast }
}
