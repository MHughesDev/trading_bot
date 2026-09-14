import { useEffect } from 'react'
import { createPortal } from 'react-dom'
import { AlertTriangle, CheckCircle2, Info, X, XCircle } from 'lucide-react'
import { registerToast, useToastState, type ToastVariant } from '@/hooks/useToast'
import { IconButton } from '@/components/primitives/Button'
import { cn } from '@/lib/utils'

const ICON: Record<ToastVariant, typeof Info> = {
  default: Info,
  success: CheckCircle2,
  error: XCircle,
  warning: AlertTriangle,
}

const TONE: Record<ToastVariant, string> = {
  default: 'info',
  success: 'pos',
  error: 'neg',
  warning: 'warn',
}

export function ToastContainer() {
  const { toasts, addToast, dismissToast } = useToastState()

  useEffect(() => {
    registerToast(addToast)
  }, [addToast])

  if (typeof document === 'undefined') return null

  return createPortal(
    <div className="toastwrap" role="region" aria-label="Notifications">
      {toasts.map((t) => {
        const variant = t.variant ?? 'default'
        const Icon = ICON[variant]
        return (
          <div
            key={t.id}
            className={cn('toast', TONE[variant])}
            role="status"
            aria-live={variant === 'error' ? 'assertive' : 'polite'}
          >
            <span className="ico">
              <Icon size={14} aria-hidden />
            </span>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontWeight: 'var(--w-semibold)' }}>{t.title}</div>
              {t.description && (
                <div className="sec" style={{ marginTop: 2, lineHeight: 1.45 }}>
                  {t.description}
                </div>
              )}
            </div>
            <IconButton label="Dismiss" bare onClick={() => dismissToast(t.id)}>
              <X size={13} aria-hidden />
            </IconButton>
          </div>
        )
      })}
    </div>,
    document.body,
  )
}
