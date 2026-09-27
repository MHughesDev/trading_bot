import type { TriggerSpec } from './TriggerStep'

export const DEFAULT_TRIGGER: TriggerSpec = {
  kind: 'ohlcv_bar',
  timeframe: 'minutes1',
}
