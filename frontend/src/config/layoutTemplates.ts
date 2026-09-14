import type { LucideIcon } from 'lucide-react'
import {
  BarChart3,
  CandlestickChart,
  LayoutList,
  Layers,
  ListOrdered,
  Radar,
  Receipt,
  Star,
  Wallet,
} from 'lucide-react'

/* =============================================================================
   The trading dashboard's panel vocabulary.

   The dashboard is the desk: many markets, many views, arranged how the user
   wants. The per-asset terminal (/terminal/:symbol) is the other half of the
   product — one market, full depth. Panels here are deliberately small and
   composable; anything that needs the whole screen belongs in the terminal.
   ============================================================================= */

export type PanelKind =
  | 'chart'
  | 'ticket'
  | 'scanner'
  | 'watchlist'
  | 'book'
  | 'positions'
  | 'orders'
  | 'tape'
  | 'automations'

export interface PanelKindInfo {
  kind: PanelKind
  label: string
  description: string
  icon: LucideIcon
  /** Needs an instrument chosen when it is added. */
  needsInstrument: boolean
  defaultWidth: number
}

export const PANEL_KINDS: PanelKindInfo[] = [
  {
    kind: 'chart',
    label: 'Chart',
    description: 'Candles, volume and indicators for one market.',
    icon: CandlestickChart,
    needsInstrument: true,
    defaultWidth: 580,
  },
  {
    kind: 'ticket',
    label: 'Order ticket',
    description: 'Place market, limit, stop and bracket orders.',
    icon: Receipt,
    needsInstrument: true,
    defaultWidth: 360,
  },
  {
    kind: 'book',
    label: 'Order book',
    description: 'Live L2 depth with click-to-price.',
    icon: Layers,
    needsInstrument: true,
    defaultWidth: 300,
  },
  {
    kind: 'tape',
    label: 'Time & sales',
    description: 'Every print as it happens.',
    icon: ListOrdered,
    needsInstrument: true,
    defaultWidth: 280,
  },
  {
    kind: 'watchlist',
    label: 'Watchlist',
    description: 'Your markets, grouped by asset class, live.',
    icon: Star,
    needsInstrument: false,
    defaultWidth: 300,
  },
  {
    kind: 'scanner',
    label: 'Scanner',
    description: 'Run a discovery strategy across a universe and watch what triggers.',
    icon: Radar,
    needsInstrument: false,
    defaultWidth: 360,
  },
  {
    kind: 'positions',
    label: 'Positions',
    description: 'Every open position across every asset class.',
    icon: Wallet,
    needsInstrument: false,
    defaultWidth: 620,
  },
  {
    kind: 'orders',
    label: 'Working orders',
    description: 'Orders resting at a venue, with their fills.',
    icon: LayoutList,
    needsInstrument: false,
    defaultWidth: 560,
  },
  {
    kind: 'automations',
    label: 'Automations',
    description: 'What is running, and what it has done today.',
    icon: BarChart3,
    needsInstrument: false,
    defaultWidth: 380,
  },
]

export function panelKindInfo(kind: PanelKind): PanelKindInfo {
  return PANEL_KINDS.find((p) => p.kind === kind) ?? PANEL_KINDS[0]
}

export interface PanelSpec {
  id: string
  kind: PanelKind
  instrument?: string
  instruments?: string[]
  strategyId?: string
  timeframe?: string
  venue?: string
  assetClass?: string
  /** Persisted chrome state, so a refresh restores the desk exactly. */
  width?: number
  collapsed?: boolean
}

export interface LayoutTemplate {
  id: string
  name: string
  description: string
  panels: PanelSpec[]
}

export type LayoutTemplateRegistry = Record<string, LayoutTemplate>

const crypto = (id: string, kind: PanelKind, instrument?: string, width?: number): PanelSpec => ({
  id,
  kind,
  instrument,
  venue: 'kraken',
  assetClass: 'crypto_spot_cex',
  width: width ?? panelKindInfo(kind).defaultWidth,
})

export const layoutTemplates: LayoutTemplateRegistry = {
  default: {
    id: 'default',
    name: 'Default desk',
    description: 'A watchlist, one chart and a ticket. The smallest useful desk.',
    panels: [
      { id: 'watch-1', kind: 'watchlist', width: 300 },
      crypto('chart-1', 'chart', 'BTC-USD'),
      crypto('ticket-1', 'ticket', 'BTC-USD'),
    ],
  },
  execution: {
    id: 'execution',
    name: 'Execution',
    description: 'Depth, tape and a ticket beside the chart — for working an order.',
    panels: [
      crypto('chart-x', 'chart', 'BTC-USD', 620),
      crypto('book-x', 'book', 'BTC-USD'),
      crypto('tape-x', 'tape', 'BTC-USD'),
      crypto('ticket-x', 'ticket', 'BTC-USD'),
    ],
  },
  monitoring: {
    id: 'monitoring',
    name: 'Monitoring',
    description: 'Positions, working orders and running automations, side by side.',
    panels: [
      { id: 'pos-m', kind: 'positions', width: 620 },
      { id: 'ord-m', kind: 'orders', width: 560 },
      { id: 'auto-m', kind: 'automations', width: 380 },
    ],
  },
  discovery: {
    id: 'discovery',
    name: 'Discovery',
    description: 'A scanner beside two charts — for finding something to trade.',
    panels: [
      { id: 'scan-d', kind: 'scanner', width: 360 },
      crypto('chart-d1', 'chart', 'BTC-USD'),
      crypto('chart-d2', 'chart', 'ETH-USD'),
    ],
  },
}
