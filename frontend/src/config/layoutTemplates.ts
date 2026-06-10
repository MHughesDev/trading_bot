export type PanelKind = 'scanner' | 'terminal'

export interface PanelSpec {
  id: string
  kind: PanelKind
  instrument?: string
  strategyId?: string
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
        id: 'scanner-1',
        kind: 'scanner',
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
