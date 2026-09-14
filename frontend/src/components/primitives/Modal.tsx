import { useEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { AlertTriangle, X } from 'lucide-react'
import { cn } from '@/lib/utils'
import { Button } from './Button'
import { IconButton } from './Button'
import { Input } from './Field'

/* Spec §3.18 — modal. Focus trapped, Esc closes UNLESS the modal is a
   destructive confirm (§5.5), footer actions right-aligned with primary last. */

function useFocusTrap(open: boolean, ref: React.RefObject<HTMLElement | null>, onEsc?: () => void) {
  useEffect(() => {
    if (!open) return
    const root = ref.current
    const previouslyFocused = document.activeElement as HTMLElement | null

    const focusables = () =>
      Array.from(
        root?.querySelectorAll<HTMLElement>(
          'button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])',
        ) ?? [],
      ).filter((el) => el.offsetParent !== null)

    window.setTimeout(() => focusables()[0]?.focus(), 0)

    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape' && onEsc) {
        e.preventDefault()
        onEsc()
        return
      }
      if (e.key !== 'Tab') return
      const els = focusables()
      if (!els.length) return
      const first = els[0]
      const last = els[els.length - 1]
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault()
        last.focus()
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault()
        first.focus()
      }
    }

    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('keydown', onKey)
      previouslyFocused?.focus?.()
    }
  }, [open, ref, onEsc])
}

export interface ModalProps {
  open: boolean
  onClose: () => void
  title: ReactNode
  children: ReactNode
  footer?: ReactNode
  /** Destructive confirms MUST NOT be dismissible with Esc or a scrim click. */
  dismissible?: boolean
  size?: 'default' | 'wide' | 'xwide'
  description?: ReactNode
  headerAside?: ReactNode
}

export function Modal({
  open,
  onClose,
  title,
  children,
  footer,
  dismissible = true,
  size = 'default',
  description,
  headerAside,
}: ModalProps) {
  const ref = useRef<HTMLDivElement>(null)
  useFocusTrap(open, ref, dismissible ? onClose : undefined)

  if (!open) return null

  return createPortal(
    <div
      className="scrim"
      onMouseDown={(e) => {
        if (dismissible && e.target === e.currentTarget) onClose()
      }}
    >
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-label={typeof title === 'string' ? title : undefined}
        className={cn('modal', size === 'wide' && 'wide', size === 'xwide' && 'xwide')}
      >
        <div className="modal-hd">
          <div style={{ minWidth: 0 }}>
            <div className="h2 truncate-1">{title}</div>
            {description && (
              <div className="mut" style={{ fontSize: 'var(--t-12)', marginTop: 3 }}>
                {description}
              </div>
            )}
          </div>
          {headerAside}
          <span className="spacer" />
          {dismissible && (
            <IconButton label="Close" bare onClick={onClose}>
              <X size={15} aria-hidden />
            </IconButton>
          )}
        </div>
        <div className="modal-bd">{children}</div>
        {footer && <div className="modal-ft">{footer}</div>}
      </div>
    </div>,
    document.body,
  )
}

/* Spec §5.5 — destructive and irreversible actions.
   - names the exact consequence WITH NUMBERS
   - the highest-severity actions require typing a confirmation word
   - Esc MUST NOT dismiss; cancel must be clicked
   - always offer the reversible alternative where one exists */
export function ConfirmDialog({
  open,
  onCancel,
  onConfirm,
  title,
  /** Must state the consequence with numbers. */
  consequence,
  confirmLabel,
  /** Typing this word is required before confirm enables. */
  typeToConfirm,
  /** The reversible option, e.g. "Pause instead". */
  alternative,
  busy,
}: {
  open: boolean
  onCancel: () => void
  onConfirm: () => void
  title: ReactNode
  consequence: ReactNode
  confirmLabel: string
  typeToConfirm?: string
  alternative?: ReactNode
  busy?: boolean
}) {
  const [typed, setTyped] = useState('')

  useEffect(() => {
    if (!open) setTyped('')
  }, [open])

  const ready = !typeToConfirm || typed.trim().toUpperCase() === typeToConfirm.toUpperCase()

  return (
    <Modal
      open={open}
      onClose={onCancel}
      dismissible={false}
      title={title}
      footer={
        <>
          {alternative}
          <span className="spacer" />
          <Button onClick={onCancel}>Cancel</Button>
          <Button variant="danger" disabled={!ready} loading={busy} onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </>
      }
    >
      <div className="callout neg" style={{ marginBottom: 'var(--s-4)' }}>
        <AlertTriangle size={15} aria-hidden style={{ flex: 'none', marginTop: 1 }} />
        <div>{consequence}</div>
      </div>
      {typeToConfirm && (
        <Input
          label={
            <>
              Type <strong>{typeToConfirm}</strong> to confirm
            </>
          }
          value={typed}
          autoFocus
          autoComplete="off"
          spellCheck={false}
          onChange={(e) => setTyped(e.target.value)}
          placeholder={typeToConfirm}
        />
      )}
    </Modal>
  )
}
