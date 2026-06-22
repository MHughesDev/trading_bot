// Modular trigger configuration step.
// Renders different controls based on trigger type; used by both
// SingleInstrumentFlow and PipelineFlow.

import { ChevronDown } from 'lucide-react'
import { cn } from '@/lib/utils'

export type TriggerKind = 'ohlcv_bar' | 'timer'

export interface OhlcvBarTrigger {
  kind: 'ohlcv_bar'
  timeframe: string
}

export interface TimerTrigger {
  kind: 'timer'
  interval_secs: number
}

export type TriggerSpec = OhlcvBarTrigger | TimerTrigger

export const DEFAULT_TRIGGER: TriggerSpec = {
  kind: 'ohlcv_bar',
  timeframe: 'minutes1',
}

const TIMEFRAMES = [
  { value: 'minutes1', label: '1 minute' },
  { value: 'minutes5', label: '5 minutes' },
  { value: 'minutes15', label: '15 minutes' },
  { value: 'hours1', label: '1 hour' },
  { value: 'hours4', label: '4 hours' },
  { value: 'daily', label: 'Daily' },
]

const INTERVALS = [
  { value: 30, label: '30 seconds' },
  { value: 60, label: '1 minute' },
  { value: 300, label: '5 minutes' },
  { value: 900, label: '15 minutes' },
  { value: 1800, label: '30 minutes' },
  { value: 3600, label: '1 hour' },
]

interface TriggerStepProps {
  value: TriggerSpec
  onChange: (spec: TriggerSpec) => void
}

const selectClass = cn(
  'w-full appearance-none rounded-lg px-3 py-2 pr-8 text-sm',
  'bg-surface-2 border border-border text-text',
  'focus:outline-none focus:ring-1 focus:ring-accent',
)

export function TriggerStep({ value, onChange }: TriggerStepProps) {
  return (
    <div className="space-y-3">
      <div className="relative">
        <select
          value={value.kind}
          onChange={(e) => {
            const k = e.target.value as TriggerKind
            onChange(
              k === 'ohlcv_bar'
                ? { kind: 'ohlcv_bar', timeframe: 'minutes_1' }
                : { kind: 'timer', interval_secs: 300 },
            )
          }}
          className={selectClass}
        >
          <option value="ohlcv_bar">On bar close (OHLCV)</option>
          <option value="timer">On timer interval</option>
        </select>
        <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
      </div>

      {value.kind === 'ohlcv_bar' && (
        <div className="space-y-1.5">
          <div className="relative">
            <select
              value={value.timeframe}
              onChange={(e) => onChange({ kind: 'ohlcv_bar', timeframe: e.target.value })}
              className={selectClass}
            >
              {TIMEFRAMES.map((tf) => (
                <option key={tf.value} value={tf.value}>
                  {tf.label}
                </option>
              ))}
            </select>
            <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
          </div>
          {value.timeframe !== 'minutes1' && (
            <p className="text-xs text-text-dim">
              HTF bars are built from 1m data — same window boundaries as the chart.
            </p>
          )}
        </div>
      )}

      {value.kind === 'timer' && (
        <div className="relative">
          <select
            value={value.interval_secs}
            onChange={(e) =>
              onChange({ kind: 'timer', interval_secs: Number(e.target.value) })
            }
            className={selectClass}
          >
            {INTERVALS.map((iv) => (
              <option key={iv.value} value={iv.value}>
                {iv.label}
              </option>
            ))}
          </select>
          <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-text-dim" />
        </div>
      )}
    </div>
  )
}
