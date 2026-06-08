export type IndicatorKind = 'ema' | 'sma' | 'rsi' | 'atr'
export type ConditionType = 'cross_above' | 'cross_below' | 'greater_than' | 'less_than' | 'rising' | 'falling'
export type ExitType = 'stop_loss' | 'take_profit' | 'trailing_stop'
export type Side = 'buy' | 'sell'
export type SizeType = 'percent_of_equity' | 'fixed_quantity'
export type ForecastDirection = 'bullish' | 'bearish' | 'any'

export const INDICATOR_LABELS: Record<IndicatorKind, string> = {
  ema: 'EMA — Exponential Moving Average',
  sma: 'SMA — Simple Moving Average',
  rsi: 'RSI — Relative Strength Index',
  atr: 'ATR — Average True Range',
}

export const CONDITION_LABELS: Record<ConditionType, string> = {
  cross_above: 'Crosses Above',
  cross_below: 'Crosses Below',
  greater_than: 'Greater Than',
  less_than: 'Less Than',
  rising: 'Is Rising',
  falling: 'Is Falling',
}

export const EXIT_LABELS: Record<ExitType, string> = {
  stop_loss: 'Stop Loss',
  take_profit: 'Take Profit',
  trailing_stop: 'Trailing Stop',
}

export interface IndicatorSpec {
  id: string
  kind: IndicatorKind
  period: number
}

export interface Condition {
  type: ConditionType | 'model_forecast'
  left: string
  right_id?: string
  right_value?: number
}

export interface EntryRule {
  side: Side
  all: Condition[]
  any: Condition[]
}

export interface SizeRule {
  type: SizeType
  value: number
}

export interface ExitRule {
  type: ExitType
  value: number
}

export interface RuleStrategySpec {
  name: string
  indicators: IndicatorSpec[]
  entry: EntryRule
  size: SizeRule
  exits: ExitRule[]
}

export interface PreviewResponse {
  valid: boolean
  explanation?: string
  errors?: string[]
}

export interface SavedStrategy {
  id: string
  registry_key: string
  name: string
  spec: RuleStrategySpec
  explanation?: string
  created_at: string
  updated_at: string
}
