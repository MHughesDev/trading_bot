import type { BlockDef } from './BlockPalette'

export const BLOCKS: BlockDef[] = [
  // ── data ────────────────────────────────────────────────────────────────
  { label: 'Price series', family: 'data', type: 'market_data', data: { source: 'price', symbol: '', timeframe: '1h', field: 'close' }, keywords: 'ohlc candles bars close' },
  { label: 'Volume', family: 'data', type: 'market_data', data: { source: 'volume', symbol: '', timeframe: '1h', field: 'close' } },
  { label: 'Order book', family: 'data', type: 'market_data', data: { source: 'orderbook', symbol: '', timeframe: '1m', field: 'close' }, keywords: 'depth l2 bids asks' },
  { label: 'Funding rate', family: 'data', type: 'market_data', data: { source: 'funding', symbol: '', timeframe: '1h', field: 'close' }, keywords: 'perp perpetual carry' },

  // ── indicators ──────────────────────────────────────────────────────────
  { label: 'EMA', family: 'indicator', type: 'indicator', data: { indicatorId: 'ema_1', kind: 'ema', period: 14 }, keywords: 'exponential moving average' },
  { label: 'SMA', family: 'indicator', type: 'indicator', data: { indicatorId: 'sma_1', kind: 'sma', period: 14 }, keywords: 'simple moving average' },
  { label: 'RSI', family: 'indicator', type: 'indicator', data: { indicatorId: 'rsi_1', kind: 'rsi', period: 14 }, keywords: 'relative strength momentum' },
  { label: 'ATR', family: 'indicator', type: 'indicator', data: { indicatorId: 'atr_1', kind: 'atr', period: 14 }, keywords: 'average true range volatility' },

  // ── signals ─────────────────────────────────────────────────────────────
  { label: 'Crosses above', family: 'signal', type: 'condition', data: { conditionType: 'cross_above', rightMode: 'indicator', rightValue: 0 }, keywords: 'crossover golden' },
  { label: 'Crosses below', family: 'signal', type: 'condition', data: { conditionType: 'cross_below', rightMode: 'indicator', rightValue: 0 }, keywords: 'crossunder death' },
  { label: 'Greater than', family: 'signal', type: 'condition', data: { conditionType: 'greater_than', rightMode: 'value', rightValue: 50 }, keywords: 'above >' },
  { label: 'Less than', family: 'signal', type: 'condition', data: { conditionType: 'less_than', rightMode: 'value', rightValue: 50 }, keywords: 'below <' },
  { label: 'Is rising', family: 'signal', type: 'condition', data: { conditionType: 'rising', rightMode: 'value', rightValue: 0 }, keywords: 'slope up trend' },
  { label: 'Is falling', family: 'signal', type: 'condition', data: { conditionType: 'falling', rightMode: 'value', rightValue: 0 }, keywords: 'slope down trend' },

  // ── logic ───────────────────────────────────────────────────────────────
  { label: 'AND gate', family: 'logic', type: 'logic', data: { op: 'and', inputCount: 2 }, keywords: 'all both' },
  { label: 'OR gate', family: 'logic', type: 'logic', data: { op: 'or', inputCount: 2 }, keywords: 'any either' },

  // ── intelligence ────────────────────────────────────────────────────────
  {
    label: 'AI forecast',
    family: 'ai',
    type: 'ai_inference',
    data: {
      targetKind: 'model',
      targetRef: '',
      alias: 'production',
      direction: 'bullish',
      minConfidence: 0.6,
      timeframe: '1h',
      lookback: 200,
    },
    keywords: 'model prediction inference ml',
  },

  // ── trade action ────────────────────────────────────────────────────────
  { label: 'Buy / Sell', family: 'intent', type: 'action', data: { side: 'buy' }, keywords: 'long short order entry' },
  { label: 'Position size', family: 'intent', type: 'size', data: { sizeType: 'percent_of_equity', value: 0.02 }, keywords: 'risk sizing quantity' },

  // ── exit rules ──────────────────────────────────────────────────────────
  { label: 'Stop loss', family: 'risk', type: 'exit', data: { exitType: 'stop_loss', value: 0.015 }, keywords: 'sl protective' },
  { label: 'Take profit', family: 'risk', type: 'exit', data: { exitType: 'take_profit', value: 0.04 }, keywords: 'tp target' },
  { label: 'Trailing stop', family: 'risk', type: 'exit', data: { exitType: 'trailing_stop', value: 0.02 }, keywords: 'trail ratchet' },
]
