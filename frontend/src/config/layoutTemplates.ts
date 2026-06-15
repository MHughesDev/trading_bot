export type PanelKind = 'scanner' | 'chart' | 'terminal'

export interface PanelSpec {
  id: string
  kind: PanelKind
  instrument?: string
  instruments?: string[]
  strategyId?: string
  timeframe?: string
  venue?: string
  assetClass?: string
}

export interface LayoutTemplate {
  id: string
  name: string
  panels: PanelSpec[]
}

export type LayoutTemplateRegistry = Record<string, LayoutTemplate>

export const layoutTemplates: LayoutTemplateRegistry = {
  default: {
    id: 'default',
    name: 'Default',
    panels: [
      {
        id: 'chart-1',
        kind: 'chart',
        instrument: 'BTC-USD',
        venue: 'kraken',
        assetClass: 'crypto_spot_cex',
      },
      {
        id: 'terminal-1',
        kind: 'terminal',
        instrument: 'BTC-USD',
        venue: 'kraken',
        assetClass: 'crypto_spot_cex',
      },
    ],
  },
  crypto_focus: {
    id: 'crypto_focus',
    name: 'Crypto Focus',
    panels: [
      {
        id: 'scanner-crypto',
        kind: 'scanner',
      },
      {
        id: 'terminal-btc',
        kind: 'terminal',
        instrument: 'BTC-USD',
        venue: 'kraken',
        assetClass: 'crypto_spot_cex',
      },
      {
        id: 'terminal-eth',
        kind: 'terminal',
        instrument: 'ETH-USD',
        venue: 'kraken',
        assetClass: 'crypto_spot_cex',
      },
    ],
  },
}
