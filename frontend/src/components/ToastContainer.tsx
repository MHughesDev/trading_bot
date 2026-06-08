import { useEffect } from 'react'
import {
  ToastProvider, ToastViewport, Toast, ToastTitle, ToastDescription, ToastClose
} from '@/components/ui/toast'
import { useToastState, registerToast } from '@/hooks/useToast'

export function ToastContainer() {
  const { toasts, addToast, dismissToast } = useToastState()

  useEffect(() => {
    registerToast(addToast)
  }, [addToast])

  return (
    <ToastProvider>
      {toasts.map((t) => (
        <Toast key={t.id} open={t.open} onOpenChange={() => dismissToast(t.id)} variant={t.variant}>
          <div className="flex-1">
            <ToastTitle>{t.title}</ToastTitle>
            {t.description && <ToastDescription>{t.description}</ToastDescription>}
          </div>
          <ToastClose />
        </Toast>
      ))}
      <ToastViewport />
    </ToastProvider>
  )
}
