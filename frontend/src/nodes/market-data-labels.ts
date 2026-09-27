import type { MarketDataSource } from './index'

export const MARKET_DATA_LABELS: Record<MarketDataSource, string> = {
  price: 'Price series',
  volume: 'Volume',
  orderbook: 'Order book',
  funding: 'Funding rate',
}

export const MARKET_DATA_LANES: Record<MarketDataSource, string> = {
  price: 'market.bars.1m',
  volume: 'market.bars.1m',
  orderbook: 'market.orderbook.l2',
  funding: 'market.funding_rate',
}
