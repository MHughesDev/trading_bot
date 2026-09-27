import type { ReactNode } from 'react'
import {
  AlertTriangle,
  Database,
} from 'lucide-react'
import { cn } from '@/lib/utils'
import { FAMILIES } from './families'

/* =============================================================================
   Spec §2.1.8 / §3.20 — the strategy node taxonomy.

   Seven families. Colour encodes WHAT KIND OF THING this block is, which is the
   only property worth colour-coding on a canvas. The family colour appears as a
   3px left rail + icon + uppercase family label. It MUST NOT flood the header:
   twenty saturated headers on one canvas is why the old canvas was unreadable.

   `logic` is deliberately neutral grey — gates are plumbing, not meaning.
   ============================================================================= */

export type NodeFamily = 'data' | 'indicator' | 'signal' | 'logic' | 'ai' | 'intent' | 'risk'

export interface FamilyInfo {
  family: NodeFamily
  label: string
  token: string
  icon: typeof Database
  /** What this family is for, shown in the palette and the inspector. */
  blurb: string
  /** Left-to-right stage order for auto-layout (spec §3.21). */
  stage: number
}

/* -----------------------------------------------------------------------------
   The node shell: header rail + icon + family label, title, summary fields.
   Nodes stay SMALL; the inspector holds the full form (spec §4.3).
   -------------------------------------------------------------------------- */

export function NodeShell({
  family,
  title,
  selected,
  invalid,
  disabled,
  warning,
  children,
  familyLabel,
}: {
  family: NodeFamily
  title: ReactNode
  selected?: boolean
  invalid?: boolean
  disabled?: boolean
  warning?: string
  children?: ReactNode
  familyLabel?: string
}) {
  const info = FAMILIES[family]
  const Icon = info.icon
  return (
    <div
      className={cn('node', selected && 'sel', invalid && 'invalid', disabled && 'muted')}
      style={{ ['--fam' as string]: info.token }}
    >
      <div className="hd">
        <span className="ico">
          <Icon size={12} aria-hidden />
        </span>
        <span className="fam">{familyLabel ?? info.label}</span>
        {(invalid || warning) && (
          <span style={{ marginLeft: 'auto', color: 'var(--fg-warn)', display: 'flex' }} title={warning}>
            <AlertTriangle size={11} aria-hidden />
          </span>
        )}
      </div>
      <div className="ttl truncate-1">{title}</div>
      {children && <div className="fields">{children}</div>}
    </div>
  )
}

/** One `label · value` row inside a node. Values are summaries, not editors. */
export function NodeField({ k, v }: { k: ReactNode; v: ReactNode }) {
  return (
    <div className="nf">
      <span className="k">{k}</span>
      <span className="v" title={typeof v === 'string' ? v : undefined}>
        {v}
      </span>
    </div>
  )
}
