import type { ReactNode } from 'react'
import {
  Activity,
  AlertTriangle,
  Database,
  GitMerge,
  Shield,
  Sparkles,
  TrendingUp,
  Waves,
} from 'lucide-react'
import { cn } from '@/lib/utils'

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

export const FAMILIES: Record<NodeFamily, FamilyInfo> = {
  data: {
    family: 'data',
    label: 'Market data',
    token: 'var(--node-data)',
    icon: Database,
    blurb: 'Price series, volume, order book, funding.',
    stage: 0,
  },
  indicator: {
    family: 'indicator',
    label: 'Indicator',
    token: 'var(--node-indicator)',
    icon: Activity,
    blurb: 'EMA, SMA, RSI, ATR, MACD, Bollinger.',
    stage: 1,
  },
  signal: {
    family: 'signal',
    label: 'Signal',
    token: 'var(--node-signal)',
    icon: TrendingUp,
    blurb: 'Crosses above/below, greater, less, rising, falling.',
    stage: 2,
  },
  logic: {
    family: 'logic',
    label: 'Logic',
    token: 'var(--node-logic)',
    icon: GitMerge,
    blurb: 'AND, OR, NOT, If/Then.',
    stage: 3,
  },
  ai: {
    family: 'ai',
    label: 'Intelligence',
    token: 'var(--node-ai)',
    icon: Sparkles,
    blurb: 'AI forecast, regime detector, sentiment.',
    stage: 3,
  },
  intent: {
    family: 'intent',
    label: 'Trade action',
    token: 'var(--node-intent)',
    icon: Waves,
    blurb: 'Buy/sell, position size, scale-in.',
    stage: 4,
  },
  risk: {
    family: 'risk',
    label: 'Exit rule',
    token: 'var(--node-risk)',
    icon: Shield,
    blurb: 'Stop loss, take profit, trailing, daily loss cap.',
    stage: 5,
  },
}

/** Which family a React Flow node type belongs to. */
export const TYPE_FAMILY: Record<string, NodeFamily> = {
  market_data: 'data',
  indicator: 'indicator',
  condition: 'signal',
  logic: 'logic',
  ai_inference: 'ai',
  action: 'intent',
  size: 'intent',
  exit: 'risk',
}

export function familyOf(type: string): FamilyInfo {
  return FAMILIES[TYPE_FAMILY[type] ?? 'logic']
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
