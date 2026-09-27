import { Activity, Database, GitMerge, Shield, Sparkles, TrendingUp, Waves } from 'lucide-react'
import type { FamilyInfo, NodeFamily } from './NodeShell'

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
