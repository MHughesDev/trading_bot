import { useCallback, useState } from 'react'

/* Toasts are for events that happen OUTSIDE the panel the user is looking at —
   an order fill, a background job finishing. A panel-scoped failure gets an
   inline error state instead (spec §3.22), never a toast. */

export type ToastVariant = 'default' | 'success' | 'error' | 'warning'

export interface ToastItem {
  id: string
  title: string
  description?: string
  variant?: ToastVariant
  open: boolean
}

export type ToastInput = Omit<ToastItem, 'id' | 'open'>

let globalToast: ((t: ToastInput) => void) | null = null

export function registerToast(fn: (t: ToastInput) => void) {
  globalToast = fn
}

export function toast(t: ToastInput) {
  globalToast?.(t)
}

/** Hook form, so components can call `const toast = useToast()`. */
export function useToast() {
  return useCallback((t: ToastInput) => toast(t), [])
}

export function useToastState() {
  const [toasts, setToasts] = useState<ToastItem[]>([])

  const addToast = useCallback((t: ToastInput) => {
    const id = Math.random().toString(36).slice(2)
    setToasts((prev) => [...prev, { ...t, id, open: true }])
    setTimeout(() => {
      setToasts((prev) => prev.filter((x) => x.id !== id))
    }, 5000)
  }, [])

  const dismissToast = useCallback((id: string) => {
    setToasts((prev) => prev.filter((x) => x.id !== id))
  }, [])

  return { toasts, addToast, dismissToast }
}
