import { useEffect, useState } from 'react'
import { AlertTriangle, Check, ChevronDown, ChevronUp, Info } from 'lucide-react'
import { IconButton } from '@/components/primitives/Button'
import { Badge } from '@/components/primitives/Badge'
import { Label } from '@/components/primitives/Num'
import { clockTime } from '@/lib/format'
import { cn } from '@/lib/utils'

/* Spec §4.3 — the "Tips" box that occupied prime canvas real estate is replaced
   by a floating VALIDATION CONSOLE docked bottom-right, showing typed messages
   (ok / warning / info) with timestamps. It is collapsible and remembers its
   state. Validation is continuous, not on-demand. */

export type ValidationLevel = 'ok' | 'warn' | 'error' | 'info'

export interface ValidationMessage {
  id: string
  level: ValidationLevel
  text: string
  at: number
}

const COLLAPSE_KEY = 'meridian-validation-collapsed'

const ICON = { ok: Check, warn: AlertTriangle, error: AlertTriangle, info: Info }
const TONE: Record<ValidationLevel, string> = { ok: 'pos', warn: 'warn', error: 'neg', info: 'info' }

export function ValidationConsole({
  messages,
  lastRun,
  className,
}: {
  messages: ValidationMessage[]
  lastRun: number | null
  className?: string
}) {
  const [collapsed, setCollapsed] = useState(() => {
    try {
      return localStorage.getItem(COLLAPSE_KEY) === '1'
    } catch {
      return false
    }
  })

  useEffect(() => {
    try {
      localStorage.setItem(COLLAPSE_KEY, collapsed ? '1' : '0')
    } catch {
      /* ignore */
    }
  }, [collapsed])

  const errors = messages.filter((m) => m.level === 'error').length
  const warns = messages.filter((m) => m.level === 'warn').length

  return (
    <div className={cn('console', className)}>
      <div className="hd">
        <Label>Validation</Label>
        {errors > 0 && <Badge tone="neg">{errors} error{errors === 1 ? '' : 's'}</Badge>}
        {warns > 0 && <Badge tone="warn">{warns} warning{warns === 1 ? '' : 's'}</Badge>}
        {errors === 0 && warns === 0 && <Badge tone="pos">clean</Badge>}
        <span className="spacer" />
        <Label>{lastRun ? `Last run ${clockTime(lastRun)}` : 'Not yet run'}</Label>
        <IconButton
          label={collapsed ? 'Expand validation console' : 'Collapse validation console'}
          bare
          onClick={() => setCollapsed((c) => !c)}
        >
          {collapsed ? <ChevronUp size={13} aria-hidden /> : <ChevronDown size={13} aria-hidden />}
        </IconButton>
      </div>

      {!collapsed && (
        <div style={{ overflow: 'auto', maxHeight: 200 }} aria-live="polite">
          {messages.length === 0 ? (
            <div className="cmsg">
              <span className="t">{clockTime(Date.now())}</span>
              <Check size={12} className="i pos" aria-hidden />
              <span>Nothing to report. Add blocks and wire them up.</span>
            </div>
          ) : (
            messages.map((m) => {
              const Icon = ICON[m.level]
              return (
                <div key={m.id} className="cmsg">
                  <span className="t">{clockTime(m.at)}</span>
                  <Icon size={12} className={cn('i', TONE[m.level])} aria-hidden />
                  <span>{m.text}</span>
                </div>
              )
            })
          )}
        </div>
      )}
    </div>
  )
}
